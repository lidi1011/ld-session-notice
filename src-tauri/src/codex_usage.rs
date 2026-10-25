use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::{CodexUsageStatus, CodexUsageWindow};

const CACHE_TTL: Duration = Duration::from_secs(5);
const GENERAL_LIMIT_STALE_AFTER: Duration = Duration::from_secs(120);

static CACHE: OnceLock<Mutex<Option<CachedUsage>>> = OnceLock::new();
static TRANSCRIPT_INDEX: OnceLock<Mutex<HashMap<PathBuf, IndexedTranscript>>> = OnceLock::new();
static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct CachedUsage {
    loaded_at: Instant,
    value: Option<CodexUsageStatus>,
}

#[derive(Clone)]
struct IndexedTranscript {
    len: u64,
    modified: Option<SystemTime>,
    offset: u64,
    candidates: UsageCandidates,
}

struct ScanGuard;

impl Drop for ScanGuard {
    fn drop(&mut self) {
        SCAN_RUNNING.store(false, Ordering::Release);
    }
}

pub async fn latest() -> Option<CodexUsageStatus> {
    if let Some(remote) = remote_usage().await {
        return Some(remote);
    }
    if let Some(value) = cached_value() {
        return value;
    }
    // A scan can outlive the poll interval. Reuse the last value instead of stacking scans.
    if SCAN_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return last_cached_value();
    }
    let _scan_guard = ScanGuard;

    let value = tokio::task::spawn_blocking(scan_latest_usage)
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();

    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedUsage {
            loaded_at: Instant::now(),
            value: value.clone(),
        });
    }

    value
}

/// Usage straight from ChatGPT, signed in as Codex is (~/.codex/auth.json).
/// Codex only writes rate limits into its transcripts when it talks to OpenAI
/// directly; behind a gateway such as magpie they come back empty, so asking
/// OpenAI is the one source that keeps working. This is the endpoint the Codex
/// client itself polls; it is not a public API, so on any failure the caller
/// falls back to the transcripts.
const REMOTE_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const REMOTE_REFRESH: Duration = Duration::from_secs(60);

static REMOTE: OnceLock<Mutex<Option<CachedUsage>>> = OnceLock::new();
static REMOTE_RUNNING: AtomicBool = AtomicBool::new(false);

struct RemoteGuard;

impl Drop for RemoteGuard {
    fn drop(&mut self) {
        REMOTE_RUNNING.store(false, Ordering::Release);
    }
}

async fn remote_usage() -> Option<CodexUsageStatus> {
    let cache = REMOTE.get_or_init(|| Mutex::new(None));
    let cached = cache.lock().ok().and_then(|guard| guard.clone());
    if let Some(cached) = &cached {
        if cached.loaded_at.elapsed() <= REMOTE_REFRESH {
            return cached.value.clone();
        }
    }
    if REMOTE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return cached.and_then(|cached| cached.value);
    }
    let _guard = RemoteGuard;

    // Failures are cached too, so a signed-out Codex is not asked every poll.
    let value = fetch_remote_usage().await.ok();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedUsage {
            loaded_at: Instant::now(),
            value: value.clone(),
        });
    }
    value
}

async fn fetch_remote_usage() -> anyhow::Result<CodexUsageStatus> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let auth = fs::read_to_string(home.join(".codex").join("auth.json"))?;
    let tokens = serde_json::from_str::<CodexAuthFile>(&auth)?
        .tokens
        .ok_or_else(|| anyhow::anyhow!("Codex is not signed in with ChatGPT"))?;
    let mut request = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?
        .get(REMOTE_USAGE_URL)
        .bearer_auth(tokens.access_token)
        .header("User-Agent", "codex-cli")
        .header("Accept", "application/json");
    if let Some(account_id) = tokens.account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let body = request.send().await?.error_for_status()?.text().await?;
    parse_remote_usage(&body, Utc::now()).ok_or_else(|| anyhow::anyhow!("unexpected usage response"))
}

