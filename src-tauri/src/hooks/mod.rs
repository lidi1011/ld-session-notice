pub mod claude;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::domain::{HookPreview, HookStatus};

const START_MARKER: &str = "# >>> Notice managed hooks >>>";
const END_MARKER: &str = "# <<< Notice managed hooks <<<";
const EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

pub fn codex_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
}

pub fn preview(data_dir: PathBuf) -> anyhow::Result<HookPreview> {
    let config_path = codex_config_path();
    let will_create_config = !config_path.exists();
    let command = hook_command(data_dir);
    Ok(HookPreview {
        config_path: config_path.display().to_string(),
        will_create_config,
        preview: managed_block(&command),
    })
}

pub fn status() -> anyhow::Result<HookStatus> {
    let config_path = codex_config_path();
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let installed = managed_block_complete(&content);
    let has_managed_block = content.contains(START_MARKER) && content.contains(END_MARKER);
    Ok(HookStatus {
        installed,
        config_path: config_path.display().to_string(),
        managed_block_hash: has_managed_block.then(|| hash_notice_block(&content)),
        backup_path: None,
        message: if installed {
            "Notice hooks installed".to_string()
        } else if has_managed_block {
            "Notice hooks need reinstall".to_string()
        } else {
            "Notice hooks not installed".to_string()
        },
    })
}

pub fn install(data_dir: PathBuf) -> anyhow::Result<HookStatus> {
    let config_path = codex_config_path();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let original = fs::read_to_string(&config_path).unwrap_or_default();
    let backup_path = backup_config(&data_dir, &original)?;
    let cleaned = remove_managed_block(&original);
    write_wrapper_script(&data_dir)?;
    let command = hook_command(data_dir);
    let next = append_hooks(&cleaned, &command)?;
    fs::write(&config_path, next)?;
    let mut current = status()?;
    current.backup_path = Some(backup_path.display().to_string());
    current.message = "Notice hooks installed with backup".to_string();
    Ok(current)
}

pub fn uninstall(data_dir: PathBuf) -> anyhow::Result<HookStatus> {
    let config_path = codex_config_path();
    let original = fs::read_to_string(&config_path).unwrap_or_default();
    let backup_path = backup_config(&data_dir, &original)?;
    let cleaned = remove_managed_block(&original);
    fs::write(&config_path, cleaned)?;
    let mut current = status()?;
    current.backup_path = Some(backup_path.display().to_string());
    current.message = "Notice hooks removed".to_string();
    Ok(current)
}

fn hook_command(data_dir: PathBuf) -> String {
    let script = data_dir.join("hooks").join("notice-hook.sh");
    format!("/bin/sh {}", shell_escape(&script.display().to_string()))
}

fn managed_block(command: &str) -> String {
    let mut block = String::new();
    block.push_str(START_MARKER);
    block.push('\n');
    for event in EVENTS {
        block.push_str(&format!("\n[[hooks.{event}]]\n"));
        block.push_str(&format!("[[hooks.{event}.hooks]]\n"));
        block.push_str("type = \"command\"\n");
        block.push_str(&format!("command = \"{}\"\n", command.replace('"', "\\\"")));
        block.push_str("timeout = 30\n");
        block.push_str("statusMessage = \"Notice processing hook\"\n");
    }
    block.push_str(END_MARKER);
    block.push('\n');
    block
}

fn append_hooks(existing: &str, command: &str) -> anyhow::Result<String> {
    let block = managed_block(command);
    let cleaned = remove_managed_block(existing);
    Ok(format!("{}\n{}", cleaned.trim_end(), block))
}

