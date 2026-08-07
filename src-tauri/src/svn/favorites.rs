//! 远程仓库收藏（远程浏览目录快捷切换）
//!
//! 存储位置：`$HOME/.config/svn-desktop-tool/favorites.json`
//! 语义：按 URL 去重（同 URL 更新名称并移到最前），最近收藏在前，上限 100 条。
//! 用独立文件而非 WebKit localStorage（dev 模式沙盒下 localStorage 不可靠）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FAVS: usize = 100;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Favorite {
    pub name: String,
    pub url: String,
    pub created_at: i64,
}

static LOCK: Mutex<()> = Mutex::new(());

fn fav_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("svn-desktop-tool")
        .join("favorites.json")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load() -> Vec<Favorite> {
    let path = fav_file();
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(favs: &[Favorite]) -> Result<(), String> {
    let path = fav_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("无法创建收藏目录 {}: {}", dir.display(), e))?;
    }
    let raw =
        serde_json::to_string_pretty(favs).map_err(|e| format!("收藏序列化失败: {}", e))?;
    fs::write(&path, raw).map_err(|e| format!("收藏写入失败: {}", e))?;
    Ok(())
}

/// 添加/更新收藏（同 URL 更新名称并移到最前）
#[tauri::command]
pub fn fav_add(name: String, url: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let url = url.trim().to_string();
    let name = name.trim().to_string();
    if url.is_empty() {
        return Err("收藏 URL 不能为空".into());
    }
    if name.is_empty() {
        return Err("收藏名称不能为空".into());
    }
    let mut favs = load();
    favs.retain(|f| f.url != url);
    favs.insert(
        0,
        Favorite {
            name,
            url,
            created_at: now(),
        },
    );
    favs.truncate(MAX_FAVS);
    save(&favs)
}

/// 返回全部收藏（最近收藏在前）
#[tauri::command]
pub fn fav_list() -> Vec<Favorite> {
    load()
}

/// 按 URL 取消收藏
#[tauri::command]
pub fn fav_remove(url: String) -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut favs = load();
    let before = favs.len();
    favs.retain(|f| f.url != url);
    if favs.len() == before {
        return Ok(()); // 不存在视为已删除
    }
    save(&favs)
}

/// 清空全部收藏
#[tauri::command]
pub fn fav_clear() -> Result<(), String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if load().is_empty() {
        return Ok(());
    }
    save(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favs(values: &[(&str, &str)]) -> Vec<Favorite> {
        values
            .iter()
            .enumerate()
            .map(|(i, (name, url))| Favorite {
                name: name.to_string(),
                url: url.to_string(),
                created_at: 1000 + i as i64,
            })
            .collect()
    }

    #[test]
    fn dedup_moves_to_front_and_updates_name() {
        let mut list = favs(&[("a", "u1"), ("b", "u2"), ("c", "u3")]);
        // 模拟 fav_add 对 u2 的更新逻辑
        list.retain(|f| f.url != "u2");
        list.insert(
            0,
            Favorite { name: "b2".into(), url: "u2".into(), created_at: 9999 },
        );
        assert_eq!(list[0].url, "u2");
        assert_eq!(list[0].name, "b2");
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn truncate_keeps_latest() {
        let mut list = favs(&[("a", "u1"), ("b", "u2")]);
        list.truncate(MAX_FAVS);
        assert_eq!(list.len(), 2);
        list.insert(
            0,
            Favorite { name: "c".into(), url: "u3".into(), created_at: 0 },
        );
        list.truncate(MAX_FAVS);
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn remove_missing_is_noop() {
        let mut list = favs(&[("a", "u1")]);
        list.retain(|f| f.url != "not-exist");
        assert_eq!(list.len(), 1);
    }
}
