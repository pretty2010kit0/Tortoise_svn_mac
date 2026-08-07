//! TaskManager：长任务异步化（功能待办批次 8）。
//! - 重操作（checkout/update/import/export）在后台线程执行，前端轮询 `task_list` 显示进度
//! - 每个任务带取消标志（`AtomicBool`）；执行循环检测到取消即 kill 子进程
//! - 写任务在后台线程内获取 `write_lock`（阻塞等待，不占主线程/主进程事件循环）
//! - 任务列表保留最近 `MAX_TASKS` 条历史

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 写操作全局串行锁（计划 4.3 TaskManager 约束：同一工作副本禁止并发写）。
/// 同步写命令在主线程直接持有；后台任务在子线程内持有。
pub static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn write_lock() -> MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 任务状态（与前端 TaskState 对应）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// 对外暴露的任务信息
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: u64,
    pub desc: String,
    pub state: TaskState,
    pub started_at: u64, // epoch millis
    pub finished_at: Option<u64>,
    /// 失败时的错误摘要 / 取消标记；成功时可为空
    pub output: String,
    /// 成功时的结果摘要（如「已更新到 revision 5」）
    pub result: Option<String>,
}

/// 任务重试参数（前端「重试」按钮使用）：保存原始输入，失败后可重新创建任务
#[derive(Clone)]
pub enum RetrySpec {
    Import { local: String, url: String, message: String },
    Checkout { url: String, dest: String },
    Update { path: String },
    Export { url: String, dest: String, rev: Option<i64> },
}

/// 内部任务条目（含取消标志）
struct TaskEntry {
    id: u64,
    desc: String,
    state: TaskState,
    started_at: u64,
    finished_at: Option<u64>,
    output: String,
    result: Option<String>,
    cancel: Arc<AtomicBool>,
    retry: Option<RetrySpec>,
}

const MAX_TASKS: usize = 50;

static TASKS: OnceLock<Mutex<Vec<TaskEntry>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn tasks() -> &'static Mutex<Vec<TaskEntry>> {
    TASKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 创建任务，返回 (id, 取消标志)。执行线程持有取消标志，检测到后应尽快终止。
pub fn create_task(desc: String) -> (u64, Arc<AtomicBool>) {
    create_task_with_retry(desc, None)
}

/// 创建任务（可携带重试参数）
pub fn create_task_with_retry(desc: String, retry: Option<RetrySpec>) -> (u64, Arc<AtomicBool>) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    tasks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(TaskEntry {
            id,
            desc,
            state: TaskState::Running,
            started_at: now_millis(),
            finished_at: None,
            output: String::new(),
            result: None,
            cancel: cancel.clone(),
            retry,
        });
    (id, cancel)
}

/// 结束任务：写入最终状态（执行线程最后调用）
pub fn finish_task(id: u64, state: TaskState, output: String, result: Option<String>) {
    let mut t = tasks().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = t.iter_mut().find(|e| e.id == id) {
        e.state = state;
        e.finished_at = Some(now_millis());
        e.output = output;
        e.result = result;
    }
    // 截断历史
    if t.len() > MAX_TASKS {
        let drop = t.len() - MAX_TASKS;
        t.drain(..drop);
    }
}

/// 请求取消任务。仅 Running 任务可取消；已结束返回 false（前端据此提示）。
pub fn cancel_task(id: u64) -> bool {
    let mut t = tasks().lock().unwrap_or_else(|e| e.into_inner());
    match t.iter_mut().find(|e| e.id == id) {
        Some(e) if e.state == TaskState::Running => {
            e.cancel.store(true, Ordering::Relaxed);
            true
        }
        _ => false,
    }
}

/// 取任务的重试参数（仅失败/取消任务提供）
pub fn get_retry(id: u64) -> Option<RetrySpec> {
    let t = tasks().lock().unwrap_or_else(|e| e.into_inner());
    t.iter()
        .find(|e| e.id == id && (e.state == TaskState::Failed || e.state == TaskState::Cancelled))
        .and_then(|e| e.retry.clone())
}

/// 任务列表（含历史，倒序：新→旧）
pub fn list_tasks() -> Vec<TaskInfo> {
    let t = tasks().lock().unwrap_or_else(|e| e.into_inner());
    t.iter()
        .rev()
        .map(|e| TaskInfo {
            id: e.id,
            desc: e.desc.clone(),
            state: e.state,
            started_at: e.started_at,
            finished_at: e.finished_at,
            output: e.output.clone(),
            result: e.result.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lifecycle_works() {
        let (id, cancel) = create_task("测试任务".into());
        assert!(!cancel.load(Ordering::Relaxed));
        let list = list_tasks();
        assert_eq!(
            list.iter().find(|t| t.id == id).unwrap().state,
            TaskState::Running
        );
        assert!(cancel_task(id));
        assert!(cancel.load(Ordering::Relaxed));
        // 已取消但未 finish 前状态仍是 Running（执行线程负责收尾）
        finish_task(id, TaskState::Cancelled, String::new(), None);
        let list = list_tasks();
        let t = list.iter().find(|t| t.id == id).unwrap();
        assert_eq!(t.state, TaskState::Cancelled);
        assert!(t.finished_at.is_some());
        // 已结束任务不可再取消
        assert!(!cancel_task(id));
    }

    #[test]
    fn task_list_keeps_history_and_trims() {
        for i in 0..(MAX_TASKS + 5) {
            let (id, _) = create_task(format!("任务 {i}"));
            finish_task(id, TaskState::Done, String::new(), Some("ok".into()));
        }
        let list = list_tasks();
        assert_eq!(list.len(), MAX_TASKS);
        // 倒序：最新在最前
        assert_eq!(list[0].desc, format!("任务 {}", MAX_TASKS + 4));
    }

    #[test]
    fn get_retry_only_for_failed_or_cancelled() {
        // 失败任务：可取到参数
        let (id1, _) = create_task_with_retry(
            "导入".into(),
            Some(RetrySpec::Import {
                local: "/tmp/x".into(),
                url: "svn://h/r".into(),
                message: "m".into(),
            }),
        );
        finish_task(id1, TaskState::Failed, "err".into(), None);
        match get_retry(id1) {
            Some(RetrySpec::Import { local, url, message }) => {
                assert_eq!(local, "/tmp/x");
                assert_eq!(url, "svn://h/r");
                assert_eq!(message, "m");
            }
            other => panic!("应返回 Import 参数: {:?}", other.is_some()),
        }
        // Running 任务：不可重试
        let (id2, _) = create_task_with_retry(
            "更新".into(),
            Some(RetrySpec::Update { path: "/wc".into() }),
        );
        assert!(get_retry(id2).is_none(), "Running 任务不可重试");
        // 无参数任务：None
        let (id3, _) = create_task("普通任务".into());
        finish_task(id3, TaskState::Failed, "err".into(), None);
        assert!(get_retry(id3).is_none());
    }
}
