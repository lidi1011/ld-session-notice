use axum::{extract::State, http::{HeaderMap, StatusCode}, Json};
use serde_json::Value;
use crate::{app_state::AppState, domain::{NoticeEvent, NoticeEventType, NoticeLevel, Provider}, storage};
use super::{authorized, event_from_payload, send_feishu_if_enabled, sync_pet_status};

// This endpoint only observes Claude. It never returns a permission decision.
pub(super) async fn webhook(
    State(state): State<AppState>, headers: HeaderMap, Json(payload): Json<Value>,
) -> StatusCode {
    if !authorized(&state, &headers) { return StatusCode::UNAUTHORIZED; }
    let event = match normalize(&payload) {
        Ok(Some(event)) => event,
        Ok(None) => return StatusCode::NO_CONTENT,
        Err(()) => return StatusCode::BAD_REQUEST,
    };
    if storage::insert_event(&state.pool, &event).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    // Persist before acknowledging; network notifications must not delay the hook.
    tokio::spawn(async move {
        if matches!(event.event_type, NoticeEventType::TaskFinish | NoticeEventType::TaskFail | NoticeEventType::UserConfirm) {
            send_feishu_if_enabled(&state, &event).await;
        }
        sync_pet_status(&state).await;
    });
    StatusCode::NO_CONTENT
}

fn normalize(payload: &Value) -> Result<Option<NoticeEvent>, ()> {
    let session = payload.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or(())?;
    let name = payload.get("hook_event_name").and_then(Value::as_str).ok_or(())?;
    use NoticeEventType::{SessionIdle, SessionEnd, TaskStart, TaskFinish, TaskFail, UserConfirm};
    use NoticeLevel::*;
    let (kind, level, title) = match name {
        "SessionStart" => (SessionIdle, Info, "Claude session ready"),
        "SessionEnd" => (SessionEnd, Info, "Claude session ended"),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "ElicitationResult" => (TaskStart, Info, "Claude task running"),
        // A failed tool can be recovered by the agent; it is not a failed turn.
        "PostToolUseFailure" => (TaskStart, Warning, "Claude tool failed; task continuing"),
        "PermissionRequest" | "Elicitation" => (UserConfirm, Warning, "Claude needs confirmation"),
        "Notification" if payload.get("notification_type").and_then(Value::as_str) == Some("permission_prompt") =>
            (UserConfirm, Warning, "Claude needs confirmation"),
        "Notification" if payload.get("notification_type").and_then(Value::as_str) == Some("idle_prompt") =>
            (SessionIdle, Info, "Claude waiting for a new prompt"),
        "Stop" => (TaskFinish, Success, "Claude response finished"),
        "StopFailure" => (TaskFail, Error, "Claude response failed"),
        _ => return Ok(None),
    };
    // Keep only status metadata: no prompts, transcript contents or tool arguments.
    let metadata = serde_json::json!({
        "session_id": session,
        "hook_event_name": name,
        "cwd": payload.get("cwd"),
    });
    let mut event = event_from_payload(name, &metadata, None, kind, level, title);
    event.provider = Provider::ClaudeCode;
    event.content = format!("{name} received from Claude Code");
    event.dedupe_key = Some(format!("claude_code:{}", event.id));
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_lifecycle_without_recording_prompt() {
        for (name, kind) in [
            ("SessionStart", NoticeEventType::SessionIdle),
            ("UserPromptSubmit", NoticeEventType::TaskStart),
            ("PermissionRequest", NoticeEventType::UserConfirm),
            ("PostToolUseFailure", NoticeEventType::TaskStart),
            ("Stop", NoticeEventType::TaskFinish),
            ("StopFailure", NoticeEventType::TaskFail),
            ("SessionEnd", NoticeEventType::SessionEnd),
        ] {
            let event = normalize(&serde_json::json!({"session_id":"test", "hook_event_name":name,"prompt":"private prompt"})).unwrap().unwrap();
            assert_eq!(event.provider, Provider::ClaudeCode);
            assert_eq!(event.event_type, kind);
            assert!(!event.raw_payload.unwrap().to_string().contains("private prompt"));
        }
        assert!(normalize(&serde_json::json!({"hook_event_name":"Stop"})).is_err());
    }
}
