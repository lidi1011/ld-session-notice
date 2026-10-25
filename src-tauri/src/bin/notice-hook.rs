use std::env;
use std::io::{self, Read};
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

/// Same contract as hooks/codex-hook.sh: forward the event to Notice and let
/// Codex carry on. Notice records commands; approving them is left to Codex.
/// Nothing is printed, because Codex rejects the usual fields on PreToolUse.
#[tokio::main]
async fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(token) = read_token() else {
        return;
    };
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({ "raw": input }));
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(3))
        .build()
    else {
        return;
    };
    if let Err(error) = client
        .post("http://127.0.0.1:3746/api/webhook/codex")
        .header("X-Notice-Token", token)
        .json(&payload)
        .send()
        .await
    {
        eprintln!("Notice hook request failed: {error}");
    }
}

fn read_token() -> anyhow::Result<String> {
    let args = env::args().collect::<Vec<_>>();
    if let Some(index) = args.iter().position(|arg| arg == "--token-file") {
        if let Some(path) = args.get(index + 1) {
            return Ok(std::fs::read_to_string(path)?.trim().to_string());
        }
    }
    if let Ok(value) = env::var("NOTICE_TOKEN") {
        return Ok(value);
    }
    anyhow::bail!("missing --token-file or NOTICE_TOKEN")
}
