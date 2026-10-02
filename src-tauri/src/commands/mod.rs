use tauri::{
    AppHandle, LogicalSize, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;

use crate::app_state::AppState;
use crate::channels::feishu;
use crate::domain::{
    ChannelConfig, DashboardSummary, EventFilter, HookPreview, HookStatus, MochiVoiceConfig,
    NoticeEvent, Pagination, PendingApproval, PetConfig, TrafficWidgetStatus,
};
use crate::{codex_usage, hooks, mochi_voice, pet, secret_store, storage};

const TRAFFIC_WIDGET_WIDTH: f64 = 232.0;
const TRAFFIC_WIDGET_HEIGHT: f64 = 76.0;

#[tauri::command]
pub async fn get_dashboard_summary(state: State<'_, AppState>) -> Result<DashboardSummary, String> {
    storage::dashboard_summary(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_events(
    state: State<'_, AppState>,
    filter: EventFilter,
    pagination: Pagination,
) -> Result<Vec<NoticeEvent>, String> {
    storage::list_events(&state.pool, filter, pagination)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn clear_events(state: State<'_, AppState>) -> Result<(), String> {
    storage::clear_events(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn refresh_runtime_status(
    state: State<'_, AppState>,
) -> Result<TrafficWidgetStatus, String> {
    storage::refresh_runtime_status(&state.pool)
        .await
        .map_err(|error| error.to_string())?;
    codex_usage::invalidate_cache();
    let snapshot = storage::traffic_widget_status_snapshot(&state.pool)
        .await
        .map_err(|error| error.to_string())?;
    sync_pet_when_usage_changes(&state, &snapshot).await;
    Ok(snapshot.status)
}

async fn sync_pet_when_usage_changes(
    state: &State<'_, AppState>,
    snapshot: &storage::TrafficWidgetStatusSnapshot,
) {
    if snapshot.codex_usage_changed {
        if let Err(error) = pet::sync_status(&state.pool, &snapshot.status).await {
            eprintln!("Notice pet usage sync failed: {error}");
        }
    }
}

#[tauri::command]
pub async fn get_app_locale(state: State<'_, AppState>) -> Result<String, String> {
    storage::get_setting(&state.pool, "app_locale")
        .await
        .map_err(|error| error.to_string())
        .map(|value| value.unwrap_or_else(|| "en".to_string()))
}

#[tauri::command]
pub async fn set_app_locale(state: State<'_, AppState>, locale: String) -> Result<String, String> {
    let locale = if locale == "zh-CN" { "zh-CN" } else { "en" };
    storage::put_setting(&state.pool, "app_locale", locale)
        .await
        .map_err(|error| error.to_string())?;
    Ok(locale.to_string())
}

#[tauri::command]
pub fn get_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_autostart_enabled(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|error| error.to_string())?;
    } else {
        autostart.disable().map_err(|error| error.to_string())?;
    }
    autostart.is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_channel_config(state: State<'_, AppState>) -> Result<ChannelConfig, String> {
    get_channel_config_from_settings(&state.pool).await
}

async fn get_channel_config_from_settings(
    pool: &sqlx::SqlitePool,
) -> Result<ChannelConfig, String> {
    let webhook_masked = storage::get_setting(pool, "feishu_webhook_masked")
        .await
        .map_err(|e| e.to_string())?;
    let has_sign_secret = storage::get_setting(pool, "feishu_has_sign_secret")
        .await
        .map_err(|e| e.to_string())?
        .map(|value| value == "true")
        .unwrap_or(false);
    let last_status = storage::get_setting(pool, "feishu_last_status")
        .await
        .map_err(|e| e.to_string())?;
    let enabled = storage::bool_setting(pool, "feishu_enabled", true)
        .await
        .map_err(|e| e.to_string())?;
    let has_webhook = webhook_masked.is_some();
    Ok(ChannelConfig {
        webhook_masked,
        has_webhook,
        has_sign_secret,
        enabled,
        last_status,
    })
}

#[tauri::command]
pub async fn set_feishu_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<ChannelConfig, String> {
    storage::put_setting(
        &state.pool,
        "feishu_enabled",
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(|e| e.to_string())?;
    get_channel_config(state).await
}

#[tauri::command]
pub async fn save_feishu_config(
    state: State<'_, AppState>,
    webhook_url: String,
    sign_secret: Option<String>,
) -> Result<ChannelConfig, String> {
    let config = state.config.read().await.clone();
    let webhook_url = webhook_url.trim();
    let existing = get_channel_config(state.clone()).await?;
    if webhook_url.is_empty() && !existing.has_webhook {
        return Err("请输入飞书 Webhook URL".to_string());
    }
    if !webhook_url.is_empty() {
        secret_store::set_secret(&config.webhook_key, webhook_url).map_err(|e| e.to_string())?;
        storage::put_setting(
            &state.pool,
            "feishu_webhook_masked",
            &feishu::mask_webhook(webhook_url),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(secret) = sign_secret.filter(|s| !s.trim().is_empty()) {
        secret_store::set_secret(&config.sign_secret_key, secret.trim())
            .map_err(|e| e.to_string())?;
        storage::put_setting(&state.pool, "feishu_has_sign_secret", "true")
            .await
            .map_err(|e| e.to_string())?;
    }
    get_channel_config(state).await
}

#[tauri::command]
pub async fn test_feishu_channel(state: State<'_, AppState>) -> Result<String, String> {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::domain::{NoticeEventType, NoticeLevel, Provider};

    let config = state.config.read().await.clone();
    let enabled = storage::bool_setting(&state.pool, "feishu_enabled", true)
        .await
        .map_err(|e| e.to_string())?;
    if !enabled {
        let status = "Feishu notifications are disabled".to_string();
        storage::put_setting(&state.pool, "feishu_last_status", &status)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(status);
    }
    let event = NoticeEvent {
        id: Uuid::new_v4().to_string(),
        version: 1,
        provider: Provider::Webhook,
        event_type: NoticeEventType::TaskFinish,
        session_id: None,
        run_id: None,
        dedupe_key: Some(format!("test-{}", Utc::now().timestamp())),
        title: "Notice test notification".to_string(),
        content: "Feishu channel is configured correctly.".to_string(),
        level: NoticeLevel::Success,
        project: Some("Notice".to_string()),
        cwd: None,
        command: None,
        exit_code: None,
        duration_ms: None,
        timestamp: Utc::now(),
        received_at: Utc::now(),
        raw_payload: None,
    };
    storage::insert_event(&state.pool, &event)
        .await
        .map_err(|e| e.to_string())?;
    let status = feishu::send_event(
        &state.pool,
        &config.webhook_key,
        &config.sign_secret_key,
        &event,
    )
    .await
    .map_err(|e| e.to_string())?;
    storage::put_setting(&state.pool, "feishu_last_status", &status)
        .await
        .map_err(|e| e.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn get_hook_status() -> Result<HookStatus, String> {
    hooks::status().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_hook_install(state: State<'_, AppState>) -> Result<HookPreview, String> {
    hooks::preview(state.data_dir.clone()).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn install_codex_hooks(state: State<'_, AppState>) -> Result<HookStatus, String> {
    let hook_dir = state.data_dir.join("hooks");
    tokio::fs::create_dir_all(&hook_dir)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::write(hook_dir.join("token"), state.token.as_str())
        .await
        .map_err(|error| error.to_string())?;
    hooks::install(state.data_dir.clone()).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn uninstall_codex_hooks(state: State<'_, AppState>) -> Result<HookStatus, String> {
    hooks::uninstall(state.data_dir.clone()).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_pending_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<PendingApproval>, String> {
    storage::list_pending_approvals(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_approval(
    state: State<'_, AppState>,
    id: String,
    decision: String,
) -> Result<(), String> {
    let status = if decision == "approved" {
        "approved"
    } else {
        "rejected"
    };
    storage::resolve_approval(&state.pool, &id, status)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_traffic_widget_status(
    state: State<'_, AppState>,
) -> Result<TrafficWidgetStatus, String> {
    let snapshot = storage::traffic_widget_status_snapshot(&state.pool)
        .await
        .map_err(|error| error.to_string())?;
    sync_pet_when_usage_changes(&state, &snapshot).await;
    Ok(snapshot.status)
}

#[tauri::command]
pub async fn get_pet_config(state: State<'_, AppState>) -> Result<PetConfig, String> {
    pet::config(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_pet_config(
    state: State<'_, AppState>,
    enabled: bool,
    base_url: Option<String>,
) -> Result<PetConfig, String> {
    pet::save_config(&state.pool, enabled, base_url)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn test_pet_connection(state: State<'_, AppState>) -> Result<String, String> {
    pet::test_connection(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_mochi_voice_config(
    state: State<'_, AppState>,
) -> Result<MochiVoiceConfig, String> {
    mochi_voice::config(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_mochi_voice_config(
    state: State<'_, AppState>,
    enabled: bool,
    serial_port: Option<String>,
    asr_url: Option<String>,
) -> Result<MochiVoiceConfig, String> {
    mochi_voice::save_config(&state.pool, enabled, serial_port, asr_url)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_traffic_widget_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<TrafficWidgetStatus, String> {
    storage::put_setting(
        &state.pool,
        "traffic_widget_enabled",
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(|error| error.to_string())?;

    if enabled {
        show_traffic_widget_for_state(&app, &state).await?;
    } else if let Some(window) = app.get_webview_window("traffic-widget") {
        window.hide().map_err(|error| error.to_string())?;
    }

    storage::traffic_widget_status(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_traffic_widget_always_on_top(
    app: AppHandle,
    state: State<'_, AppState>,
    always_on_top: bool,
) -> Result<TrafficWidgetStatus, String> {
    storage::put_setting(
        &state.pool,
        "traffic_widget_always_on_top",
        if always_on_top { "true" } else { "false" },
    )
    .await
    .map_err(|error| error.to_string())?;

    if let Some(window) = app.get_webview_window("traffic-widget") {
        window
            .set_always_on_top(always_on_top)
            .map_err(|error| error.to_string())?;
    }

    storage::traffic_widget_status(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_traffic_widget_manual_override(
    state: State<'_, AppState>,
    manual_state: Option<String>,
) -> Result<TrafficWidgetStatus, String> {
    storage::set_traffic_widget_manual_override(&state.pool, manual_state)
        .await
        .map_err(|error| error.to_string())?;
    storage::traffic_widget_status(&state.pool)
        .await
        .map_err(|error| error.to_string())
}

pub async fn show_traffic_widget_for_state(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let always_on_top = storage::bool_setting(&state.pool, "traffic_widget_always_on_top", true)
        .await
        .map_err(|error| error.to_string())?;
    show_traffic_widget(app, always_on_top)
}

pub fn show_traffic_widget(app: &AppHandle, always_on_top: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("traffic-widget") {
        resize_traffic_widget(&window)?;
        window
            .set_always_on_top(always_on_top)
            .map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        "traffic-widget",
        WebviewUrl::App("index.html?widget=traffic".into()),
    )
    .title("Notice Status")
    .inner_size(TRAFFIC_WIDGET_WIDTH, TRAFFIC_WIDGET_HEIGHT)
    .min_inner_size(TRAFFIC_WIDGET_WIDTH, TRAFFIC_WIDGET_HEIGHT)
    .max_inner_size(TRAFFIC_WIDGET_WIDTH, TRAFFIC_WIDGET_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .accept_first_mouse(true)
    .always_on_top(always_on_top)
    .skip_taskbar(true)
    .build()
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn resize_traffic_widget(window: &WebviewWindow) -> Result<(), String> {
    let size = LogicalSize::new(TRAFFIC_WIDGET_WIDTH, TRAFFIC_WIDGET_HEIGHT);
    window
        .set_min_size(Some(size))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(size))
        .map_err(|error| error.to_string())?;
    window.set_size(size).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::storage;

    use super::get_channel_config_from_settings;

    #[tokio::test]
    async fn channel_config_reads_masked_settings_without_secret_lookup() {
        let dir = tempdir().unwrap();
        let pool = storage::initialize(dir.path()).await.unwrap();
        storage::put_setting(
            &pool,
            "feishu_webhook_masked",
            "https://open.feishu.cn/...abcd",
        )
        .await
        .unwrap();
        storage::put_setting(&pool, "feishu_has_sign_secret", "true")
            .await
            .unwrap();
        storage::put_setting(&pool, "feishu_last_status", "ok")
            .await
            .unwrap();

        let config = get_channel_config_from_settings(&pool).await.unwrap();

        assert!(config.has_webhook);
        assert_eq!(
            config.webhook_masked.as_deref(),
            Some("https://open.feishu.cn/...abcd")
        );
        assert!(config.has_sign_secret);
        assert!(config.enabled);
        assert_eq!(config.last_status.as_deref(), Some("ok"));
    }
}

#[tauri::command]
pub async fn get_claude_hook_status(state: State<'_, AppState>) -> Result<HookStatus,String> { hooks::claude::status(&state.data_dir).map_err(|e|e.to_string()) }
#[tauri::command]
pub async fn preview_claude_hook_install(state: State<'_, AppState>) -> Result<HookPreview,String> { hooks::claude::preview(&state.data_dir).map_err(|e|e.to_string()) }
#[tauri::command]
pub async fn install_claude_hooks(state: State<'_, AppState>) -> Result<HookStatus,String> { hooks::claude::update(&state.data_dir,true).map_err(|e|e.to_string()) }
#[tauri::command]
pub async fn uninstall_claude_hooks(state: State<'_, AppState>) -> Result<HookStatus,String> { hooks::claude::update(&state.data_dir,false).map_err(|e|e.to_string()) }
#[tauri::command]
pub async fn get_claude_usage_status(state: State<'_, AppState>) -> Result<crate::claude_usage::UsageStatus,String> { Ok(crate::claude_usage::status(&state.pool).await) }
#[tauri::command]
pub async fn open_claude_account(app: AppHandle, state: State<'_, AppState>) -> Result<(),String> {
    storage::put_setting(&state.pool,"claude_usage_enabled","true").await.map_err(|e|e.to_string())?;
    crate::claude_usage::open_account(&app,true)
}
#[tauri::command]
pub async fn refresh_claude_usage(app: AppHandle) -> Result<crate::claude_usage::UsageStatus,String> { crate::claude_usage::refresh(&app).await }
