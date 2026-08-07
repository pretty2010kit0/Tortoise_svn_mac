//! 应用设置（JSON 持久化，$HOME/.config/svn-desktop-tool/settings.json）
//! 字段：
//! - external_diff —— 外部 diff 工具可执行路径（默认 /usr/bin/opendiff）
//! - trusted_cert_urls —— 永久信任的证书站点列表（协议+host[:port]，如 https://svn.example.internal）

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const DEFAULT_DIFF: &str = "/usr/bin/opendiff";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    external_diff: String,
    trusted_cert_urls: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            external_diff: DEFAULT_DIFF.to_string(),
            trusted_cert_urls: Vec::new(),
        }
    }
}

static LOCK: Mutex<()> = Mutex::new(());

fn settings_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("svn-desktop-tool")
        .join("settings.json")
}

fn load() -> Settings {
    let raw = match fs::read_to_string(settings_file()) {
        Ok(r) => r,
        Err(_) => return Settings::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(s: &Settings) -> Result<(), String> {
    let path = settings_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("无法创建设置目录 {}: {}", dir.display(), e))?;
    }
    let raw = serde_json::to_string_pretty(s).map_err(|e| format!("设置序列化失败: {}", e))?;
    fs::write(&path, raw).map_err(|e| format!("设置写入失败: {}", e))?;
    Ok(())
}

/// 读取外部 diff 工具路径
#[tauri::command]
pub fn get_external_diff() -> String {
    load().external_diff
}

/// 设置外部 diff 工具路径（留空恢复默认 opendiff）
#[tauri::command]
pub fn set_external_diff(cmd: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = load();
    let c = cmd.trim().to_string();
    s.external_diff = if c.is_empty() { DEFAULT_DIFF.to_string() } else { c };
    save(&s)
}

/// 内部读取（wc_diff_external 用）
pub fn external_diff_cmd() -> String {
    load().external_diff
}

/// 永久信任的证书站点列表
#[tauri::command]
pub fn cert_trust_list() -> Vec<String> {
    load().trusted_cert_urls
}

/// 添加永久信任站点（按 协议+host[:port] 前缀匹配）
#[tauri::command]
pub fn cert_trust_add(url: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = load();
    let site = site_of(&url);
    if site.is_empty() {
        return Err("无法解析站点（需要完整 URL，如 https://host:port/svn/…）".into());
    }
    if !s.trusted_cert_urls.iter().any(|x| x == &site) {
        s.trusted_cert_urls.push(site);
        save(&s)?;
    }
    Ok(())
}

/// 移除永久信任站点
#[tauri::command]
pub fn cert_trust_remove(url: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = load();
    let site = site_of(&url);
    s.trusted_cert_urls.retain(|x| x != &site);
    save(&s)
}

/// URL → 站点（协议 + host[:port]）
fn site_of(url: &str) -> String {
    let url = url.trim();
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => return String::new(),
    };
    let scheme = &url[..url.find("://").unwrap_or(0)];
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or("").to_string();
    if hostport.is_empty() {
        String::new()
    } else {
        format!("{scheme}://{hostport}")
    }
}

/// 判断 URL 是否命中已信任站点（前缀匹配）
pub fn is_cert_trusted(url: &str) -> bool {
    let s = load();
    s.trusted_cert_urls.iter().any(|site| url.starts_with(site))
}
