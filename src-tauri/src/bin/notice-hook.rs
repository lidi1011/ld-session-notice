use std::env;
use std::io::{self, Read};
use std::time::Duration;

use reqwest::Client;
use notice_lib::rules::risk::{classify_command, RiskLevel};
use serde_json::{json, Value};

/// Same contract as hooks/codex-hook.sh: Codex blocks a PreToolUse call only on
/// exit code 2 with the reason on stderr, so allowed calls print nothing.
#[tokio::main]
async fn main() {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("Notice hook stdin failed: {error}");
        return;
    }
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({ "raw": input }));
    let event = payload
        .get("hook_event_name")
        .or_else(|| payload.get("hookEventName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let pre_tool_use = event == "PreToolUse";
    let critical = pre_tool_use && is_critical_payload(&payload);

    let token = match read_token() {
        Ok(token) => token,
        Err(error) => {
            if critical {
                deny(&format!("Notice token unavailable: {error}"));
            }
            return;
        }
    };
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(if pre_tool_use { 28 } else { 5 }))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            if critical {
                deny(&format!("Notice hook client failed: {error}"));
            }
            return;
        }
    };

    let response = client
        .post("http://127.0.0.1:3746/api/webhook/codex")
        .header("X-Notice-Token", token)
        .json(&payload)
        .send()
        .await;
    if !pre_tool_use {
        return;
    }
    match response {
        Ok(resp) => {
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if !status.is_success() {
                if critical {
                    deny(&format!("Notice answered HTTP {status}"));
                }
            } else if body.get("continue").and_then(Value::as_bool) == Some(false) {
                let reason = body
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("rejected in Notice");
                deny(reason);
            }
        }
        Err(error) if error.is_connect() => {
            if critical {
                deny("Notice is not running, so this destructive command cannot be approved");
            }
        }
        Err(error) => deny(&format!("Notice did not answer in time: {error}")),
    }
}

fn deny(reason: &str) -> ! {
    eprintln!("Notice blocked this command: {reason}");
    std::process::exit(2);
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

fn is_critical_payload(payload: &Value) -> bool {
    let command = payload
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .or_else(|| {
            payload
                .get("toolInput")
                .and_then(|input| input.get("command"))
        })
        .or_else(|| payload.get("command"));
    let text = match command {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => return false,
    };
    classify_command(&text).level == RiskLevel::Critical
}
