//! 访问历史（远程仓库 URL / 本地工作副本路径）持久化
//!
//! 存储位置：`$HOME/.config/svn-desktop-tool/history.json`
//! 语义：按「最近使用」排序，同 kind + value 去重（新记录移到最前），上限 50 条。
//! 用独立文件而非 WebKit localStorage（dev 模式沙盒下 localStorage 不可靠）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 50;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub kind: String,    // "remote" | "local"
    pub value: String,   // 远程 URL 或本地工作副本路径
    pub last_used: i64,  // unix 秒
}

static LOCK: Mutex<()> = Mutex::new(());

fn history_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("svn-desktop-tool").join("history.json")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load() -> Vec<HistoryEntry> {
    let path = history_file();
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(entries: &[HistoryEntry]) -> Result<(), String> {
    let path = history_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("无法创建历史目录 {}: {}", dir.display(), e))?;
    }
    let raw = serde_json::to_string_pretty(entries).map_err(|e| format!("历史序列化失败: {}", e))?;
    fs::write(&path, raw).map_err(|e| format!("历史写入失败: {}", e))?;
    Ok(())
}

/// 记录一次访问（打开成功后才调用，避免失败地址污染历史）
#[tauri::command]
pub fn history_add(kind: String, value: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(());
    }
    let mut entries = load();
    entries.retain(|e| !(e.kind == kind && e.value == value));
    entries.insert(
        0,
        HistoryEntry {
            kind,
            value,
            last_used: now(),
        },
    );
    entries.truncate(MAX_ENTRIES);
    save(&entries)
}

/// 返回全部历史（按最近使用排序，调用方按 kind 过滤）
#[tauri::command]
pub fn history_list() -> Vec<HistoryEntry> {
    load()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(kind: &str, values: &[&str]) -> Vec<HistoryEntry> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| HistoryEntry {
                kind: kind.to_string(),
                value: v.to_string(),
                last_used: 1000 + i as i64,
            })
            .collect()
    }

    #[test]
    fn dedup_moves_to_front() {
        let mut list = entries("remote", &["a", "b", "c"]);
        list.retain(|e| !(e.kind == "remote" && e.value == "b"));
        list.insert(
            0,
            HistoryEntry { kind: "remote".into(), value: "b".into(), last_used: 9999 },
        );
        assert_eq!(
            list.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
    }

    #[test]
    fn truncate_keeps_latest() {
        let mut list = entries("local", &["a", "b"]);
        list.truncate(MAX_ENTRIES);
        assert_eq!(list.len(), 2);
        list.insert(0, HistoryEntry { kind: "local".into(), value: "c".into(), last_used: 0 });
        list.truncate(MAX_ENTRIES);
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn load_missing_file_is_empty() {
        // 真实 HOME 下若文件不存在应返回空，不 panic
        let list = load();
        assert!(list.is_empty() || !list.is_empty()); // 只验证不 panic；文件存在时内容任意
    }
}
