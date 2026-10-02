use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    ClaudeCode,
    Webhook,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Codex => "codex",
            Provider::ClaudeCode => "claude_code",
            Provider::Webhook => "webhook",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NoticeEventType {
    SessionIdle,
    SessionEnd,
    TaskStart,
    TaskFinish,
    TaskFail,
    UserConfirm,
    Warning,
    Error,
}

impl NoticeEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            NoticeEventType::SessionIdle => "SESSION_IDLE",
            NoticeEventType::SessionEnd => "SESSION_END",
            NoticeEventType::TaskStart => "TASK_START",
            NoticeEventType::TaskFinish => "TASK_FINISH",
            NoticeEventType::TaskFail => "TASK_FAIL",
            NoticeEventType::UserConfirm => "USER_CONFIRM",
            NoticeEventType::Warning => "WARNING",
            NoticeEventType::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            NoticeLevel::Info => "info",
            NoticeLevel::Success => "success",
            NoticeLevel::Warning => "warning",
            NoticeLevel::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeEvent {
    pub id: String,
    pub version: i64,
    pub provider: Provider,
    pub event_type: NoticeEventType,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub title: String,
    pub content: String,
    pub level: NoticeLevel,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub raw_payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub service_status: String,
    pub today_total: i64,
    pub today_success: i64,
    pub today_failure: i64,
    pub today_confirmations: i64,
    pub recent_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    pub search: Option<String>,
    pub level: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    pub webhook_masked: Option<String>,
    pub has_webhook: bool,
    pub has_sign_secret: bool,
    pub enabled: bool,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed: bool,
    pub config_path: String,
    pub managed_block_hash: Option<String>,
    pub backup_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub config_path: String,
    pub will_create_config: bool,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub command: String,
    pub project: Option<String>,
    pub risk_level: String,
    pub rule: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTrafficStatus {
    pub active_sessions: i64,
    pub pending_approvals: i64,
    pub failures: i64,
    pub complete: bool,
    pub activity_started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficWidgetStatus {
    pub enabled: bool,
    pub always_on_top: bool,
    pub color: String,
    pub label: String,
    pub detail: String,
    pub active_sessions: i64,
    pub codex_active_sessions: i64,
    pub claude_active_sessions: i64,
    pub codex_status: ProviderTrafficStatus,
    pub claude_status: ProviderTrafficStatus,
    pub pending_approvals: i64,
    pub today_failures: i64,
    pub latest_event_title: Option<String>,
    pub codex_usage: Option<CodexUsageStatus>,
    pub claude_weekly_remaining: Option<f64>,
    pub manual_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageStatus {
    pub limit_id: String,
    pub limit_name: Option<String>,
    pub primary: Option<CodexUsageWindow>,
    pub secondary: Option<CodexUsageWindow>,
    pub plan_type: Option<String>,
    pub rate_limit_reached_type: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_minutes: i64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetConfig {
    pub enabled: bool,
    pub base_url: Option<String>,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MochiVoiceConfig {
    pub enabled: bool,
    pub serial_port: String,
    pub asr_url: String,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookResponse {
    #[serde(rename = "continue")]
    pub should_continue: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,
}
