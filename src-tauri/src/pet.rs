use std::time::Duration;

use sqlx::SqlitePool;

use crate::domain::{CodexUsageWindow, PetConfig, TrafficWidgetStatus};
use crate::storage;

const PET_ENABLED_KEY: &str = "pet_enabled";
const PET_BASE_URL_KEY: &str = "pet_base_url";
const PET_LAST_STATUS_KEY: &str = "pet_last_status";
const PET_LAST_PAYLOAD_KEY: &str = "pet_last_payload";

pub async fn config(pool: &SqlitePool) -> anyhow::Result<PetConfig> {
    Ok(PetConfig {
        enabled: storage::bool_setting(pool, PET_ENABLED_KEY, false).await?,
        base_url: storage::get_setting(pool, PET_BASE_URL_KEY).await?,
        last_status: storage::get_setting(pool, PET_LAST_STATUS_KEY).await?,
    })
}

pub async fn save_config(
    pool: &SqlitePool,
    enabled: bool,
    base_url: Option<String>,
) -> anyhow::Result<PetConfig> {
    storage::put_setting(
        pool,
        PET_ENABLED_KEY,
        if enabled { "true" } else { "false" },
    )
    .await?;
    storage::put_setting(
        pool,
        PET_BASE_URL_KEY,
        normalize_base_url(base_url.as_deref())
            .as_deref()
            .unwrap_or(""),
    )
    .await?;
    config(pool).await
}

pub async fn sync_current_status(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    let status = storage::traffic_widget_status(pool).await?;
    sync_status(pool, &status).await
}

pub async fn sync_status(
    pool: &SqlitePool,
    status: &TrafficWidgetStatus,
) -> anyhow::Result<Option<String>> {
    let config = config(pool).await?;
    if !config.enabled {
        return Ok(None);
    }

    let Some(base_url) = normalize_base_url(config.base_url.as_deref()) else {
        let message = "Pet sync is enabled but no Mochi URL is configured".to_string();
        storage::put_setting(pool, PET_LAST_STATUS_KEY, &message).await?;
        return Ok(Some(message));
    };

    let pet_state = pet_state_from_traffic(status);
    let (primary, secondary) = pet_usage_labels(status);
    let payload = pet_payload_key(pet_state, &primary, &secondary);
    if storage::get_setting(pool, PET_LAST_PAYLOAD_KEY)
        .await?
        .as_deref()
        == Some(payload.as_str())
    {
        return Ok(Some("Pet payload unchanged".to_string()));
    }

    send_pet_state_with_usage(&base_url, pet_state, &primary, &secondary).await?;
    storage::put_setting(pool, PET_LAST_PAYLOAD_KEY, &payload).await?;

    let message = format!("Sent {pet_state} to {base_url}");
    storage::put_setting(pool, PET_LAST_STATUS_KEY, &message).await?;
    Ok(Some(message))
}

pub async fn test_connection(pool: &SqlitePool) -> anyhow::Result<String> {
    let config = config(pool).await?;
    let Some(base_url) = normalize_base_url(config.base_url.as_deref()) else {
        anyhow::bail!("Mochi URL is not configured");
    };

    send_pet_state(&base_url, "ready").await?;
    let message = format!("Test sent ready to {base_url}");
    storage::put_setting(pool, PET_LAST_STATUS_KEY, &message).await?;
    Ok(message)
}

pub fn pet_state_from_traffic(status: &TrafficWidgetStatus) -> &'static str {
    if status.color == "red" {
        "failed"
    } else if status.color == "yellow" || status.pending_approvals > 0 {
        "waiting"
    } else if status.color == "running" {
        "running"
    } else if status.color == "green" && status.latest_event_title.is_some() {
        "complete"
    } else {
        "ready"
    }
}

fn normalize_base_url(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("http://{trimmed}"))
    }
}

fn pet_usage_labels(status: &TrafficWidgetStatus) -> (String, String) {
    let primary = status
        .codex_usage
        .as_ref()
        .and_then(|usage| usage.primary.as_ref())
        .map(format_usage_window)
        .unwrap_or_default();
    let secondary = status
        .codex_usage
        .as_ref()
        .and_then(|usage| usage.secondary.as_ref())
        .map(format_usage_window)
        .unwrap_or_default();
    (primary, secondary)
}

fn pet_payload_key(state: &str, primary: &str, secondary: &str) -> String {
    format!("{state}|{primary}|{secondary}")
}