fn parse_remote_usage(body: &str, updated_at: DateTime<Utc>) -> Option<CodexUsageStatus> {
    let usage: RemoteUsage = serde_json::from_str(body).ok()?;
    let limits = usage.rate_limit?;
    let primary = limits.primary_window.map(RemoteUsageWindow::into_window);
    let secondary = limits.secondary_window.map(RemoteUsageWindow::into_window);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(CodexUsageStatus {
        limit_id: "codex".to_string(),
        limit_name: None,
        primary,
        secondary,
        plan_type: usage.plan_type,
        rate_limit_reached_type: usage.rate_limit_reached_type,
        updated_at,
    })
}

#[derive(Deserialize)]
struct CodexAuthFile {
    tokens: Option<CodexAuthTokens>,
}

#[derive(Deserialize)]
struct CodexAuthTokens {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct RemoteUsage {
    plan_type: Option<String>,
    rate_limit_reached_type: Option<String>,
    rate_limit: Option<RemoteRateLimit>,
}

#[derive(Deserialize)]
struct RemoteRateLimit {
    primary_window: Option<RemoteUsageWindow>,
    secondary_window: Option<RemoteUsageWindow>,
}

#[derive(Deserialize)]
struct RemoteUsageWindow {
    used_percent: f64,
    limit_window_seconds: i64,
    reset_at: Option<i64>,
}

impl RemoteUsageWindow {
    fn into_window(self) -> CodexUsageWindow {
        let used_percent = self.used_percent.clamp(0.0, 100.0);
        CodexUsageWindow {
            used_percent,
            remaining_percent: 100.0 - used_percent,
            window_minutes: self.limit_window_seconds / 60,
            resets_at: self.reset_at.and_then(timestamp_from_seconds),
        }
    }
}

pub fn invalidate_cache() {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        if let Some(cached) = guard.as_mut() {
            cached.loaded_at = Instant::now() - CACHE_TTL - Duration::from_millis(1);
        }
    }
}

fn cached_value() -> Option<Option<CodexUsageStatus>> {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let guard = cache.lock().ok()?;
    let cached = guard.as_ref()?;
    if cached.loaded_at.elapsed() <= CACHE_TTL {
        return Some(cached.value.clone());
    }
    None
}

fn last_cached_value() -> Option<CodexUsageStatus> {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    cache.lock().ok()?.as_ref()?.value.clone()
}

fn scan_latest_usage() -> anyhow::Result<Option<CodexUsageStatus>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };

    scan_latest_usage_for_home(&home)
}

fn scan_latest_usage_for_home(home: &Path) -> anyhow::Result<Option<CodexUsageStatus>> {
    let codexmate_status = read_codexmate_usage(home);
    let transcript_status = scan_transcript_usage(home)?;

    Ok(select_usage_source(codexmate_status, transcript_status))
}

fn scan_transcript_usage(home: &Path) -> anyhow::Result<Option<CodexUsageStatus>> {
    let index = TRANSCRIPT_INDEX.get_or_init(|| Mutex::new(HashMap::new()));
    let mut index = index
        .lock()
        .map_err(|_| anyhow::anyhow!("Codex usage index is unavailable"))?;
    scan_transcript_usage_with_index(home, &mut index)
}

fn scan_transcript_usage_with_index(
    home: &Path,
    index: &mut HashMap<PathBuf, IndexedTranscript>,
) -> anyhow::Result<Option<CodexUsageStatus>> {
    let mut seen = HashSet::new();
    let mut candidates = UsageCandidates::default();
    for root in [
        home.join(".codex").join("sessions"),
        home.join(".codex").join("archived_sessions"),
    ] {
        scan_dir(&root, index, &mut seen)?;
    }
    index.retain(|path, _| seen.contains(path));
    for indexed in index.values() {
        candidates.merge(indexed.candidates.clone());
    }

    Ok(candidates.into_latest())
}

