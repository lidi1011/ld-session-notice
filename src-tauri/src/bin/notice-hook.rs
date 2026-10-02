use std::env;
use std::io::{self, Read};
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let token = match read_token() {
        Ok(token) => token,
        Err(error) => {
            eprintln!("Notice hook token unavailable: {error}");
            print_continue(true, None);
            return;
        }
    };

    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("Notice hook stdin failed: {error}");
        print_continue(true, None);
        return;
    }

    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({ "raw": input }));
    let critical = is_critical_payload(&payload);
    let timeout = if critical { 30 } else { 2 };
    let client = match Client::builder()
        .timeout(Duration::from_secs(timeout))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Notice hook client failed: {error}");
            print_continue(!critical, Some("Notice hook client failed"));
            return;
        }
    };

    let response = client
        .post("http://127.0.0.1:3746/api/webhook/codex")
        .header("X-Notice-Token", token)
        .json(&payload)
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() && !body.trim().is_empty() {
                println!("{body}");
            } else if critical {
                print_continue(
                    false,
                    Some("Notice rejected or failed to process critical command"),
                );
            } else {
                print_continue(true, None);
            }
        }
        Err(error) => {
            eprintln!("Notice hook request failed: {error}");
            print_continue(!critical, Some("Notice unavailable for critical command"));
        }
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

fn is_critical_payload(payload: &Value) -> bool {
    let command = payload
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .or_else(|| {
            payload
                .get("toolInput")
                .and_then(|input| input.get("command"))
        })
        .or_else(|| payload.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    command.contains("rm -rf /")
        || command.contains("sudo rm")
        || command.contains("diskutil erase")
        || command.contains("drop table")
        || command.contains("truncate table")
}

fn print_continue(should_continue: bool, reason: Option<&str>) {
    let mut value = json!({
        "continue": should_continue,
        "suppressOutput": true
    });
    if let Some(reason) = reason {
        value["stopReason"] = json!(reason);
        value["suppressOutput"] = json!(false);
    }
    println!("{value}");
}
