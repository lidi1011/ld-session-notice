use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::domain::{CodexUsageStatus, CodexUsageWindow};

const CACHE_TTL: Duration = Duration::from_secs(5);
const GENERAL_LIMIT_STALE_AFTER: Duration = Duration::from_secs(120);

static CACHE: OnceLock<Mutex<Option<CachedUsage>>> = OnceLock::new();

#[derive(Clone)]
struct CachedUsage {
    loaded_at: Instant,
    value: Option<CodexUsageStatus>,
}

pub async fn latest() -> Option<CodexUsageStatus> {
    if let Some(value) = cached_value() {
        return value;
    }

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

pub fn invalidate_cache() {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = None;
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
    let mut candidates = UsageCandidates::default();
    for root in [
        home.join(".codex").join("sessions"),
        home.join(".codex").join("archived_sessions"),
    ] {
        scan_dir(&root, &mut candidates)?;
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

#[derive(Default)]
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

fn scan_dir(path: &Path, candidates: &mut UsageCandidates) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(path) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, candidates)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            scan_file(&path, candidates)?;
        }
    }

    Ok(())
}

fn scan_file(path: &Path, candidates: &mut UsageCandidates) -> anyhow::Result<()> {
    let file = File::open(path)?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"rate_limits\"") {
            continue;
        }
        if let Some(status) = parse_usage_line(&line) {
            candidates.record(status);
        }
    }
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
        cached_value, invalidate_cache, parse_codexmate_bootstrap_active_usage,
        parse_codexmate_quota_store, parse_usage_line, scan_latest_usage_for_home, CachedUsage,
        UsageCandidates, CACHE,
    };
    use std::fs;
    use std::time::Instant;

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
    fn invalidate_cache_clears_cached_usage() {
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
        *cache.lock().unwrap() = Some(CachedUsage {
            loaded_at: Instant::now(),
            value: None,
        });

        assert!(cached_value().is_some());
        invalidate_cache();
        assert!(cached_value().is_none());
    }
}
