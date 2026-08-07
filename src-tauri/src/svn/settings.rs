//! 应用设置（JSON 持久化，$HOME/.config/svn-desktop-tool/settings.json）
//! 当前字段：external_diff —— 外部 diff 工具可执行路径（默认 /usr/bin/opendiff）

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const DEFAULT_DIFF: &str = "/usr/bin/opendiff";

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    external_diff: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            external_diff: DEFAULT_DIFF.to_string(),
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
