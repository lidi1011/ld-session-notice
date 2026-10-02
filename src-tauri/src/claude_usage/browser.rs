//! Read only claude.ai cookies from the selected browser; never persist credentials.
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, Connection, Row};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

pub struct Browser {
    pub name: &'static str,
    pub bundle: &'static str,
    root: PathBuf,
    service: Option<&'static str>,
}
pub fn selected() -> Result<Browser, String> {
    let home = dirs::home_dir().ok_or("无法找到用户目录")?;
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(home.join(
            "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist",
        ))
        .output()
        .map_err(|_| "无法读取默认浏览器")?;
    let data: Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let bundle = data["LSHandlers"]
        .as_array()
        .and_then(|a| a.iter().find(|h| h["LSHandlerURLScheme"] == "https"))
        .and_then(|h| h["LSHandlerRoleAll"].as_str())
        .unwrap_or("com.apple.Safari");
    let (name, bundle, path, service) = match bundle.to_ascii_lowercase().as_str() {
        "com.google.chrome" => (
            "Chrome",
            "com.google.Chrome",
            "Library/Application Support/Google/Chrome",
            Some("Chrome Safe Storage"),
        ),
        "company.thebrowser.browser" => (
            "Arc",
            "company.thebrowser.Browser",
            "Library/Application Support/Arc/User Data",
            Some("Arc Safe Storage"),
        ),
        "com.brave.browser" => (
            "Brave",
            "com.brave.Browser",
            "Library/Application Support/BraveSoftware/Brave-Browser",
            Some("Brave Safe Storage"),
        ),
        "com.apple.safari" => (
            "Safari",
            "com.apple.Safari",
            "Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies",
            None,
        ),
        _ => return Err("默认浏览器暂不支持额度读取；支持 Chrome、Arc、Brave、Safari".into()),
    };
    Ok(Browser {
        name,
        bundle,
        root: home.join(path),
        service,
    })
}
fn profile_name(state: &Value) -> String {
    let name = state["profile"]["last_used"].as_str().unwrap_or("Default");
    if name == "Default"
        || name
            .strip_prefix("Profile ")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
    {
        name.into()
    } else {
        "Default".into()
    }
}
fn allowed(name: &str) -> bool {
    matches!(
        name,
        "sessionKey" | "lastActiveOrg" | "cf_clearance" | "__cf_bm"
    )
}
fn cookie_domain(domain: &str) -> bool {
    domain == "claude.ai" || domain == ".claude.ai"
}
impl Browser {
    pub fn open(&self) -> Result<(), String> {
        let status = Command::new("/usr/bin/open")
            .args(["-b", self.bundle, "https://claude.ai/settings/usage"])
            .status()
            .map_err(|_| "无法打开现有浏览器")?;
        if status.success() {
            Ok(())
        } else {
            Err("无法打开现有浏览器".into())
        }
    }
    pub async fn cookies(&self) -> Result<BTreeMap<String, String>, String> {
        let Some(service) = self.service else {
            let bytes = std::fs::read(&self.root)
                .map_err(|_| "无法读取 Safari 登录态，请在系统设置中允许 Notice 完全磁盘访问")?;
            return safari(&bytes).ok_or("Safari Cookie 格式无法读取".into());
        };
        let state: Value = std::fs::read(self.root.join("Local State"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let profile = self.root.join(profile_name(&state));
        let mut path = profile.join("Cookies");
        if !path.exists() {
            path = profile.join("Network/Cookies");
        }
        // Read the live database in SQLite read-only mode so WAL updates are included.
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .busy_timeout(std::time::Duration::from_secs(2));
        let mut db=sqlx::SqliteConnection::connect_with(&options).await.map_err(|_|"无法读取浏览器登录态，请检查当前个人资料；如系统拒绝访问，请给 Notice 完全磁盘访问权限")?;
        let version: i64 =
            sqlx::query_scalar::<_, String>("SELECT value FROM meta WHERE key='version'")
                .fetch_optional(&mut db)
                .await
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        let rows=sqlx::query("SELECT host_key,name,value,encrypted_value FROM cookies WHERE host_key IN ('claude.ai','.claude.ai') AND name IN ('sessionKey','lastActiveOrg','cf_clearance','__cf_bm') AND (expires_utc=0 OR expires_utc > (strftime('%s','now')+11644473600)*1000000)").fetch_all(&mut db).await.map_err(|_|"浏览器 Cookie 格式无法读取")?;
        if rows.is_empty() {
            return Err("当前浏览器个人资料没有 Claude 登录态，请先在浏览器登录 Claude".into());
        }
        let key = tauri::async_runtime::spawn_blocking(move || {
            let output = Command::new("/usr/bin/security")
                .args(["find-generic-password", "-w", "-s", service])
                .output()
                .map_err(|_| "无法访问浏览器安全存储")?;
            if !output.status.success() {
                return Err("浏览器安全存储访问被拒绝，请允许钥匙串访问".to_string());
            }
            let mut password = output.stdout;
            while password.last().is_some_and(|b| b.is_ascii_whitespace()) {
                password.pop();
            }
            let key = derive(&password);
            password.fill(0);
            key
        })
        .await
        .map_err(|_| "读取浏览器安全存储失败")??;
        let mut result = BTreeMap::new();
        for row in rows {
            let name: String = row.get("name");
            let domain: String = row.get("host_key");
            let plain: String = row.get("value");
            let encrypted: Vec<u8> = row.get("encrypted_value");
            let value = if encrypted.is_empty() {
                Some(plain)
            } else {
                decrypt(&encrypted, &key, &domain, version)
            };
            if let Some(value) = value.filter(|v| !v.is_empty() && !v.contains(['\r', '\n', ';'])) {
                result.insert(name, value);
            }
        }
        Ok(result)
    }
}
#[link(name = "System")]
extern "C" {
    fn CCKeyDerivationPBKDF(
        algorithm: u32,
        password: *const u8,
        password_len: usize,
        salt: *const u8,
        salt_len: usize,
        prf: u32,
        rounds: u32,
        key: *mut u8,
        key_len: usize,
    ) -> i32;
    fn CCCrypt(
        op: u32,
        algorithm: u32,
        options: u32,
        key: *const u8,
        key_len: usize,
        iv: *const u8,
        input: *const u8,
        input_len: usize,
        output: *mut u8,
        output_len: usize,
        written: *mut usize,
    ) -> i32;
}
fn derive(password: &[u8]) -> Result<[u8; 16], String> {
    let mut key = [0; 16];
    let salt = b"saltysalt";
    let rc = unsafe {
        CCKeyDerivationPBKDF(
            2,
            password.as_ptr(),
            password.len(),
            salt.as_ptr(),
            salt.len(),
            1,
            1003,
            key.as_mut_ptr(),
            16,
        )
    };
    if rc == 0 {
        Ok(key)
    } else {
        Err("浏览器密钥解析失败".into())
    }
}
fn decrypt(blob: &[u8], key: &[u8; 16], domain: &str, version: i64) -> Option<String> {
    if !blob.starts_with(b"v10") {
        return None;
    }
    let input = &blob[3..];
    let mut output = vec![0; input.len() + 16];
    let mut written = 0;
    let rc = unsafe {
        CCCrypt(
            1,
            0,
            1,
            key.as_ptr(),
            16,
            [b' '; 16].as_ptr(),
            input.as_ptr(),
            input.len(),
            output.as_mut_ptr(),
            output.len(),
            &mut written,
        )
    };
    if rc != 0 {
        return None;
    }
    output.truncate(written);
    let payload = if version >= 24 {
        if output.get(..32)? != Sha256::digest(domain.as_bytes()).as_slice() {
            return None;
        }
        &output[32..]
    } else {
        &output[..]
    };
    let value = String::from_utf8(payload.to_vec()).ok();
    output.fill(0);
    value
}
fn uint(data: &[u8], offset: usize, be: bool) -> Option<usize> {
    let a = data.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if be {
        u32::from_be_bytes(a)
    } else {
        u32::from_le_bytes(a)
    } as usize)
}
fn cstr(data: &[u8], offset: usize) -> Option<&str> {
    let b = data.get(offset..)?;
    std::str::from_utf8(b.get(..b.iter().position(|b| *b == 0)?)?).ok()
}
fn safari(data: &[u8]) -> Option<BTreeMap<String, String>> {
    if data.get(..4)? != b"cook" {
        return None;
    }
    let count = uint(data, 4, true)?;
    let mut offset = 8usize.checked_add(count.checked_mul(4)?)?;
    if offset > data.len() {
        return None;
    }
    let mut cookies = BTreeMap::new();
    for i in 0..count {
        let size = uint(data, 8 + i * 4, true)?;
        let page = data.get(offset..offset.checked_add(size)?)?;
        offset += size;
        let records = uint(page, 4, false)?;
        if records > page.len() / 4 {
            return None;
        }
        for n in 0..records {
            let start = uint(page, 8 + n * 4, false)?;
            let size = uint(page, start, false)?;
            let record = page.get(start..start.checked_add(size)?)?;
            let domain = cstr(record, uint(record, 16, false)?)?;
            let name = cstr(record, uint(record, 20, false)?)?;
            if cookie_domain(domain) && allowed(name) {
                let value = cstr(record, uint(record, 28, false)?)?;
                if !value.contains(['\r', '\n', ';']) {
                    cookies.insert(name.into(), value.into());
                }
            }
        }
    }
    Some(cookies)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_profiles_and_domains() {
        assert_eq!(
            profile_name(&serde_json::json!({"profile":{"last_used":"../Other"}})),
            "Default"
        );
        assert_eq!(
            profile_name(&serde_json::json!({"profile":{"last_used":"Profile 1"}})),
            "Profile 1"
        );
        assert!(!cookie_domain("evilclaude.ai"));
        assert!(cookie_domain(".claude.ai"));
        assert!(safari(b"cook\xff\xff\xff\xff").is_none());
    }
    #[test]
    fn decrypts_domain_bound_cookie() {
        let key = derive(b"test-only").unwrap();
        let domain = ".claude.ai";
        let mut plain = Sha256::digest(domain.as_bytes()).to_vec();
        plain.extend_from_slice(b"fixture-value");
        let mut encrypted = vec![0; plain.len() + 16];
        let mut n = 0;
        assert_eq!(
            unsafe {
                CCCrypt(
                    0,
                    0,
                    1,
                    key.as_ptr(),
                    16,
                    [b' '; 16].as_ptr(),
                    plain.as_ptr(),
                    plain.len(),
                    encrypted.as_mut_ptr(),
                    encrypted.len(),
                    &mut n,
                )
            },
            0
        );
        encrypted.truncate(n);
        let mut blob = b"v10".to_vec();
        blob.extend(encrypted);
        assert_eq!(
            decrypt(&blob, &key, domain, 24).as_deref(),
            Some("fixture-value")
        );
        assert_eq!(decrypt(&blob, &key, "other", 24), None);
    }
}