async fn send_pet_state(base_url: &str, state: &str) -> anyhow::Result<()> {
    send_pet_state_with_usage(base_url, state, "", "").await
}

async fn send_pet_state_with_usage(
    base_url: &str,
    state: &str,
    primary: &str,
    secondary: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    client
        .get(format!("{base_url}/pet/status"))
        .query(&[("state", state), ("quota", primary), ("quota2", secondary)])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn format_usage_window(window: &CodexUsageWindow) -> String {
    let window_label = if window.window_minutes >= 1440 {
        format!(
            "{}d",
            (window.window_minutes as f64 / 1440.0).round() as i64
        )
    } else {
        format!("{}h", (window.window_minutes as f64 / 60.0).round() as i64)
    };
    format!("{} {:.0}%", window_label, window.remaining_percent.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(
        color: &str,
        pending_approvals: i64,
        latest_event_title: Option<&str>,
    ) -> TrafficWidgetStatus {
        TrafficWidgetStatus {
            enabled: true,
            always_on_top: true,
            color: color.to_string(),
            label: String::new(),
            detail: String::new(),
            active_sessions: 0,
            codex_active_sessions: 0,
            claude_active_sessions: 0,
            codex_status: Default::default(),
            claude_status: Default::default(),
            pending_approvals,
            today_failures: 0,
            latest_event_title: latest_event_title.map(ToOwned::to_owned),
            codex_usage: None,
            claude_weekly_remaining: None,
            manual_override: None,
        }
    }

    #[test]
    fn maps_traffic_status_to_pet_state_priority() {
        assert_eq!(pet_state_from_traffic(&status("red", 0, None)), "failed");
        assert_eq!(
            pet_state_from_traffic(&status("yellow", 0, None)),
            "waiting"
        );
        assert_eq!(
            pet_state_from_traffic(&status("running", 0, None)),
            "running"
        );
        assert_eq!(
            pet_state_from_traffic(&status("green", 0, Some("Codex task finished"))),
            "complete"
        );
        assert_eq!(pet_state_from_traffic(&status("green", 0, None)), "ready");
    }

    #[test]
    fn normalizes_pet_url() {
        assert_eq!(
            normalize_base_url(Some("192.168.1.23/")),
            Some("http://192.168.1.23".to_string())
        );
        assert_eq!(
            normalize_base_url(Some("http://192.168.1.23/")),
            Some("http://192.168.1.23".to_string())
        );
        assert_eq!(normalize_base_url(Some("  ")), None);
    }

    #[test]
    fn formats_usage_window_for_mochi() {
        let primary = CodexUsageWindow {
            used_percent: 12.0,
            remaining_percent: 88.0,
            window_minutes: 300,
            resets_at: None,
        };
        let secondary = CodexUsageWindow {
            used_percent: 59.0,
            remaining_percent: 41.0,
            window_minutes: 10080,
            resets_at: None,
        };

        assert_eq!(format_usage_window(&primary), "5h 88%");
        assert_eq!(format_usage_window(&secondary), "7d 41%");
    }

    #[tokio::test]
    async fn sync_status_skips_unchanged_payload_before_network() {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::initialize(dir.path()).await.unwrap();
        save_config(&pool, true, Some("http://127.0.0.1:1".to_string()))
            .await
            .unwrap();
        let status = status_with_usage("running", 88.0);
        storage::put_setting(&pool, PET_LAST_PAYLOAD_KEY, "running|5h 88%|7d 41%")
            .await
            .unwrap();

        let message = sync_status(&pool, &status).await.unwrap();

        assert_eq!(message.as_deref(), Some("Pet payload unchanged"));
    }

    fn status_with_usage(color: &str, remaining_percent: f64) -> TrafficWidgetStatus {
        let mut status = status(color, 0, None);
        status.codex_usage = Some(crate::domain::CodexUsageStatus {
            limit_id: "codex".to_string(),
            limit_name: None,
            primary: Some(CodexUsageWindow {
                used_percent: 100.0 - remaining_percent,
                remaining_percent,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: Some(CodexUsageWindow {
                used_percent: 59.0,
                remaining_percent: 41.0,
                window_minutes: 10080,
                resets_at: None,
            }),
            plan_type: None,
            rate_limit_reached_type: None,
            updated_at: chrono::Utc::now(),
        });
        status
    }
}