fn select_usage_source(
    codexmate_status: Option<CodexUsageStatus>,
    transcript_status: Option<CodexUsageStatus>,
) -> Option<CodexUsageStatus> {
    match (codexmate_status, transcript_status) {
        (Some(codexmate), Some(transcript)) => {
            if should_prefer_transcript_usage(&codexmate, &transcript) {
                Some(transcript)
            } else {
                Some(codexmate)
            }
        }
        (Some(codexmate), None) => Some(codexmate),
        (None, transcript) => transcript,
    }
}

fn should_prefer_transcript_usage(
    codexmate: &CodexUsageStatus,
    transcript: &CodexUsageStatus,
) -> bool {
    transcript.limit_id == "codex" && is_general_limit_stale(codexmate, transcript)
}

fn read_codexmate_usage(home: &Path) -> Option<CodexUsageStatus> {
    let codexmate_dir = home.join(".codex").join("codexmate");
    let bootstrap_content = fs::read_to_string(codexmate_dir.join("bootstrap-cache.json")).ok();
    if let Some(status) = bootstrap_content
        .as_deref()
        .and_then(parse_codexmate_bootstrap_active_usage)
    {
        return Some(status);
    }

    let active_account_key = bootstrap_content
        .as_deref()
        .and_then(parse_codexmate_active_account_key);
    let content = fs::read_to_string(codexmate_dir.join("quota-store.json")).ok()?;
    parse_codexmate_quota_store(&content, active_account_key.as_deref())
}

fn parse_codexmate_quota_store(
    content: &str,
    active_account_key: Option<&str>,
) -> Option<CodexUsageStatus> {
    let store: CodexMateQuotaStore = serde_json::from_str(content).ok()?;
    if let Some(account_key) = active_account_key {
        if let Some(status) = store
            .items
            .iter()
            .filter(|item| item.account_key.as_deref() == Some(account_key))
            .cloned()
            .filter_map(CodexMateAccountUsage::into_status)
            .max_by_key(|status| status.updated_at)
        {
            return Some(status);
        }
    }

    store
        .items
        .into_iter()
        .filter_map(CodexMateAccountUsage::into_status)
        .max_by_key(|status| status.updated_at)
}

fn parse_codexmate_bootstrap_active_usage(content: &str) -> Option<CodexUsageStatus> {
    let cache: CodexMateBootstrapCache = serde_json::from_str(content).ok()?;
    cache
        .snapshot_progressive?
        .status?
        .active_account?
        .into_status()
}

fn parse_codexmate_active_account_key(content: &str) -> Option<String> {
    let cache: CodexMateBootstrapCache = serde_json::from_str(content).ok()?;
    cache.snapshot_progressive?.status?.active_account_key
}

#[derive(Clone, Default)]
struct UsageCandidates {
    latest_codex: Option<CodexUsageStatus>,
    latest_any: Option<CodexUsageStatus>,
}

impl UsageCandidates {
    fn record(&mut self, status: CodexUsageStatus) {
        if is_newer(&status, &self.latest_any) {
            self.latest_any = Some(status.clone());
        }
        if status.limit_id == "codex" && is_newer(&status, &self.latest_codex) {
            self.latest_codex = Some(status);
        }
    }

    fn merge(&mut self, other: UsageCandidates) {
        if let Some(status) = other.latest_any {
            self.record(status);
        }
        if let Some(status) = other.latest_codex {
            self.record(status);
        }
    }

    fn into_latest(self) -> Option<CodexUsageStatus> {
        match (self.latest_codex, self.latest_any) {
            (Some(codex), Some(any)) => {
                if is_general_limit_stale(&codex, &any) {
                    Some(any)
                } else {
                    Some(codex)
                }
            }
            (Some(codex), None) => Some(codex),
            (None, any) => any,
        }
    }
}

fn is_general_limit_stale(codex: &CodexUsageStatus, latest: &CodexUsageStatus) -> bool {
    latest
        .updated_at
        .signed_duration_since(codex.updated_at)
        .to_std()
        .map(|age| age > GENERAL_LIMIT_STALE_AFTER)
        .unwrap_or(false)
}

fn is_newer(status: &CodexUsageStatus, current: &Option<CodexUsageStatus>) -> bool {
    current
        .as_ref()
        .map(|current| status.updated_at > current.updated_at)
        .unwrap_or(true)
}