fn remove_managed_block(input: &str) -> String {
    let mut output = String::new();
    let mut skipping = false;
    for line in input.lines() {
        if line.trim() == START_MARKER {
            skipping = true;
            continue;
        }
        if line.trim() == END_MARKER {
            skipping = false;
            continue;
        }
        if !skipping {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

fn managed_block_complete(content: &str) -> bool {
    content.contains(START_MARKER)
        && content.contains(END_MARKER)
        && EVENTS
            .iter()
            .all(|event| content.contains(&format!("[[hooks.{event}]]")))
}

fn backup_config(data_dir: &PathBuf, content: &str) -> anyhow::Result<PathBuf> {
    let dir = data_dir.join("hooks").join("backups");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("config-{}.toml", Utc::now().format("%Y%m%d%H%M%S")));
    fs::write(&path, content)?;
    Ok(path)
}

fn write_wrapper_script(data_dir: &PathBuf) -> anyhow::Result<PathBuf> {
    let dir = data_dir.join("hooks");
    fs::create_dir_all(&dir)?;
    let token_file = dir.join("token");
    let script = dir.join("notice-hook.sh");
    let body = format!(
        r#"#!/bin/sh
TOKEN_FILE={token_file}
PAYLOAD="$(cat)"
TIMEOUT=2
if printf "%s" "$PAYLOAD" | /usr/bin/grep -Eiq 'rm -rf /|sudo rm|diskutil erase|drop table|truncate table'; then
  TIMEOUT=30
fi
if [ ! -f "$TOKEN_FILE" ]; then
  printf '{{"continue":true,"suppressOutput":true}}\n'
  exit 0
fi
TOKEN="$(cat "$TOKEN_FILE")"
RESP="$(printf "%s" "$PAYLOAD" | /usr/bin/curl -sS --max-time "$TIMEOUT" \
  -H "Content-Type: application/json" \
  -H "X-Notice-Token: $TOKEN" \
  --data-binary @- \
  http://127.0.0.1:3746/api/webhook/codex)"
STATUS=$?
if [ "$STATUS" -eq 0 ] && [ -n "$RESP" ]; then
  printf "%s\n" "$RESP"
elif [ "$TIMEOUT" -eq 30 ]; then
  printf '{{"continue":false,"stopReason":"Notice unavailable for critical command","suppressOutput":false}}\n'
else
  printf '{{"continue":true,"suppressOutput":true}}\n'
fi
"#,
        token_file = shell_escape(&token_file.display().to_string())
    );
    fs::write(&script, body)?;
    let mut permissions = fs::metadata(&script)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions)?;
    Ok(script)
}

fn hash_notice_block(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{
        append_hooks, hook_command, managed_block_complete, remove_managed_block, START_MARKER,
    };
    use std::path::PathBuf;

    #[test]
    fn install_preserves_existing_hooks() {
        let existing = "[hooks]\n\n[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"echo existing\"\n";
        let next = append_hooks(existing, "notice-hook").unwrap();
        assert!(next.contains("echo existing"));
        assert!(next.contains(START_MARKER));
        assert!(next.contains("notice-hook"));
        assert!(next.contains("[[hooks.UserPromptSubmit]]"));
    }

    #[test]
    fn uninstall_removes_only_managed_block() {
        let input = "before\n# >>> Notice managed hooks >>>\nmanaged\n# <<< Notice managed hooks <<<\nafter\n";
        let cleaned = remove_managed_block(input);
        assert!(cleaned.contains("before"));
        assert!(cleaned.contains("after"));
        assert!(!cleaned.contains("managed"));
    }

    #[test]
    fn managed_block_is_incomplete_without_user_prompt_submit() {
        let input = "# >>> Notice managed hooks >>>\n[[hooks.SessionStart]]\n[[hooks.PermissionRequest]]\n[[hooks.PreToolUse]]\n[[hooks.PostToolUse]]\n[[hooks.Stop]]\n# <<< Notice managed hooks <<<\n";

        assert!(!managed_block_complete(input));
    }

    #[test]
    fn command_runs_wrapper_through_shell_for_paths_with_spaces() {
        let command = hook_command(PathBuf::from(
            "/Users/di/Library/Application Support/Notice",
        ));
        assert_eq!(
            command,
            "/bin/sh '/Users/di/Library/Application Support/Notice/hooks/notice-hook.sh'"
        );
    }
}
