use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::json;
use sha2::Sha256;
use sqlx::SqlitePool;
use tokio::time::sleep;

use crate::domain::NoticeEvent;
use crate::rules::redaction::redact;
use crate::{secret_store, storage};

type HmacSha256 = Hmac<Sha256>;

pub async fn send_event(
    pool: &SqlitePool,
    webhook_key: &str,
    sign_key: &str,
    event: &NoticeEvent,
) -> anyhow::Result<String> {
    if let Some(dedupe_key) = event.dedupe_key.as_deref() {
        if storage::recent_delivery_exists(pool, dedupe_key).await? {
            return Ok("deduped".to_string());
        }
    }

    let webhook =
        secret_store::get_secret(webhook_key)?.context("Feishu webhook is not configured")?;
    let sign_secret = secret_store::get_secret(sign_key)?;
    let client = Client::builder().timeout(Duration::from_secs(5)).build()?;
    let text = redact(&format!(
        "{}\n\n项目：{}\n\n{}",
        event.title,
        event
            .project
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        event.content
    ));
    let mut payload = json!({
        "msg_type": "text",
        "content": { "text": text }
    });

    if let Some(secret) = sign_secret.filter(|s| !s.is_empty()) {
        let timestamp = Utc::now().timestamp().to_string();
        let sign = feishu_sign(&timestamp, &secret)?;
        payload["timestamp"] = json!(timestamp);
        payload["sign"] = json!(sign);
    }

    let mut last_error = None;
    for attempt in 1..=3 {
        let response = client.post(&webhook).json(&payload).send().await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                storage::insert_delivery_attempt(
                    pool,
                    &event.id,
                    "feishu",
                    event.dedupe_key.as_deref(),
                    "sent",
                    attempt,
                    None,
                )
                .await?;
                return Ok("sent".to_string());
            }
            Ok(resp) => {
                last_error = Some(format!("HTTP {}", resp.status()));
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }
        sleep(Duration::from_millis(200 * attempt as u64)).await;
    }

    storage::insert_delivery_attempt(
        pool,
        &event.id,
        "feishu",
        event.dedupe_key.as_deref(),
        "failed",
        3,
        last_error.as_deref(),
    )
    .await?;
    anyhow::bail!(last_error.unwrap_or_else(|| "Feishu delivery failed".to_string()))
}

pub fn mask_webhook(value: &str) -> String {
    if let Some((prefix, _)) = value.split_once("/hook/") {
        return format!("{prefix}/hook/<redacted>");
    }
    if value.len() <= 12 {
        return "<configured>".to_string();
    }
    format!("{}...<redacted>", &value[..8])
}

fn feishu_sign(timestamp: &str, secret: &str) -> anyhow::Result<String> {
    let string_to_sign = format!("{timestamp}\n{secret}");
    let mut mac = HmacSha256::new_from_slice(string_to_sign.as_bytes())?;
    mac.update(&[]);
    let result = mac.finalize().into_bytes();
    Ok(base64::engine::general_purpose::STANDARD.encode(result))
}

#[cfg(test)]
mod tests {
    use super::mask_webhook;

    #[test]
    fn masks_webhook_url() {
        let masked = mask_webhook("https://open.feishu.cn/open-apis/bot/v2/hook/abcdef");
        assert!(masked.starts_with("https://"));
        assert!(masked.contains("<redacted>"));
        assert!(!masked.contains("abcdef"));
    }
}