fn scan_dir(
    path: &Path,
    index: &mut HashMap<PathBuf, IndexedTranscript>,
    seen: &mut HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(path) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, index, seen)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            seen.insert(path.clone());
            scan_file(&path, index)?;
        }
    }

    Ok(())
}

fn scan_file(path: &Path, index: &mut HashMap<PathBuf, IndexedTranscript>) -> anyhow::Result<()> {
    let metadata = fs::metadata(path)?;
    let len = metadata.len();
    let modified = metadata.modified().ok();
    let mut indexed = match index.get(path) {
        Some(existing) if existing.len == len && existing.modified == modified => return Ok(()),
        // Codex transcripts are append-only. Any other change requires a clean reparse.
        Some(existing) if len >= existing.offset => existing.clone(),
        _ => IndexedTranscript {
            len: 0,
            modified: None,
            offset: 0,
            candidates: UsageCandidates::default(),
        },
    };

    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(indexed.offset))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if !line.ends_with('\n') {
            // Keep the offset before this record so the next poll rereads it after its newline arrives.
            if line.contains("\"rate_limits\"") {
                if let Some(status) = parse_usage_line(&line) {
                    indexed.candidates.record(status);
                }
            }
            break;
        }
        indexed.offset += read as u64;
        if !line.contains("\"rate_limits\"") {
            continue;
        }
        if let Some(status) = parse_usage_line(line.trim_end()) {
            indexed.candidates.record(status);
        }
    }
    indexed.len = len;
    indexed.modified = modified;
    index.insert(path.to_path_buf(), indexed);
    Ok(())
}

fn parse_usage_line(line: &str) -> Option<CodexUsageStatus> {
    let event: CodexLogLine = serde_json::from_str(line).ok()?;
    let payload = event.payload?;
    let raw = payload
        .rate_limits
        .or_else(|| payload.info.and_then(|info| info.rate_limits))?;
    raw.into_status(event.timestamp?)
}

#[derive(Debug, Deserialize)]
struct CodexLogLine {
    timestamp: Option<DateTime<Utc>>,
    payload: Option<CodexLogPayload>,
}

#[derive(Debug, Deserialize)]
struct CodexLogPayload {
    rate_limits: Option<RawUsageStatus>,
    info: Option<CodexLogInfo>,
}

#[derive(Debug, Deserialize)]
struct CodexLogInfo {
    rate_limits: Option<RawUsageStatus>,
}

#[derive(Debug, Deserialize)]
struct RawUsageStatus {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<RawUsageWindow>,
    secondary: Option<RawUsageWindow>,
    plan_type: Option<String>,
    rate_limit_reached_type: Option<String>,
}

