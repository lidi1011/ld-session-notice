use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
mod browser;
use crate::{app_state::AppState, storage};

const SNAPSHOT: &str = "claude_usage_snapshot";
const SOURCE: &str = "claude_usage_browser_source";

struct FetchError {
    message: String,
    invalidate: bool,
}
impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            invalidate: false,
        }
    }
}
impl From<&str> for FetchError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl FetchError {
    fn account(message: &str) -> Self {
        Self {
            message: message.into(),
            invalidate: true,
        }
    }
}
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

async fn saved_browser(pool: &SqlitePool) -> Result<browser::Browser, String> {
    match storage::get_setting(pool, SOURCE)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(raw) => {
            let source = serde_json::from_str(&raw)
                .map_err(|_| "额度浏览器资料无法读取，请重新点击在浏览器打开 Claude")?;
            browser::from_source(&source)
        }
        None => browser::selected(),
    }
}

pub async fn open_account(app: &AppHandle, _visible: bool) -> Result<(), String> {
    let _guard = REFRESH.lock().await;
    let state = app.state::<AppState>();
    let browser = browser::selected()?;
    browser.open()?;
    // This existing, explicit action is also the way to choose a different profile.
    let raw = serde_json::to_string(&browser.source).map_err(|e| e.to_string())?;
    if storage::get_setting(&state.pool, SOURCE)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(&raw)
    {
        storage::put_setting(&state.pool, SNAPSHOT, "")
            .await
            .map_err(|e| e.to_string())?;
    }
    storage::put_setting(&state.pool, SOURCE, &raw)
        .await
        .map_err(|e| e.to_string())?;
    storage::put_setting(
        &state.pool,
        "claude_usage_message",
        &format!("已选择 {}，登录 Claude 后刷新额度", browser.label()),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
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

async fn clear_changed_account(
    pool: &SqlitePool,
    organization: Option<&str>,
) -> Result<(), String> {
    let raw = storage::get_setting(pool, SNAPSHOT)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(previous) = raw.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        if previous.get("organization").and_then(Value::as_str) != organization {
            storage::put_setting(pool, SNAPSHOT, "")
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn fetch_usage(pool: &SqlitePool, browser: &browser::Browser) -> Result<Value, FetchError> {
    let cookies = browser.cookies().await?;
    if !cookies.contains_key("sessionKey") {
        return Err(FetchError::account(&format!(
            "{} 尚未登录 Claude，请在此资料中登录后刷新",
            browser.label()
        )));
    }
    // Never retain a previous account's quota when the browser changes organizations.
    clear_changed_account(pool, cookies.get("lastActiveOrg").map(String::as_str)).await?;
    let cookie = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    // No redirects: authentication cookies must only reach the intended Claude API origin.
    let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(20)).user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36").build().map_err(|_|"无法创建额度连接")?;
    async fn get(client: &reqwest::Client, cookie: &str, path: &str) -> Result<Value, FetchError> {
        let response = client
            .get(format!("https://claude.ai/api/{path}"))
            .header(reqwest::header::COOKIE, cookie)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| "Claude 额度请求失败，请检查网络后重试")?;
        match response.status().as_u16() {
            401 => {
                return Err(FetchError::account(
                    "浏览器 Claude 登录已失效，请登录后刷新",
                ))
            }
            403 => return Err("Claude 拒绝额度请求，请在浏览器打开额度页完成验证后重试".into()),
            429 => return Err("额度查询过于频繁，请稍后重试".into()),
            200 => {}
            _ => return Err("Claude 额度服务暂时不可用".into()),
        }
        response
            .json()
            .await
            .map_err(|_| FetchError::from("Claude 额度响应无法解析"))
    }
    let org = if let Some(org) = cookies.get("lastActiveOrg") {
        org.clone()
    } else {
        let data = get(&client, &cookie, "organizations").await?;
        data.as_array()
            .and_then(|a| a.first())
            .and_then(|v| v["uuid"].as_str())
            .ok_or_else(|| FetchError::account("账户没有可读取的组织"))?
            .to_string()
    };
    if org.is_empty() || !org.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(FetchError::account("账户组织标识无效"));
    }
    let data = get(&client, &cookie, &format!("organizations/{org}/usage")).await?;
    Ok(
        serde_json::json!({"weekly_percent":data["seven_day"]["utilization"],"weekly_resets_at":data["seven_day"]["resets_at"],"updated_at":Utc::now().to_rfc3339(),"organization":org}),
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
    let browser = saved_browser(&state.pool).await;
    let result = match &browser {
        Ok(browser) => fetch_usage(&state.pool, browser).await,
        Err(message) => Err(message.clone().into()),
    };
    record_refresh(&state.pool, browser.as_ref().ok(), result).await
}

async fn record_refresh(
    pool: &SqlitePool,
    browser: Option<&browser::Browser>,
    result: Result<Value, FetchError>,
) -> Result<UsageStatus, String> {
    let message = match result {
        Ok(value) if parse_weekly(&value, Utc::now()).is_some() => {
            if let Some(browser) = browser {
                let source = serde_json::to_string(&browser.source).map_err(|e| e.to_string())?;
                storage::put_setting(pool, SOURCE, &source)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            storage::put_setting(pool, SNAPSHOT, &value.to_string())
                .await
                .map_err(|e| e.to_string())?;
            let label = browser
                .map(|b| b.label())
                .unwrap_or_else(|| "浏览器".into());
            format!("Claude 7 天额度已从 {label} 更新")
        }
        Ok(_) => {
            storage::put_setting(pool, SNAPSHOT, "")
                .await
                .map_err(|e| e.to_string())?;
            "此账户没有有效的 7 天额度".into()
        }
        Err(error) => {
            if error.invalidate {
                storage::put_setting(pool, SNAPSHOT, "")
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if status(pool).await.remaining_percent.is_some() {
                format!(
                    "{}；暂时显示上次获取的额度（最多保留 15 分钟）",
                    error.message
                )
            } else {
                error.message
            }
        }
    };
    storage::put_setting(pool, "claude_usage_message", &message)
        .await
        .map_err(|e| e.to_string())?;
    Ok(status(pool).await)
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
    #[tokio::test]
    async fn transient_failure_retains_only_valid_quota_and_logout_clears_it() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)")
            .execute(&pool).await.unwrap();
        let now = Utc::now();
        let snapshot = serde_json::json!({"weekly_percent":4,"updated_at":now.to_rfc3339(),"weekly_resets_at":(now+Duration::days(7)).to_rfc3339(),"organization":"account-a"});
        let source = browser::Source {
            bundle: "com.google.Chrome".into(),
            profile: Some("Profile 1".into()),
        };
        let browser = browser::from_source(&source).unwrap();
        record_refresh(&pool, Some(&browser), Ok(snapshot.clone()))
            .await
            .unwrap();
        assert_eq!(saved_browser(&pool).await.unwrap().source, source);
        for failure in [
            "网络失败",
            "网站验证",
            "请求限流",
            "Cookie 数据库暂时不可读",
        ] {
            let result = record_refresh(&pool, None, Err(failure.into()))
                .await
                .unwrap();
            assert_eq!(result.remaining_percent, Some(96.0));
            assert_eq!(result.updated_at.unwrap().to_rfc3339(), now.to_rfc3339());
        }
        clear_changed_account(&pool, Some("account-a"))
            .await
            .unwrap();
        assert_eq!(status(&pool).await.remaining_percent, Some(96.0));
        clear_changed_account(&pool, Some("account-b"))
            .await
            .unwrap();
        assert_eq!(
            record_refresh(&pool, None, Err("网络失败".into()))
                .await
                .unwrap()
                .remaining_percent,
            None
        );
        record_refresh(&pool, None, Ok(snapshot.clone()))
            .await
            .unwrap();
        clear_changed_account(&pool, None).await.unwrap();
        assert_eq!(status(&pool).await.remaining_percent, None);
        let mut stale = snapshot.clone();
        stale["updated_at"] = (now - Duration::minutes(16)).to_rfc3339().into();
        storage::put_setting(&pool, SNAPSHOT, &stale.to_string())
            .await
            .unwrap();
        assert_eq!(
            record_refresh(&pool, None, Err("网络失败".into()))
                .await
                .unwrap()
                .remaining_percent,
            None
        );
        let mut zero = snapshot.clone();
        zero["weekly_percent"] = 100.into();
        assert_eq!(
            record_refresh(&pool, None, Ok(zero))
                .await
                .unwrap()
                .remaining_percent,
            Some(0.0)
        );
        assert_eq!(
            record_refresh(&pool, None, Err("网络失败".into()))
                .await
                .unwrap()
                .remaining_percent,
            Some(0.0)
        );
        assert_eq!(
            record_refresh(&pool, None, Err(FetchError::account("登录失效")))
                .await
                .unwrap()
                .remaining_percent,
            None
        );
        record_refresh(&pool, None, Ok(snapshot)).await.unwrap();
        assert_eq!(
            record_refresh(&pool, None, Ok(serde_json::json!({})))
                .await
                .unwrap()
                .remaining_percent,
            None
        );
    }
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
