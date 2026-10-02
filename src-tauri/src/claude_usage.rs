use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
mod browser;
use crate::{app_state::AppState, storage};

const SNAPSHOT: &str = "claude_usage_snapshot";
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatus {
    pub remaining_percent: Option<f64>,
    pub updated_at: Option<DateTime<Utc>>,
    pub message: String,
}

pub async fn status(pool: &SqlitePool) -> UsageStatus {
    let raw = storage::get_setting(pool, SNAPSHOT).await.ok().flatten();
    let value = raw.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let remaining = value.as_ref().and_then(|v| parse_weekly(v, Utc::now()));
    let updated = value
        .as_ref()
        .and_then(|v| v.get("updated_at"))
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let message = storage::get_setting(pool, "claude_usage_message")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "请在当前默认浏览器登录 Claude，然后刷新额度".into());
    UsageStatus {
        remaining_percent: remaining,
        updated_at: updated,
        message,
    }
}

pub async fn weekly_remaining(pool: &SqlitePool) -> Option<f64> {
    status(pool).await.remaining_percent
}

pub fn open_account(_app: &AppHandle, _visible: bool) -> Result<(), String> {
    browser::selected()?.open()
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            if storage::bool_setting(&state.pool, "claude_usage_enabled", false)
                .await
                .unwrap_or(false)
            {
                let _ = refresh(&app).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        }
    });
}

async fn fetch_usage() -> Result<Value, String> {
    let browser = browser::selected()?;
    let cookies = browser.cookies().await?;
    if !cookies.contains_key("sessionKey") {
        return Err(format!(
            "{} 当前个人资料尚未登录 Claude，请在浏览器登录后刷新",
            browser.name
        ));
    }
    let cookie = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    // No redirects: authentication cookies must only reach the intended Claude API origin.
    let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(20)).user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36").build().map_err(|_|"无法创建额度连接")?;
    async fn get(client: &reqwest::Client, cookie: &str, path: &str) -> Result<Value, String> {
        let response = client
            .get(format!("https://claude.ai/api/{path}"))
            .header(reqwest::header::COOKIE, cookie)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| "Claude 额度请求失败，请检查网络后重试")?;
        match response.status().as_u16() {
            401 => return Err("浏览器 Claude 登录已失效，请登录后刷新".into()),
            403 => return Err("Claude 拒绝额度请求，请在浏览器打开额度页完成验证后重试".into()),
            429 => return Err("额度查询过于频繁，请稍后重试".into()),
            200 => {}
            _ => return Err("Claude 额度服务暂时不可用".into()),
        }
        response
            .json()
            .await
            .map_err(|_| "Claude 额度响应无法解析".into())
    }
    let org = if let Some(org) = cookies.get("lastActiveOrg") {
        org.clone()
    } else {
        let data = get(&client, &cookie, "organizations").await?;
        data.as_array()
            .and_then(|a| a.first())
            .and_then(|v| v["uuid"].as_str())
            .ok_or("账户没有可读取的组织")?
            .to_string()
    };
    if org.is_empty() || !org.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("账户组织标识无效".into());
    }
    let data = get(&client, &cookie, &format!("organizations/{org}/usage")).await?;
    Ok(
        serde_json::json!({"weekly_percent":data["seven_day"]["utilization"],"weekly_resets_at":data["seven_day"]["resets_at"],"updated_at":Utc::now().to_rfc3339()}),
    )
}

pub async fn refresh(app: &AppHandle) -> Result<UsageStatus, String> {
    let state = app.state::<AppState>();
    let Ok(_guard) = REFRESH.try_lock() else {
        return Ok(status(&state.pool).await);
    };
    storage::put_setting(&state.pool, "claude_usage_enabled", "true")
        .await
        .map_err(|e| e.to_string())?;
    let message = match fetch_usage().await {
        Ok(value) if parse_weekly(&value, Utc::now()).is_some() => {
            storage::put_setting(&state.pool, SNAPSHOT, &value.to_string())
                .await
                .map_err(|e| e.to_string())?;
            "Claude 7 天额度已从当前浏览器登录态更新".to_string()
        }
        result => {
            storage::put_setting(&state.pool, SNAPSHOT, "")
                .await
                .map_err(|e| e.to_string())?;
            result
                .err()
                .unwrap_or_else(|| "此账户没有有效的 7 天额度".into())
        }
    };
    storage::put_setting(&state.pool, "claude_usage_message", &message)
        .await
        .map_err(|e| e.to_string())?;
    Ok(status(&state.pool).await)
}

fn parse_weekly(data: &Value, now: DateTime<Utc>) -> Option<f64> {
    let updated = DateTime::parse_from_rfc3339(data.get("updated_at")?.as_str()?).ok()?;
    let reset = DateTime::parse_from_rfc3339(data.get("weekly_resets_at")?.as_str()?).ok()?;
    if now.signed_duration_since(updated) > Duration::minutes(15)
        || updated > now + Duration::minutes(1)
        || reset <= now
    {
        return None;
    }
    let used = data.get("weekly_percent")?.as_f64()?;
    if !used.is_finite() || !(0.0..=100.0).contains(&used) {
        return None;
    }
    Some(100.0 - used)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remaining_and_expiry() {
        let now = Utc::now();
        let d = serde_json::json!({"weekly_percent":4,"updated_at":now.to_rfc3339(),"weekly_resets_at":(now+Duration::days(7)).to_rfc3339()});
        assert_eq!(parse_weekly(&d, now), Some(96.0));
        assert_eq!(parse_weekly(&d, now + Duration::minutes(16)), None);
        assert_eq!(
            parse_weekly(&serde_json::json!({"status":"login"}), now),
            None
        );
    }
}