impl RawUsageStatus {
    fn into_status(self, updated_at: DateTime<Utc>) -> Option<CodexUsageStatus> {
        Some(CodexUsageStatus {
            limit_id: self.limit_id?,
            limit_name: self.limit_name,
            primary: self.primary.and_then(RawUsageWindow::into_window),
            secondary: self.secondary.and_then(RawUsageWindow::into_window),
            plan_type: self.plan_type,
            rate_limit_reached_type: self.rate_limit_reached_type,
            updated_at,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawUsageWindow {
    used_percent: f64,
    window_minutes: i64,
    resets_at: Option<i64>,
}

impl RawUsageWindow {
    fn into_window(self) -> Option<CodexUsageWindow> {
        let remaining_percent = (100.0 - self.used_percent).clamp(0.0, 100.0);
        Some(CodexUsageWindow {
            used_percent: self.used_percent,
            remaining_percent,
            window_minutes: self.window_minutes,
            resets_at: self.resets_at.and_then(timestamp_from_seconds),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateQuotaStore {
    #[serde(default)]
    items: Vec<CodexMateAccountUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateBootstrapCache {
    snapshot_progressive: Option<CodexMateBootstrapSnapshot>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateBootstrapSnapshot {
    status: Option<CodexMateBootstrapStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateBootstrapStatus {
    active_account_key: Option<String>,
    active_account: Option<CodexMateAccountUsage>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateAccountUsage {
    account_key: Option<String>,
    captured_at: Option<i64>,
    last_usage_at: Option<i64>,
    last_used_at: Option<i64>,
    primary_window: Option<CodexMateUsageWindow>,
    secondary_window: Option<CodexMateUsageWindow>,
}

impl CodexMateAccountUsage {
    fn into_status(self) -> Option<CodexUsageStatus> {
        let updated_at = self
            .captured_at
            .or(self.last_usage_at)
            .or(self.last_used_at)
            .and_then(timestamp_from_seconds)?;
        let primary = self
            .primary_window
            .and_then(CodexMateUsageWindow::into_window);
        let secondary = self
            .secondary_window
            .and_then(CodexMateUsageWindow::into_window);

        if primary.is_none() && secondary.is_none() {
            return None;
        }

        Some(CodexUsageStatus {
            limit_id: "codex".to_string(),
            limit_name: None,
            primary,
            secondary,
            plan_type: None,
            rate_limit_reached_type: None,
            updated_at,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexMateUsageWindow {
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

impl CodexMateUsageWindow {
    fn into_window(self) -> Option<CodexUsageWindow> {
        let remaining_percent = match (self.remaining_percent, self.used_percent) {
            (Some(remaining), _) => remaining.clamp(0.0, 100.0),
            (None, Some(used)) => (100.0 - used).clamp(0.0, 100.0),
            (None, None) => return None,
        };
        let used_percent = match self.used_percent {
            Some(used) => used.clamp(0.0, 100.0),
            None => (100.0 - remaining_percent).clamp(0.0, 100.0),
        };

        Some(CodexUsageWindow {
            used_percent,
            remaining_percent,
            window_minutes: self.window_minutes?,
            resets_at: self.resets_at.and_then(timestamp_from_seconds),
        })
    }
}

fn timestamp_from_seconds(seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(seconds, 0).single()
}

#[cfg(test)]
mod tests {
    use super::{
        cached_value, invalidate_cache, last_cached_value, parse_codexmate_bootstrap_active_usage,
        parse_codexmate_quota_store, parse_remote_usage, parse_usage_line,
        scan_latest_usage_for_home, scan_transcript_usage_with_index, CachedUsage,
        UsageCandidates, CACHE,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::time::Instant;

    #[test]
    fn parses_chatgpt_usage_response() {
        let body = r#"{"plan_type":"plus","email":"x@example.com","rate_limit":{"allowed":true,"limit_reached":false,"primary_window":{"used_percent":27,"limit_window_seconds":18000,"reset_after_seconds":13140,"reset_at":1782770922},"secondary_window":{"used_percent":4,"limit_window_seconds":604800,"reset_after_seconds":566340,"reset_at":1783357722}},"rate_limit_reached_type":null,"credits":null}"#;
        let status = parse_remote_usage(body, chrono::Utc::now()).unwrap();
        let primary = status.primary.unwrap();
        let secondary = status.secondary.unwrap();
        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.plan_type.as_deref(), Some("plus"));
        assert_eq!((primary.used_percent, primary.remaining_percent, primary.window_minutes), (27.0, 73.0, 300));
        assert_eq!(secondary.window_minutes, 10080);
        assert_eq!(primary.resets_at.unwrap().timestamp(), 1782770922);
    }

    #[test]
    fn empty_chatgpt_usage_falls_back() {
        assert!(parse_remote_usage(r#"{"plan_type":"plus","rate_limit":null}"#, chrono::Utc::now()).is_none());
        assert!(parse_remote_usage("not json", chrono::Utc::now()).is_none());
    }

    #[test]
    fn parses_rate_limit_payload() {
        let status = parse_usage_line(
            r#"{"timestamp":"2026-06-16T11:04:51.542Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":12.0,"window_minutes":300,"resets_at":1781625881},"secondary":{"used_percent":56.0,"window_minutes":10080,"resets_at":1782212681},"plan_type":"plus","rate_limit_reached_type":null}}}"#,
        )
        .unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 88.0);
        assert_eq!(status.secondary.unwrap().window_minutes, 10080);
        assert_eq!(status.plan_type.as_deref(), Some("plus"));
    }

    #[test]
    fn prefers_general_codex_limit_over_newer_model_limit() {
        let codex = parse_usage_line(
            r#"{"timestamp":"2026-06-16T14:35:03.253Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":300,"resets_at":1781637833},"secondary":{"used_percent":59.0,"window_minutes":10080,"resets_at":1781667984},"credits":{"has_credits":false,"unlimited":false,"balance":null},"individual_limit":null,"plan_type":null,"rate_limit_reached_type":null}}}"#,
        )
        .unwrap();
        let model = parse_usage_line(
            r#"{"timestamp":"2026-06-16T14:36:24.973Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark","primary":{"used_percent":0.0,"window_minutes":300,"resets_at":1781638867},"secondary":{"used_percent":0.0,"window_minutes":10080,"resets_at":1781667913},"credits":{"has_credits":false,"unlimited":false,"balance":null},"individual_limit":null,"plan_type":null,"rate_limit_reached_type":null}}}"#,
        )
        .unwrap();

        let mut candidates = UsageCandidates::default();
        candidates.record(model);
        candidates.record(codex);

        let status = candidates.into_latest().unwrap();
        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 96.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 41.0);
    }

    #[test]
    fn uses_latest_model_limit_when_general_limit_is_stale() {
        let codex = parse_usage_line(
            r#"{"timestamp":"2026-06-16T14:35:03.253Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":300,"resets_at":1781637833},"secondary":{"used_percent":59.0,"window_minutes":10080,"resets_at":1781667984}}}}"#,
        )
        .unwrap();
        let model = parse_usage_line(
            r#"{"timestamp":"2026-06-16T15:37:53.132Z","payload":{"type":"event_msg","info":{"rate_limits":{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark","primary":{"used_percent":0.0,"window_minutes":300,"resets_at":1781642249},"secondary":{"used_percent":0.0,"window_minutes":10080,"resets_at":1782229049}}}}}"#,
        )
        .unwrap();

        let mut candidates = UsageCandidates::default();
        candidates.record(codex);
        candidates.record(model);

        let status = candidates.into_latest().unwrap();
        assert_eq!(status.limit_id, "codex_bengalfox");
        assert_eq!(status.limit_name.as_deref(), Some("GPT-5.3-Codex-Spark"));
        assert_eq!(status.primary.unwrap().remaining_percent, 100.0);
    }

    #[test]
    fn falls_back_to_latest_model_limit_when_general_limit_is_absent() {
        let older = parse_usage_line(
            r#"{"timestamp":"2026-06-16T14:40:53.682Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex_bengalfox","primary":{"used_percent":1.0,"window_minutes":300,"resets_at":1781638836},"secondary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1781667913}}}}"#,
        )
        .unwrap();
        let newer = parse_usage_line(
            r#"{"timestamp":"2026-06-16T14:41:24.973Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex_bengalfox","primary":{"used_percent":3.0,"window_minutes":300,"resets_at":1781638867},"secondary":{"used_percent":4.0,"window_minutes":10080,"resets_at":1781667913}}}}"#,
        )
        .unwrap();

        let mut candidates = UsageCandidates::default();
        candidates.record(older);
        candidates.record(newer);

        let status = candidates.into_latest().unwrap();
        assert_eq!(status.limit_id, "codex_bengalfox");
        assert_eq!(status.primary.unwrap().remaining_percent, 97.0);
    }

    #[test]
    fn parses_codexmate_quota_store() {
        let status = parse_codexmate_quota_store(
            r#"{"items":[{"capturedAt":1781624584,"usageSource":"api","primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300,"resetsAt":1781637870},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080,"resetsAt":1781667984}},{"capturedAt":1781620000,"primaryWindow":{"usedPercent":4.0,"remainingPercent":96,"windowMinutes":300}}]}"#,
            None,
        )
        .unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn parses_codexmate_bootstrap_active_account() {
        let status = parse_codexmate_bootstrap_active_usage(
            r#"{"snapshotProgressive":{"status":{"activeAccountKey":"active","activeAccount":{"accountKey":"active","lastUsageAt":1781624584,"primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300,"resetsAt":1781637870},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080,"resetsAt":1781667984}}}}}"#,
        )
        .unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn prefers_codexmate_active_account_key_over_newer_quota_item() {
        let status = parse_codexmate_quota_store(
            r#"{"items":[{"accountKey":"inactive","capturedAt":1781625000,"primaryWindow":{"usedPercent":1.0,"remainingPercent":99,"windowMinutes":300},"secondaryWindow":{"usedPercent":2.0,"remainingPercent":98,"windowMinutes":10080}},{"accountKey":"active","capturedAt":1781624584,"primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080}}]}"#,
            Some("active"),
        )
        .unwrap();

        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn prefers_codexmate_quota_store_over_transcript_scan() {
        let home = tempfile::tempdir().unwrap();
        let codexmate_dir = home.path().join(".codex").join("codexmate");
        let sessions_dir = home.path().join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&codexmate_dir).unwrap();
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            codexmate_dir.join("quota-store.json"),
            r#"{"items":[{"capturedAt":1781628000,"usageSource":"api","primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300,"resetsAt":1781637870},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080,"resetsAt":1781667984}}]}"#,
        )
        .unwrap();
        fs::write(
            sessions_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-06-16T14:35:03.253Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":300,"resets_at":1781637833},"secondary":{"used_percent":59.0,"window_minutes":10080,"resets_at":1781667984}}}}"#,
        )
        .unwrap();

        let status = scan_latest_usage_for_home(home.path()).unwrap().unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn prefers_newer_general_transcript_over_stale_codexmate_quota() {
        let home = tempfile::tempdir().unwrap();
        let codexmate_dir = home.path().join(".codex").join("codexmate");
        let sessions_dir = home.path().join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&codexmate_dir).unwrap();
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            codexmate_dir.join("quota-store.json"),
            r#"{"items":[{"capturedAt":1781624584,"usageSource":"api","primaryWindow":{"usedPercent":5.0,"remainingPercent":95,"windowMinutes":300,"resetsAt":1781637870},"secondaryWindow":{"usedPercent":8.0,"remainingPercent":92,"windowMinutes":10080,"resetsAt":1781667984}}]}"#,
        )
        .unwrap();
        fs::write(
            sessions_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-06-19T09:12:42.211Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":14.0,"window_minutes":300,"resets_at":1781813562},"secondary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1782212681},"plan_type":"prolite","rate_limit_reached_type":null}}}"#,
        )
        .unwrap();

        let status = scan_latest_usage_for_home(home.path()).unwrap().unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 86.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 89.0);
        assert_eq!(status.plan_type.as_deref(), Some("prolite"));
    }

    #[test]
    fn keeps_codexmate_quota_when_newer_transcript_is_model_specific() {
        let home = tempfile::tempdir().unwrap();
        let codexmate_dir = home.path().join(".codex").join("codexmate");
        let sessions_dir = home.path().join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&codexmate_dir).unwrap();
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            codexmate_dir.join("quota-store.json"),
            r#"{"items":[{"capturedAt":1781624584,"usageSource":"api","primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300,"resetsAt":1781637870},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080,"resetsAt":1781667984}}]}"#,
        )
        .unwrap();
        fs::write(
            sessions_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-06-19T09:12:46.385Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark","primary":{"used_percent":0.0,"window_minutes":300,"resets_at":1781813566},"secondary":{"used_percent":0.0,"window_minutes":10080,"resets_at":1782212681},"plan_type":"prolite","rate_limit_reached_type":null}}}"#,
        )
        .unwrap();

        let status = scan_latest_usage_for_home(home.path()).unwrap().unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn uses_bootstrap_active_account_key_for_quota_store_scan() {
        let home = tempfile::tempdir().unwrap();
        let codexmate_dir = home.path().join(".codex").join("codexmate");
        fs::create_dir_all(&codexmate_dir).unwrap();
        fs::write(
            codexmate_dir.join("bootstrap-cache.json"),
            r#"{"snapshotProgressive":{"status":{"activeAccountKey":"active"}}}"#,
        )
        .unwrap();
        fs::write(
            codexmate_dir.join("quota-store.json"),
            r#"{"items":[{"accountKey":"inactive","capturedAt":1781625000,"primaryWindow":{"usedPercent":1.0,"remainingPercent":99,"windowMinutes":300},"secondaryWindow":{"usedPercent":2.0,"remainingPercent":98,"windowMinutes":10080}},{"accountKey":"active","capturedAt":1781624584,"primaryWindow":{"usedPercent":15.0,"remainingPercent":85,"windowMinutes":300},"secondaryWindow":{"usedPercent":60.0,"remainingPercent":40,"windowMinutes":10080}}]}"#,
        )
        .unwrap();

        let status = scan_latest_usage_for_home(home.path()).unwrap().unwrap();

        assert_eq!(status.primary.unwrap().remaining_percent, 85.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 40.0);
    }

    #[test]
    fn falls_back_to_transcript_scan_without_codexmate_quota_store() {
        let home = tempfile::tempdir().unwrap();
        let sessions_dir = home.path().join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-06-16T14:35:03.253Z","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":4.0,"window_minutes":300,"resets_at":1781637833},"secondary":{"used_percent":59.0,"window_minutes":10080,"resets_at":1781667984}}}}"#,
        )
        .unwrap();

        let status = scan_latest_usage_for_home(home.path()).unwrap().unwrap();

        assert_eq!(status.limit_id, "codex");
        assert_eq!(status.primary.unwrap().remaining_percent, 96.0);
        assert_eq!(status.secondary.unwrap().remaining_percent, 41.0);
    }

    #[test]
    fn invalidate_cache_expires_but_keeps_last_usage_for_overlapping_polls() {
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
        *cache.lock().unwrap() = Some(CachedUsage {
            loaded_at: Instant::now(),
            value: parse_usage_line(
                r#"{"timestamp":"2026-06-16T14:35:03.253Z","payload":{"rate_limits":{"limit_id":"codex"}}}"#,
            ),
        });

        assert!(cached_value().is_some());
        invalidate_cache();
        assert!(cached_value().is_none());
        assert_eq!(last_cached_value().unwrap().limit_id, "codex");
    }

    #[test]
    fn transcript_index_reads_only_complete_appended_lines() {
        let home = tempfile::tempdir().unwrap();
        let sessions_dir = home.path().join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("session.jsonl");
        fs::write(
            &path,
            "{\"timestamp\":\"2026-06-16T14:35:03.253Z\",\"payload\":{\"rate_limits\":{\"limit_id\":\"codex\",\"secondary\":{\"used_percent\":59.0,\"window_minutes\":10080}}}}\n",
        )
        .unwrap();
        let mut index = HashMap::new();
        let first = scan_transcript_usage_with_index(home.path(), &mut index)
            .unwrap()
            .unwrap();
        assert_eq!(first.secondary.unwrap().remaining_percent, 41.0);

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{{\"timestamp\":\"2026-06-16T14:40:03.253Z\",\"payload\":{{\"rate_limits\":{{\"limit_id\":\"codex\",\"secondary\":{{\"used_percent\":20.0,").unwrap();
        let partial = scan_transcript_usage_with_index(home.path(), &mut index)
            .unwrap()
            .unwrap();
        assert_eq!(partial.secondary.unwrap().remaining_percent, 41.0);

        writeln!(file, "\"window_minutes\":10080}}}}}}}}").unwrap();
        let appended = scan_transcript_usage_with_index(home.path(), &mut index)
            .unwrap()
            .unwrap();
        assert_eq!(appended.secondary.unwrap().remaining_percent, 80.0);

        fs::remove_file(path).unwrap();
        assert!(scan_transcript_usage_with_index(home.path(), &mut index)
            .unwrap()
            .is_none());
    }
}
