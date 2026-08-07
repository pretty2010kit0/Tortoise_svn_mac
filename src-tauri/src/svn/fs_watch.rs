//! 工作副本文件监听（批次 16）：notify 递归监听 WC 目录，变化时向前端发事件，
//! 前端防抖后增量刷新（替代/补充定时轮询）。

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

static WATCHERS: Mutex<Option<HashMap<u64, (String, Arc<AtomicBool>)>>> = Mutex::new(None);
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 开始监听工作副本目录（递归）。重复监听同一目录返回已有 id。
#[tauri::command]
pub fn wc_watch_start(path: String, app: AppHandle) -> Result<u64, String> {
    let mut guard = WATCHERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(id) = map
        .iter()
        .find_map(|(id, (p, _))| if p == &path { Some(*id) } else { None })
    {
        return Ok(id);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let path_clone = path.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if watcher
            .watch(std::path::Path::new(&path_clone), RecursiveMode::Recursive)
            .is_err()
        {
            return;
        }
        loop {
            if cancel_clone.load(Ordering::Relaxed) {
                break;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Ok(event)) => {
                    // 只关心 文件/目录 的创建、修改、删除、重命名（忽略访问等噪音）
                    let kinds = event.kind;
                    let interesting = matches!(
                        kinds,
                        notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    );
                    if interesting {
                        // 发给前端统一处理（前端做防抖合并）
                        let _ = app_clone.emit("wc-fs-event", ());
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => {}
            }
        }
        // 线程退出 → watcher drop
    });
    map.insert(id, (path, cancel));
    Ok(id)
}

/// 停止监听
#[tauri::command]
pub fn wc_watch_stop(id: u64) -> Result<(), String> {
    let mut guard = WATCHERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        if let Some((_, cancel)) = map.remove(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
