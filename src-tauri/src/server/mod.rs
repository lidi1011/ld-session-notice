mod claude;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::channels::feishu;
use crate::domain::{HookResponse, NoticeEvent, NoticeEventType, NoticeLevel, Provider};
use crate::rules::redaction::redact;
use crate::rules::risk::{classify_command, RiskLevel};
use crate::{codex_usage, pet, storage};

pub async fn run(state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/webhook/codex", post(codex_webhook))
        .route("/api/webhook/claude-code", post(claude::webhook))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:3746").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn codex_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(HookResponse {
                should_continue: false,
                stop_reason: Some("Notice token rejected".to_string()),
                suppress_output: Some(true),
            }),
        );
    }

    match handle_codex_payload(state.clone(), payload).await {
        Ok((response, should_sync_pet)) => {
            refresh_codex_usage_after_hook();
            if should_sync_pet {
                sync_pet_status(&state).await;
            }
            (StatusCode::OK, Json(response))
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(HookResponse {
                should_continue: true,
                stop_reason: Some(format!("Notice failed open: {error}")),
                suppress_output: Some(false),
            }),
        ),
    }
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get("X-Notice-Token")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == state.token.as_str())
        .unwrap_or(false)
}

async fn handle_codex_payload(
    state: AppState,
    payload: Value,
) -> anyhow::Result<(HookResponse, bool)> {
    let event_name = payload
        .get("hook_event_name")
        .or_else(|| payload.get("hookEventName"))
        .and_then(Value::as_str)
        .unwrap_or("PostToolUse");
    let command = extract_command(&payload);
    let project = payload
        .get("project")
        .or_else(|| payload.get("cwd"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    if event_name == "PreToolUse" {
        if let Some(command) = command.clone() {
            let risk = classify_command(&command);
            if matches!(risk.level, RiskLevel::Critical) {
                let approval = storage::create_pending_approval(
                    &state.pool,
                    &redact(&command),
                    project.as_deref(),
                    "critical",
                    &risk.rule,
                    30,
                )
                .await?;
                let event = event_from_payload(
                    event_name,
                    &payload,
                    Some(command),
                    NoticeEventType::UserConfirm,
                    NoticeLevel::Warning,
                    "High-risk command detected",
                );
                storage::insert_event(&state.pool, &event).await?;
                send_feishu_if_enabled(&state, &event).await;
                sync_pet_status(&state).await;
                let response = wait_for_approval(&state, &approval.id).await?;
                if response.should_continue {
                    let resumed = event_from_payload(
                        event_name,
                        &payload,
                        None,
                        NoticeEventType::TaskStart,
                        NoticeLevel::Info,
                        "Codex task resumed",
                    );
                    storage::insert_event(&state.pool, &resumed).await?;
                }
                return Ok((response, true));
            }
        }
    }

    let (event_type, level, title) = match event_name {
        "SessionStart" => (
            NoticeEventType::TaskStart,
            NoticeLevel::Info,
            "Codex session started",
        ),
        "UserPromptSubmit" => (
            NoticeEventType::TaskStart,
            NoticeLevel::Info,
            "Codex task started",
        ),
        "PermissionRequest" => (
            NoticeEventType::UserConfirm,
            NoticeLevel::Warning,
            "Codex needs confirmation",
        ),
        "Stop" => (
            NoticeEventType::TaskFinish,
            NoticeLevel::Success,
            "Codex task finished",
        ),
        "PostToolUse" => {
            let exit_code = payload
                .get("tool_response")
                .and_then(|v| v.get("exit_code"))
                .or_else(|| payload.get("exit_code"))
                .and_then(Value::as_i64);
            if exit_code.unwrap_or(0) == 0 {
                let resumed_from_approval =
                    resume_after_approval_if_needed(&state, &payload).await?;
                touch_session_activity(&state, event_name, &payload).await?;
                return Ok((
                    HookResponse {
                        should_continue: true,
                        stop_reason: None,
                        suppress_output: Some(true),
                    },
                    resumed_from_approval,
                ));
            } else {
                (
                    NoticeEventType::TaskFail,
                    NoticeLevel::Error,
                    "Codex tool failed",
                )
            }
        }
        "PreToolUse" => {
            let resumed_from_approval = resume_after_approval_if_needed(&state, &payload).await?;
            touch_session_activity(&state, event_name, &payload).await?;
            return Ok((
                HookResponse {
                    should_continue: true,
                    stop_reason: None,
                    suppress_output: Some(true),
                },
                resumed_from_approval,
            ));
        }
        _ => (
            NoticeEventType::Warning,
            NoticeLevel::Info,
            "Codex event received",
        ),
    };

    let event = event_from_payload(event_name, &payload, command, event_type, level, title);
    storage::insert_event(&state.pool, &event).await?;

    let session_has_failures =
        storage::session_has_failures(&state.pool, event.session_id.as_deref()).await?;
    let should_notify = should_notify_feishu(
        event_name,
        &event.event_type,
        &event.level,
        session_has_failures,
    );

    if should_notify {
        send_feishu_if_enabled(&state, &event).await;
    }

    Ok((
        HookResponse {
            should_continue: true,
            stop_reason: None,
            suppress_output: Some(true),
        },
        true,
    ))
}

async fn resume_after_approval_if_needed(
    state: &AppState,
    payload: &Value,
) -> anyhow::Result<bool> {
    let session_id = payload
        .get("session_id")
        .or_else(|| payload.get("sessionId"))
        .and_then(Value::as_str);
    let latest = storage::latest_non_stop_event_type(&state.pool, session_id).await?;
    if latest.as_deref() != Some("USER_CONFIRM") {
        return Ok(false);
    }

    let event = event_from_payload(
        "ApprovalResolved",
        payload,
        None,
        NoticeEventType::TaskStart,
        NoticeLevel::Info,
        "Codex task resumed",
    );
    storage::insert_event(&state.pool, &event).await?;
    Ok(true)
}

async fn touch_session_activity(
    state: &AppState,
    event_name: &str,
    payload: &Value,
) -> anyhow::Result<()> {
    let event = event_from_payload(
        event_name,
        payload,
        None,
        NoticeEventType::TaskStart,
        NoticeLevel::Info,
        "Codex task activity",
    );
    storage::touch_session_activity(&state.pool, &event).await
}

fn should_notify_feishu(
    event_name: &str,
    event_type: &NoticeEventType,
    level: &NoticeLevel,
    session_has_failures: bool,
) -> bool {
    matches!(event_type, NoticeEventType::UserConfirm)
        || (event_name == "Stop" && matches!(level, NoticeLevel::Success) && !session_has_failures)
}

async fn sync_pet_status(state: &AppState) {
    if let Err(error) = pet::sync_current_status(&state.pool).await {
        eprintln!("Notice pet sync failed: {error}");
    }
}

fn refresh_codex_usage_after_hook() {
    codex_usage::invalidate_cache();
    tokio::spawn(async {
        sleep(Duration::from_millis(1500)).await;
        codex_usage::invalidate_cache();
    });
}

async fn send_feishu_if_enabled(state: &AppState, event: &NoticeEvent) {
    match storage::bool_setting(&state.pool, "feishu_enabled", true).await {
        Ok(true) => {
            let config = state.config.read().await.clone();
            let _ = feishu::send_event(
                &state.pool,
                &config.webhook_key,
                &config.sign_secret_key,
                event,
            )
            .await;
        }
        Ok(false) => {
            let _ = storage::put_setting(&state.pool, "feishu_last_status", "disabled").await;
        }
        Err(error) => {
            eprintln!("Notice failed to read Feishu enabled setting: {error}");
        }
    }
}

async fn wait_for_approval(state: &AppState, id: &str) -> anyhow::Result<HookResponse> {
    for _ in 0..120 {
        if let Some(status) = storage::approval_status(&state.pool, id).await? {
            match status.as_str() {
                "approved" => {
                    return Ok(HookResponse {
                        should_continue: true,
                        stop_reason: None,
                        suppress_output: Some(true),
                    });
                }
                "rejected" => {
                    return Ok(HookResponse {
                        should_continue: false,
                        stop_reason: Some("Rejected in Notice".to_string()),
                        suppress_output: Some(false),
                    });
                }
                _ => {}
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    storage::resolve_approval(&state.pool, id, "timed_out").await?;
    Ok(HookResponse {
        should_continue: false,
        stop_reason: Some("Notice approval timed out".to_string()),
        suppress_output: Some(false),
    })
}

fn event_from_payload(
    event_name: &str,
    payload: &Value,
    command: Option<String>,
    event_type: NoticeEventType,
    level: NoticeLevel,
    title: &str,
) -> NoticeEvent {
    let now = Utc::now();
    let command = command.map(|value| redact(&value));
    let content = command
        .clone()
        .unwrap_or_else(|| format!("{event_name} received from Codex"));
    NoticeEvent {
        id: Uuid::new_v4().to_string(),
        version: 1,
        provider: Provider::Codex,
        event_type,
        session_id: payload
            .get("session_id")
            .or_else(|| payload.get("sessionId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        run_id: payload
            .get("run_id")
            .or_else(|| payload.get("runId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        dedupe_key: Some(format!(
            "codex:{}:{}",
            event_name,
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        )),
        title: title.to_string(),
        content: redact(&content),
        level,
        project: payload
            .get("project")
            .or_else(|| payload.get("cwd"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        command,
        exit_code: payload
            .get("tool_response")
            .and_then(|v| v.get("exit_code"))
            .or_else(|| payload.get("exit_code"))
            .and_then(Value::as_i64),
        duration_ms: payload
            .get("duration_ms")
            .or_else(|| payload.get("durationMs"))
            .and_then(Value::as_i64),
        timestamp: now,
        received_at: now,
        raw_payload: Some(payload.clone()),
    }
}

fn extract_command(payload: &Value) -> Option<String> {
    payload
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .or_else(|| {
            payload
                .get("toolInput")
                .and_then(|input| input.get("command"))
        })
        .or_else(|| payload.get("command"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[allow(dead_code)]
fn _body_type(_: Body) {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    use super::{handle_codex_payload, should_notify_feishu};
    use crate::app_state::{AppState, RuntimeConfig};
    use crate::domain::{EventFilter, NoticeEventType, NoticeLevel, Pagination};
    use crate::storage;

    async fn test_state() -> AppState {
        let dir = tempdir().unwrap().keep();
        let pool = storage::initialize(&dir).await.unwrap();
        AppState {
            pool,
            data_dir: dir,
            token: Arc::new("test-token".to_string()),
            config: Arc::new(RwLock::new(RuntimeConfig::default())),
        }
    }

    #[test]
    fn feishu_notifies_for_user_approval() {
        assert!(should_notify_feishu(
            "PermissionRequest",
            &NoticeEventType::UserConfirm,
            &NoticeLevel::Warning,
            false,
        ));
    }

    #[test]
    fn feishu_notifies_for_successful_session_stop() {
        assert!(should_notify_feishu(
            "Stop",
            &NoticeEventType::TaskFinish,
            &NoticeLevel::Success,
            false,
        ));
    }

    #[test]
    fn feishu_skips_failed_or_incomplete_session_stop() {
        assert!(!should_notify_feishu(
            "Stop",
            &NoticeEventType::TaskFinish,
            &NoticeLevel::Success,
            true,
        ));
        assert!(!should_notify_feishu(
            "PostToolUse",
            &NoticeEventType::TaskFail,
            &NoticeLevel::Error,
            true,
        ));
    }

    #[tokio::test]
    async fn user_prompt_submit_records_task_start_but_successful_tool_use_is_hidden() {
        let state = test_state().await;
        let (_, should_sync_pet) = handle_codex_payload(
            state.clone(),
            json!({
                "hook_event_name": "UserPromptSubmit",
                "session_id": "session-1",
                "cwd": "/tmp/demo"
            }),
        )
        .await
        .unwrap();
        assert!(should_sync_pet);

        let (_, should_sync_pet) = handle_codex_payload(
            state.clone(),
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "session-1",
                "tool_input": { "command": "ls" }
            }),
        )
        .await
        .unwrap();
        assert!(!should_sync_pet);

        let (_, should_sync_pet) = handle_codex_payload(
            state.clone(),
            json!({
                "hook_event_name": "PostToolUse",
                "session_id": "session-1",
                "tool_response": { "exit_code": 0 }
            }),
        )
        .await
        .unwrap();
        assert!(!should_sync_pet);

        let events = storage::list_events(
            &state.pool,
            EventFilter {
                search: None,
                level: None,
                project: None,
            },
            Pagination {
                page: 1,
                page_size: 10,
            },
        )
        .await
        .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, NoticeEventType::TaskStart);
        assert_eq!(events[0].title, "Codex task started");
    }

    #[tokio::test]
    async fn pre_tool_use_after_permission_request_marks_task_resumed() {
        let state = test_state().await;
        let (_, should_sync_pet) = handle_codex_payload(
            state.clone(),
            json!({
                "hook_event_name": "PermissionRequest",
                "session_id": "session-1",
                "cwd": "/tmp/demo"
            }),
        )
        .await
        .unwrap();
        assert!(should_sync_pet);

        let (_, should_sync_pet) = handle_codex_payload(
            state.clone(),
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "session-1",
                "tool_input": { "command": "ls" }
            }),
        )
        .await
        .unwrap();
        assert!(should_sync_pet);

        let events = storage::list_events(
            &state.pool,
            EventFilter {
                search: None,
                level: None,
                project: None,
            },
            Pagination {
                page: 1,
                page_size: 10,
            },
        )
        .await
        .unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, NoticeEventType::TaskStart);
        assert_eq!(events[0].title, "Codex task resumed");
        assert_eq!(events[1].event_type, NoticeEventType::UserConfirm);
    }
}
