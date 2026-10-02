use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::storage;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    pub token_key: String,
    pub webhook_key: String,
    pub sign_secret_key: String,
}

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub data_dir: PathBuf,
    pub token: Arc<String>,
    pub config: Arc<RwLock<RuntimeConfig>>,
}

impl AppState {
    pub async fn initialize() -> anyhow::Result<Self> {
        let data_dir = storage::app_data_dir()?;
        let pool = storage::initialize(&data_dir).await?;
        let config = RuntimeConfig {
            token_key: "notice-local-token".to_string(),
            webhook_key: "notice-feishu-webhook".to_string(),
            sign_secret_key: "notice-feishu-sign-secret".to_string(),
        };
        let token = match storage::get_setting(&pool, "notice_local_token").await? {
            Some(value) => value,
            None => {
                let value = Uuid::new_v4().to_string();
                storage::put_setting(&pool, "notice_local_token", &value).await?;
                value
            }
        };
        let hook_dir = data_dir.join("hooks");
        tokio::fs::create_dir_all(&hook_dir).await?;
        tokio::fs::write(hook_dir.join("token"), token.as_str()).await?;
        Ok(Self {
            pool,
            data_dir,
            token: Arc::new(token),
            config: Arc::new(RwLock::new(config)),
        })
    }
}
