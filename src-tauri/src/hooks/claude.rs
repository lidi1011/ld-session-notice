use std::{fs, path::{Path, PathBuf}, os::unix::fs::OpenOptionsExt, io::Write};
use serde_json::{json, Value};
use crate::domain::{HookPreview, HookStatus};

const EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit", "PermissionRequest", "Notification", "PreToolUse", "PostToolUse", "PostToolUseFailure", "Elicitation", "ElicitationResult", "Stop", "StopFailure", "SessionEnd"];
fn config_path() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude")).join("settings.json")
}
fn command(data: &Path) -> String { format!("/bin/sh {}", super::shell_escape(&data.join("hooks/notice-claude-hook.sh").display().to_string())) }
fn read(path: &Path) -> anyhow::Result<String> {
    match fs::read_to_string(path) { Ok(s) => Ok(s), Err(e) if e.kind()==std::io::ErrorKind::NotFound => Ok("{}".into()), Err(e)=>Err(e.into()) }
}
fn merge(original: &str, command: &str, install: bool) -> anyhow::Result<Value> {
    let mut config: Value = serde_json::from_str(original)?;
    let object = config.as_object_mut().ok_or_else(|| anyhow::anyhow!("Claude settings must be an object"))?;
    if !install && !object.contains_key("hooks") { return Ok(config); }
    let hooks = object.entry("hooks").or_insert(json!({})).as_object_mut().ok_or_else(|| anyhow::anyhow!("hooks must be an object"))?;
    for event in EVENTS {
        let mut kept = Vec::new();
        if let Some(groups) = hooks.get(*event) {
            for group in groups.as_array().ok_or_else(|| anyhow::anyhow!("hook groups must be arrays"))? {
                let mut group = group.clone();
                let entries = group.get_mut("hooks").and_then(Value::as_array_mut).ok_or_else(|| anyhow::anyhow!("hook entries must be arrays"))?;
                let was_empty = entries.is_empty();
                entries.retain(|entry| entry.get("command").and_then(Value::as_str)!=Some(command));
                if !entries.is_empty() || was_empty { kept.push(group); }
            }
        }
        if install { kept.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]})); }
        if kept.is_empty() { hooks.remove(*event); } else { hooks.insert(event.to_string(), json!(kept)); }
    }
    Ok(config)
}
fn atomic_write(path: &Path, body: &str) -> anyhow::Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("missing parent directory"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".notice-{}", uuid::Uuid::new_v4()));
    let result = (|| { let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?; file.write_all(body.as_bytes())?; file.sync_all()?; fs::rename(&temp,path) })();
    if temp.exists() { let _ = fs::remove_file(temp); }
    Ok(result?)
}
pub fn preview(data: &Path) -> anyhow::Result<HookPreview> {
    let path = config_path();
    merge(&read(&path)?,&command(data),true)?;
    Ok(HookPreview { config_path:path.display().to_string(), will_create_config:!path.exists(), preview:serde_json::to_string_pretty(&merge("{}",&command(data),true)?)? })
}
pub fn status(data: &Path) -> anyhow::Result<HookStatus> {
    status_at(data,&config_path())
}
fn status_at(data: &Path, path: &Path) -> anyhow::Result<HookStatus> {
    let config: Value=serde_json::from_str(&read(path)?)?; let cmd=command(data);
    let installed = EVENTS.iter().all(|event| config.get("hooks").and_then(|h| h.get(*event)).and_then(Value::as_array).map(|groups| groups.iter().filter_map(|g|g.get("hooks").and_then(Value::as_array)).flatten().filter(|h|h.get("command").and_then(Value::as_str)==Some(cmd.as_str())).count()==1).unwrap_or(false)) && data.join("hooks/notice-claude-hook.sh").is_file();
    Ok(HookStatus {installed,config_path:path.display().to_string(),managed_block_hash:None,backup_path:None,message:if installed {"Notice hooks installed"} else {"Notice hooks not installed"}.into()})
}
pub fn update(data: &Path, install: bool) -> anyhow::Result<HookStatus> {
    update_at(data,&config_path(),install)
}
fn update_at(data: &Path, path: &Path, install: bool) -> anyhow::Result<HookStatus> {
    let original=read(path)?; let next=merge(&original,&command(data),install)?;
    let backup=data.join(format!("hooks/backups/claude-settings-{}.json",uuid::Uuid::new_v4()));
    atomic_write(&backup,&original)?;
    if install {
        let token=super::shell_escape(&data.join("hooks/token").display().to_string());
        atomic_write(&data.join("hooks/notice-claude-hook.sh"),&format!(r#"#!/bin/sh
TOKEN_FILE={token}
if [ -f "$TOKEN_FILE" ]; then
  /usr/bin/curl --silent --max-time 1 --output /dev/null -H 'Content-Type: application/json' -H "X-Notice-Token: $(cat "$TOKEN_FILE")" --data-binary @- http://127.0.0.1:3746/api/webhook/claude-code 2>/dev/null
fi
exit 0
"#))?;
    }
    // Refuse to overwrite edits made while preparing the merge.
    anyhow::ensure!(read(&path)?==original,"Claude settings changed; retry the operation");
    atomic_write(&path,&(serde_json::to_string_pretty(&next)?+"\n"))?;
    let mut result=status_at(data,path)?; result.backup_path=Some(backup.display().to_string()); Ok(result)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn install_and_uninstall_work_without_python() {
        let temp=tempfile::tempdir().unwrap(); let data=temp.path().join("Notice Data"); let path=temp.path().join("settings.json");
        let original=json!({"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"existing"}]}]},"env":{"KEEP":"yes"}});
        fs::write(&path,original.to_string()).unwrap();
        assert!(update_at(&data,&path,true).unwrap().installed);
        let installed=fs::read_to_string(&path).unwrap();
        assert!(update_at(&data,&path,true).unwrap().installed);
        assert_eq!(installed,fs::read_to_string(&path).unwrap());
        assert!(!update_at(&data,&path,false).unwrap().installed);
        assert_eq!(original,serde_json::from_str::<Value>(&fs::read_to_string(&path).unwrap()).unwrap());
    }
    #[test] fn preserves_hooks_and_is_idempotent() {
        let original=json!({"env":{"KEEP":"value"},"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"other"}]}]}});
        let once=merge(&original.to_string(),"notice",true).unwrap();
        assert_eq!(once,merge(&once.to_string(),"notice",true).unwrap());
        assert_eq!(original,merge(&once.to_string(),"notice",false).unwrap());
        assert!(merge("{\"hooks\":false}","notice",true).is_err());
    }
}
