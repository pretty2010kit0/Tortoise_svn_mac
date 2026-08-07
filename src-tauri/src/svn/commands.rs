//! Tauri 命令层：前端通过 invoke 调用的全部操作（第一阶段 MVP 闭环）。
//! 原则：读操作可并发；写操作（checkout/update/commit/add/revert）统一串行（write_lock）。
//! 所有 svn 进程经 runner 启动（参数数组、LC_ALL=C、原始字节）。

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::Serialize;

use crate::svn::models::{
    AuthCred, BlameLine, ConflictInfo, DiffChunk, DiffResult, DirStats, FileContent, FilePair,
    ListEntry, LogEntry, PropEntry, RepoInfo, RepoLayout, StatusEntry, StatusU, SvnError,
    TaskResult, WcInfo,
};
use crate::svn::parser::{
    assemble_conflict, assemble_wc, is_binary, parse_auth_creds, parse_blame,
    parse_conflict_bytes, parse_info, parse_list, parse_log, parse_proplist, parse_status, Choice,
};
use crate::svn::runner::{
    run_svn, run_svn_any, run_svn_any_with_bin, run_svn_auth, run_svn_cancellable_long,
    set_svn_bin, svn_bin, utf8_escape,
};
use crate::svn::task::write_lock;

/// svn 版本与二进制路径（M1 骨架 IPC 验证 + 能力探测）
#[tauri::command]
pub fn svn_version() -> Result<SvnVersion, SvnError> {
    let bin = svn_bin();
    let out = run_svn_any(&["--version".into(), "--quiet".into()], None)?;
    if !out.success() {
        return Err(SvnError::from_svn(&out.stderr, "svn 版本探测"));
    }
    Ok(SvnVersion {
        bin: bin.display().to_string(),
        version: out.stdout.trim().to_string(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnVersion {
    pub bin: String,
    pub version: String,
}

/// 设置 svn 二进制路径：验证存在且可执行，返回版本号
#[tauri::command]
pub fn set_svn_executable(path: String) -> Result<SvnVersion, SvnError> {
    let p = PathBuf::from(path.trim());
    if !p.exists() || !p.is_file() {
        return Err(SvnError::new(
            "io",
            "svn 路径不存在",
            &path,
            "请填写 svn 可执行文件的完整路径（如 /opt/homebrew/bin/svn）。",
        ));
    }
    let out = run_svn_any_with_bin(&p, &["--version".into(), "--quiet".into()], None)?;
    if !out.success() {
        return Err(SvnError::new(
            "io",
            "该路径不是可用的 svn 程序",
            &out.stderr,
            "请选择 subversion 的 svn 可执行文件。",
        ));
    }
    set_svn_bin(p.clone());
    Ok(SvnVersion {
        bin: p.display().to_string(),
        version: out.stdout.trim().to_string(),
    })
}

/// 打开远程 URL：svn info --xml 连接/认证预检 + list 预检条目数
/// 列出认证缓存中的凭据（svn auth）
#[tauri::command]
pub fn svn_auth_list() -> Result<Vec<AuthCred>, SvnError> {
    let out = run_svn_any(&["auth".into()], None)?;
    Ok(parse_auth_creds(&out.stdout))
}

/// 清除认证缓存（svn auth --remove PATTERN...；"*" 清除全部）
#[tauri::command]
pub fn svn_auth_remove(patterns: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if patterns.is_empty() {
        return Err(SvnError::new("usage", "未指定要清除的凭据", "", "请选择一条凭据或「全部清除」。"));
    }
    let mut args = vec!["auth".into(), "--remove".into()];
    args.extend(patterns.iter().cloned());
    let out = run_svn_any(&args, None)?;
    if !out.success() {
        // E200009 = no matching credentials：目标已不存在，视为已清空
        if out.stderr.contains("E200009") {
            return Ok(TaskResult {
                ok: true,
                summary: "凭据缓存已清除（或原本不存在匹配项）".to_string(),
                stdout: out.stdout,
                stderr: out.stderr,
            });
        }
        return Err(SvnError::from_svn(&out.stderr, "清除认证缓存"));
    }
    Ok(TaskResult {
        ok: true,
        summary: format!("已清除 {} 条模式匹配的凭据", patterns.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

#[tauri::command]
pub fn remote_open(url: String) -> Result<RepoInfo, SvnError> {
    let out = run_svn(&["info".into(), "--xml".into(), url.clone()], None, "连接仓库")?;
    let mut info = parse_info(&out.stdout_bytes).map_err(|e| SvnError::new(
        "internal",
        "解析 svn info 失败",
        &e,
        "请报告此问题并附带诊断日志。",
    ))?;
    info.url = url.clone();
    // 预检 list：失败不阻塞（info 已通过认证），条目数留 0
    if let Ok(list) = run_svn(&["list".into(), "--xml".into(), url], None, "读取目录") {
        if let Ok(entries) = parse_list(&list.stdout_bytes) {
            info.entry_count = entries.len();
        }
    }
    Ok(info)
}

/// 带用户名/密码连接仓库（认证失败重试路径，密码仅进 stdin）
#[tauri::command]
pub fn remote_open_auth(
    url: String,
    username: String,
    password: String,
) -> Result<RepoInfo, SvnError> {
    let out = run_svn_auth(
        &["info".into(), "--xml".into(), url.clone()],
        None,
        "连接仓库（认证重试）",
        &username,
        &password,
    )?;
    let mut info = parse_info(&out.stdout_bytes).map_err(|e| SvnError::new(
        "internal",
        "解析 svn info 失败",
        &e,
        "请报告此问题并附带诊断日志。",
    ))?;
    info.url = url.clone();
    if let Ok(list) = run_svn_auth(
        &["list".into(), "--xml".into(), url],
        None,
        "读取目录",
        &username,
        &password,
    ) {
        if let Ok(entries) = parse_list(&list.stdout_bytes) {
            info.entry_count = entries.len();
        }
    }
    Ok(info)
}

/// 临时信任证书连接（证书不受信任时重试路径，批次 15）
#[tauri::command]
pub fn remote_open_trust(url: String) -> Result<RepoInfo, SvnError> {
    let trust_args = [
        "--non-interactive".to_string(),
        "--trust-server-cert-failures".to_string(),
        "unknown-ca,cn-mismatch,expired,not-yet-valid,other".to_string(),
    ];
    let mut args = trust_args.to_vec();
    args.push("info".into());
    args.push("--xml".into());
    args.push(url.clone());
    let out = run_svn(&args, None, "连接仓库（临时信任证书）")?;
    let mut info = parse_info(&out.stdout_bytes).map_err(|e| SvnError::new(
        "internal",
        "解析 svn info 失败",
        &e,
        "请报告此问题并附带诊断日志。",
    ))?;
    info.url = url.clone();
    let mut list_args = trust_args.to_vec();
    list_args.push("list".into());
    list_args.push("--xml".into());
    list_args.push(url);
    if let Ok(list) = run_svn(&list_args, None, "读取目录") {
        if let Ok(entries) = parse_list(&list.stdout_bytes) {
            info.entry_count = entries.len();
        }
    }
    Ok(info)
}

/// 将选中的未版本化路径加入其父目录的 svn:ignore（批量忽略，批次 15）
#[tauri::command]
pub fn wc_ignore_add(paths: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new(
            "usage",
            "未选择要忽略的路径",
            "",
            "请先勾选未版本化的文件或目录。",
        ));
    }
    // 按父目录分组：svn:ignore 作用于目录的直接子项
    let mut by_parent: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for p in &paths {
        let path = Path::new(p);
        let parent = path
            .parent()
            .map(|x| x.display().to_string())
            .unwrap_or_default();
        let name = path
            .file_name()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.is_empty() {
            by_parent.entry(parent).or_default().push(name);
        }
    }
    let mut added = 0usize;
    for (parent, names) in &by_parent {
        // 读取现有 svn:ignore（属性不存在时 propget 失败 → 视为空）
        let existing: Vec<String> = match run_svn_any(
            &["propget".into(), "svn:ignore".into(), parent.clone()],
            None,
        ) {
            Ok(out) if out.success() => out
                .stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        let mut merged = existing.clone();
        for n in names {
            if !merged.iter().any(|x| x == n) {
                merged.push(n.clone());
                added += 1;
            }
        }
        if merged.is_empty() {
            continue;
        }
        let tmp = write_msg_file(&merged.join("\n"))?;
        let args = vec![
            "propset".into(),
            "svn:ignore".into(),
            "-F".into(),
            tmp.to_string_lossy().into_owned(),
            parent.clone(),
        ];
        let out = run_svn(&args, None, "设置 svn:ignore")?;
        let _ = std::fs::remove_file(&tmp);
        let _ = out;
    }
    Ok(TaskResult {
        ok: true,
        summary: format!("已将 {} 个路径加入 svn:ignore（{} 个目录）", added, by_parent.len()),
        stdout: String::new(),
        stderr: String::new(),
    })
}

/// 远程目录列表（惰性加载单层）
#[tauri::command]
pub fn remote_list(url: String, rev: Option<i64>) -> Result<Vec<ListEntry>, SvnError> {
    let mut args = vec!["list".into(), "--xml".into()];
    if let Some(r) = rev {
        args.push("-r".into());
        args.push(r.to_string());
    }
    let target = peg_url(&url, rev);
    args.push(target);
    let out = run_svn(&args, None, "读取目录")?;
    parse_list(&out.stdout_bytes).map_err(|e| {
        SvnError::new("internal", "解析 svn list 失败", &e, "请报告此问题。")
    })
}

/// 远程文件内容（原始字节 → base64）
#[tauri::command]
pub fn remote_cat(url: String, rev: Option<i64>) -> Result<FileContent, SvnError> {
    let mut args = vec!["cat".into()];
    if let Some(r) = rev {
        args.push("-r".into());
        args.push(r.to_string());
    }
    args.push(peg_url(&url, rev));
    let out = run_svn(&args, None, "读取文件")?;
    Ok(FileContent {
        data_base64: B64.encode(&out.stdout_bytes),
        size: out.stdout_bytes.len(),
        is_utf8: std::str::from_utf8(&out.stdout_bytes).is_ok(),
        is_binary: is_binary(&out.stdout_bytes),
    })
}

/// 导出远程文件/目录到本地（svn export，不带 .svn 元数据的干净副本）
#[tauri::command]
pub fn remote_export(url: String, dest: String, rev: Option<i64>) -> Result<TaskResult, SvnError> {
    if dest.trim().is_empty() {
        return Err(SvnError::new(
            "io",
            "目标路径为空",
            "",
            "请选择要保存到的本地路径。",
        ));
    }
    let mut args = vec!["export".into(), "--force".into()];
    if let Some(r) = rev {
        args.push("-r".into());
        args.push(r.to_string());
    }
    args.push(peg_url(&url, rev));
    args.push(dest);
    let out = run_svn(&args, None, "导出")?;
    let stdout = utf8_escape(&out.stdout_bytes);
    let stderr = utf8_escape(&out.stderr_bytes);
    // svn export 末尾输出 "Export complete." / "Exported revision N."，取最后非空行
    let summary = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("导出完成");
    Ok(TaskResult {
        ok: true,
        summary: summary.to_string(),
        stdout,
        stderr,
    })
}

/// 仓库标准布局探测（分支/标签管理，批次 15）
/// 探测 root/trunk、root/branches、root/tags 是否存在，并列出 branches/tags 下的子目录名。
/// 不存在的目录返回 None/空，不报错。
#[tauri::command]
pub fn remote_repo_layout(root_url: String) -> Result<RepoLayout, SvnError> {
    use crate::svn::models::RepoLayout;
    let root = root_url.trim_end_matches('/').to_string();
    let mut layout = RepoLayout {
        trunk: None,
        branches_dir: None,
        tags_dir: None,
        branches: Vec::new(),
        tags: Vec::new(),
    };
    let probe = |sub: &str| -> Option<String> {
        let url = format!("{root}/{sub}");
        match run_svn_any(&["list".into(), "--xml".into(), url.clone()], None) {
            Ok(out) if out.success() => Some(url),
            _ => None,
        }
    };
    layout.trunk = probe("trunk");
    layout.branches_dir = probe("branches");
    layout.tags_dir = probe("tags");
    let list_dirs = |url: &str| -> Vec<String> {
        match run_svn_any(&["list".into(), url.to_string()], None) {
            Ok(out) if out.success() => out
                .stdout
                .lines()
                .map(|l| l.trim_end_matches('/').to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    };
    if let Some(u) = &layout.branches_dir {
        layout.branches = list_dirs(u);
    }
    if let Some(u) = &layout.tags_dir {
        layout.tags = list_dirs(u);
    }
    Ok(layout)
}

/// 远程日志（-v 含变更路径）
/// 远程日志：支持 limit / revision 范围 / --search（匹配说明/作者/路径/日期）/ 日期范围
#[tauri::command]
pub fn remote_log(
    url: String,
    limit: Option<u32>,
    rev: Option<i64>,
    search: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
) -> Result<Vec<LogEntry>, SvnError> {
    let mut args = vec!["log".into(), "--xml".into(), "-v".into()];
    if date_from.is_some() || date_to.is_some() {
        // 日期范围：-r {from}:{to}，缺侧用 HEAD / 1
        let f = date_from.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let t = date_to.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let from = f.map(|s| format!("{{{s}}}")).unwrap_or_else(|| "1".into());
        let to = t
            .map(|s| {
                if s.contains('T') {
                    format!("{{{s}}}")
                } else {
                    // 纯日期视为当天整天：补 23:59:59
                    format!("{{{s}T23:59:59}}")
                }
            })
            .unwrap_or_else(|| "HEAD".into());
        args.push("-r".into());
        args.push(format!("{from}:{to}"));
    } else if let Some(r) = rev {
        args.push("-r".into());
        args.push(format!("{r}:1"));
    }
    if let Some(s) = search {
        if !s.trim().is_empty() {
            args.push("--search".into());
            args.push(s);
        }
    }
    if let Some(l) = limit {
        args.push("-l".into());
        args.push(l.to_string());
    }
    args.push(url);
    let out = run_svn(&args, None, "读取日志")?;
    parse_log(&out.stdout_bytes).map_err(|e| {
        SvnError::new("internal", "解析 svn log 失败", &e, "请报告此问题。")
    })
}

/// 远程 revision 间差异（同一路径）
#[tauri::command]
pub fn remote_diff(url: String, rev1: i64, rev2: i64) -> Result<DiffResult, SvnError> {
    // peg 修正（计划 5.3）：先 peg 到 rev2；若路径在 rev2 不存在则回退 rev1
    let target_r2 = peg_url(&url, Some(rev2));
    let args_r2 = vec![
        "diff".into(),
        "-r".into(),
        format!("{rev1}:{rev2}"),
        target_r2,
    ];
    let out = match run_svn(&args_r2, None, "比较版本") {
        Ok(o) => o,
        Err(e) if e.category == "not-found" => {
            let target_r1 = peg_url(&url, Some(rev1));
            let args_r1 = vec![
                "diff".into(),
                "-r".into(),
                format!("{rev1}:{rev2}"),
                target_r1,
            ];
            run_svn(&args_r1, None, "比较版本")?
        }
        Err(e) => return Err(e),
    };
    Ok(diff_result(&out))
}

/// 打开工作副本目录：info + status
#[tauri::command]
pub fn wc_open(path: String) -> Result<WcInfo, SvnError> {
    let info_out = run_svn(&["info".into(), "--xml".into(), path.clone()], None, "读取工作副本")?;
    let info = parse_info(&info_out.stdout_bytes).map_err(|e| {
        SvnError::new("internal", "解析 svn info 失败", &e, "请报告此问题。")
    })?;
    let status = wc_status_inner(&path)?;
    Ok(assemble_wc(info, status))
}

/// 工作副本状态
#[tauri::command]
pub fn wc_status(path: String) -> Result<Vec<StatusEntry>, SvnError> {
    wc_status_inner(&path)
}

fn wc_status_inner(path: &str) -> Result<Vec<StatusEntry>, SvnError> {
    let out = run_svn(&["status".into(), "--xml".into(), path.to_string()], None, "读取状态")?;
    let (entries, _) = parse_status(&out.stdout_bytes).map_err(|e| {
        SvnError::new("internal", "解析 svn status 失败", &e, "请报告此问题。")
    })?;
    Ok(entries)
}

/// 与服务器比较：svn status -u --xml（返回条目 + 服务器最新 revision）
#[tauri::command]
pub fn wc_status_u(path: String) -> Result<StatusU, SvnError> {
    let out = run_svn(
        &["status".into(), "-u".into(), "--xml".into(), path.clone()],
        None,
        "与服务器比较",
    )?;
    let (entries, against) = parse_status(&out.stdout_bytes).map_err(|e| {
        SvnError::new("internal", "解析 svn status -u 失败", &e, "请报告此问题。")
    })?;
    Ok(StatusU { entries, against })
}

/// 逐行归属：svn blame（纯文本解析）
#[tauri::command]
pub fn wc_blame(path: String, rev: Option<i64>) -> Result<Vec<BlameLine>, SvnError> {
    let mut args = vec!["blame".into()];
    if let Some(r) = rev {
        args.push("-r".into());
        args.push(r.to_string());
    }
    args.push(path);
    let out = run_svn(&args, None, "查看 blame")?;
    parse_blame(&out.stdout_bytes)
        .map_err(|e| SvnError::new("internal", "解析 svn blame 失败", &e, "请报告此问题。"))
}

/// changelist：添加/移除。remove=true 时从 changelist 移除
#[tauri::command]
pub fn wc_changelist(name: String, paths: Vec<String>, remove: bool) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new("usage", "未选择要加入变更集的路径", "", ""));
    }
    let mut args = vec!["changelist".into()];
    if remove {
        args.push("--remove".into());
    } else {
        if name.trim().is_empty() {
            return Err(SvnError::new("usage", "变更集名称不能为空", "", ""));
        }
        args.push(name.clone());
    }
    args.extend(paths.iter().cloned());
    let out = run_svn(&args, None, "变更集")?;
    Ok(TaskResult {
        ok: true,
        summary: if remove {
            format!("已从变更集移除 {} 个路径", paths.len())
        } else {
            format!("已加入变更集 {name}（{} 个路径）", paths.len())
        },
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 按 changelist 提交：svn commit --changelist NAME -F tmp（cwd 需为工作副本根）
#[tauri::command]
pub fn wc_commit_cl(name: String, message: String, wc_path: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if name.trim().is_empty() {
        return Err(SvnError::new("usage", "变更集名称不能为空", "", ""));
    }
    let tmp = write_msg_file(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let args = vec![
            "commit".into(),
            "--changelist".into(),
            name.clone(),
            "--encoding".into(),
            "utf-8".into(),
            "-F".into(),
            tmp.to_string_lossy().into_owned(),
        ];
        let cwd = std::path::Path::new(&wc_path);
        let out = run_svn(&args, Some(cwd), "提交变更集")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已提交变更集 {name}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 工作副本文件两侧内容（并排 diff 用）：BASE 基线 vs 当前工作区
#[tauri::command]
pub fn wc_file_pair(path: String) -> Result<FilePair, SvnError> {
    let status = wc_status_inner(&path)?;
    let unversioned = status.iter().any(|s| s.item == "unversioned");
    let new_bytes = std::fs::read(&path).map_err(|e| {
        SvnError::new(
            "io",
            "读取文件失败",
            &e.to_string(),
            "文件可能已被删除，请使用「还原」恢复。",
        )
    })?;
    let old_text;
    let old_is_binary;
    if unversioned {
        // 未版本化文件无基线：左侧为空
        old_text = String::new();
        old_is_binary = false;
    } else {
        let cwd = Path::new(&path).parent();
        let out = run_svn(
            &["cat".into(), "-r".into(), "BASE".into(), path.clone()],
            cwd,
            "读取基线版本",
        )?;
        old_text = utf8_escape(&out.stdout_bytes);
        old_is_binary = is_binary(&out.stdout_bytes);
    }
    let new_text = utf8_escape(&new_bytes);
    Ok(FilePair {
        old_text,
        new_text,
        is_binary: old_is_binary || is_binary(&new_bytes),
        is_unversioned: unversioned,
    })
}

/// 导出 BASE/工作区到临时文件并调用外部 diff 工具（macOS opendiff / FileMerge）
#[tauri::command]
pub fn wc_diff_external(path: String) -> Result<TaskResult, SvnError> {
    let dir = std::env::temp_dir().join("svn-desktop-tool-diff");
    std::fs::create_dir_all(&dir).map_err(|e| {
        SvnError::new("io", "无法创建临时目录", &e.to_string(), "请检查临时目录权限。")
    })?;
    let fname = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let base_path = dir.join(format!("{fname}.base"));
    let wc_path = dir.join(&fname);
    // BASE 基线（cwd 用父目录，svn cat 相对路径需要）
    let cwd = Path::new(&path).parent();
    let out = run_svn(
        &["cat".into(), "-r".into(), "BASE".into(), path.clone()],
        cwd,
        "读取基线版本",
    )?;
    std::fs::write(&base_path, &out.stdout_bytes).map_err(|e| {
        SvnError::new("io", "写入基线临时文件失败", &e.to_string(), "")
    })?;
    let wc_bytes = std::fs::read(&path).map_err(|e| {
        SvnError::new("io", "读取工作副本文件失败", &e.to_string(), "文件可能已被删除。")
    })?;
    std::fs::write(&wc_path, &wc_bytes).map_err(|e| {
        SvnError::new("io", "写入工作副本临时文件失败", &e.to_string(), "")
    })?;
    // 启动外部 diff（spawn 不等待，独立窗口；工具路径来自设置，默认 /usr/bin/opendiff）
    let tool = crate::svn::settings::external_diff_cmd();
    let spawn = std::process::Command::new(&tool)
        .args([
            base_path.to_str().unwrap_or_default(),
            wc_path.to_str().unwrap_or_default(),
        ])
        .spawn();
    match spawn {
        Ok(_) => Ok(TaskResult {
            ok: true,
            summary: format!("已用外部 diff 工具打开：{}（{}）", fname, tool),
            stdout: String::new(),
            stderr: String::new(),
        }),
        Err(e) => Err(SvnError::new(
            "io",
            "无法启动外部 diff 工具",
            &format!("{tool}：{e}"),
            "请在设置中配置正确的 diff 工具路径（macOS 默认 /usr/bin/opendiff）。",
        )),
    }
}

/// 计算两段文本的变更块（大文件并排 diff 用，主线程零 diff 计算）。
/// 行级 diff 后换算为字符位置，与 @codemirror/merge 的 Chunk 语义一致。
#[tauri::command]
pub fn diff_chunks(old_text: String, new_text: String) -> Vec<DiffChunk> {
    let old_offsets = line_offsets(&old_text);
    let new_offsets = line_offsets(&new_text);
    let diff = similar::TextDiff::from_lines(&old_text, &new_text);
    let mut chunks = Vec::new();
    for op in diff.ops() {
        let last_o = old_offsets.len() - 1;
        let last_n = new_offsets.len() - 1;
        match op {
            similar::DiffOp::Equal { .. } => {}
            similar::DiffOp::Delete {
                old_index,
                new_index,
                old_len,
            } => {
                let i = *old_index;
                let n = *new_index;
                let l = *old_len;
                chunks.push(DiffChunk {
                    from_a: old_offsets[i.min(last_o)],
                    to_a: old_offsets[(i + l).min(last_o)],
                    from_b: new_offsets[n.min(last_n)],
                    to_b: new_offsets[n.min(last_n)],
                });
            }
            similar::DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                let i = *old_index;
                let n = *new_index;
                let l = *new_len;
                chunks.push(DiffChunk {
                    from_a: old_offsets[i.min(last_o)],
                    to_a: old_offsets[i.min(last_o)],
                    from_b: new_offsets[n.min(last_n)],
                    to_b: new_offsets[(n + l).min(last_n)],
                });
            }
            similar::DiffOp::Replace {
                old_index,
                new_index,
                old_len,
                new_len,
            } => {
                let i = *old_index;
                let n = *new_index;
                let ol = *old_len;
                let nl = *new_len;
                chunks.push(DiffChunk {
                    from_a: old_offsets[i.min(last_o)],
                    to_a: old_offsets[(i + ol).min(last_o)],
                    from_b: new_offsets[n.min(last_n)],
                    to_b: new_offsets[(n + nl).min(last_n)],
                });
            }
        }
    }
    chunks
}

/// 每行起始的字符偏移表（最后一项 = 文本总长，供越界兜底）
fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(i + 1);
        }
    }
    // 文本以换行结尾时末行起始已等于总长，避免重复 push
    if *offsets.last().unwrap() != text.len() {
        offsets.push(text.len());
    }
    offsets
}

/// 本地工作副本 Diff；未版本化文件返回“全新增”语义（无基线）
#[tauri::command]
pub fn wc_diff(path: String) -> Result<DiffResult, SvnError> {
    // 未版本化文件没有基线：读文件内容构造全新增 diff（左侧为空）
    let status = wc_status_inner(&path)?;
    if status.iter().any(|s| s.item == "unversioned") {
        let bytes = std::fs::read(&path).map_err(|e| {
            SvnError::new("io", "读取文件失败", &e.to_string(), "请检查文件是否存在。")
        })?;
        let content = utf8_escape(&bytes);
        let mut text = String::new();
        for line in content.lines() {
            text.push('+');
            text.push(' ');
            text.push_str(line);
            text.push('\n');
        }
        return Ok(DiffResult {
            is_empty: text.trim().is_empty(),
            is_binary: is_binary(&bytes),
            unversioned: true,
            text,
        });
    }
    // cwd 用文件所在目录（svn 要求 current_dir 为目录）
    let cwd = Path::new(&path).parent();
    let out = run_svn(&["diff".into(), path.clone()], cwd, "生成差异")?;
    Ok(diff_result(&out))
}

/// 提交：提交说明走 -F 临时文件（0600），写操作串行
#[tauri::command]
pub fn wc_commit(paths: Vec<String>, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if message.trim().is_empty() {
        return Err(SvnError::new(
            "usage",
            "提交说明不能为空",
            "",
            "请填写提交说明后重试。",
        ));
    }
    let tmp = write_msg_file(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut args = vec![
            "commit".into(),
            "-F".into(),
            tmp.to_string_lossy().into_owned(),
            // 显式声明消息为 UTF-8：LC_ALL=C 下 native encoding 为 ASCII，
            // 若不声明，中文提交说明会报 E000022 无法转换
            "--encoding".into(),
            "utf-8".into(),
        ];
        for p in &paths {
            args.push(p.clone());
        }
        let out = run_svn(&args, None, "提交")?;
        Ok(TaskResult {
            ok: true,
            summary: extract_committed_revision(&out.stdout, &out.stderr),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 远程写操作共用：提交说明非空校验 + -F 临时文件消息参数
fn remote_msg_args(message: &str) -> Result<(Vec<String>, std::path::PathBuf), SvnError> {
    if message.trim().is_empty() {
        return Err(SvnError::new(
            "usage",
            "提交说明不能为空",
            "",
            "远程写操作必须填写提交说明。",
        ));
    }
    let tmp = write_msg_file(message)?;
    Ok((
        vec![
            "-F".into(),
            tmp.to_string_lossy().into_owned(),
            "--encoding".into(),
            "utf-8".into(),
        ],
        tmp,
    ))
}

/// 远程创建目录（svn mkdir URL -m）
#[tauri::command]
pub fn remote_mkdir(url: String, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let (mut args, tmp) = remote_msg_args(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["mkdir".into()];
        full.append(&mut args);
        full.push(url.clone());
        let out = run_svn(&full, None, "创建目录")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已创建目录 {url}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 远程删除（svn delete URL -m）
#[tauri::command]
pub fn remote_delete(url: String, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let (mut args, tmp) = remote_msg_args(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["delete".into()];
        full.append(&mut args);
        full.push(url.clone());
        let out = run_svn(&full, None, "删除")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已删除 {url}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 创建分支/标签（svn copy SRC DST -m）
#[tauri::command]
pub fn remote_copy(src: String, dst: String, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let (mut args, tmp) = remote_msg_args(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["copy".into()];
        full.append(&mut args);
        full.push(src.clone());
        full.push(dst.clone());
        let out = run_svn(&full, None, "创建分支/标签")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已从 {src} 复制到 {dst}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 远程移动/重命名（svn move SRC DST -m）
#[tauri::command]
pub fn remote_move(src: String, dst: String, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let (mut args, tmp) = remote_msg_args(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["move".into()];
        full.append(&mut args);
        full.push(src.clone());
        full.push(dst.clone());
        let out = run_svn(&full, None, "移动/重命名")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已从 {src} 移动到 {dst}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 导入本地目录（svn import LOCAL URL -m：导入目录**内容**到目标 URL）
#[tauri::command]
pub fn remote_import(local: String, url: String, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let (mut args, tmp) = remote_msg_args(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["import".into()];
        full.append(&mut args);
        full.push(local.clone());
        full.push(url.clone());
        let out = run_svn(&full, None, "导入")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已导入 {local} 到 {url}"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 属性列表共用：svn proplist -v --xml（工作副本路径或远程 URL 均可）
fn props_list(target: &str) -> Result<Vec<PropEntry>, SvnError> {
    let args = vec![
        "proplist".to_string(),
        "-v".to_string(),
        "--xml".to_string(),
        target.to_string(),
    ];
    let out = run_svn(&args, None, "属性列表")?;
    parse_proplist(out.stdout.as_bytes())
        .map_err(|e| SvnError::new("parse", "属性解析失败", &e, ""))
}

/// 工作副本属性列表
#[tauri::command]
pub fn wc_proplist(path: String) -> Result<Vec<PropEntry>, SvnError> {
    props_list(&path)
}

/// 远程属性列表（只读查看）
#[tauri::command]
pub fn remote_proplist(url: String) -> Result<Vec<PropEntry>, SvnError> {
    props_list(&url)
}

/// 设置属性（含 svn:ignore）：svn propset NAME -F tmp PATH
#[tauri::command]
pub fn wc_propset(path: String, name: String, value: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if name.trim().is_empty() {
        return Err(SvnError::new(
            "usage",
            "属性名不能为空",
            "",
            "请输入要设置的属性名（如 svn:ignore）。",
        ));
    }
    let tmp = write_msg_file(&value)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let mut full = vec!["propset".into(), name.clone(), "-F".into()];
        full.push(tmp.to_string_lossy().into_owned());
        full.push(path.clone());
        let out = run_svn(&full, None, "设置属性")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已设置属性 {name}（{path}）"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 删除属性：svn propdel NAME PATH
#[tauri::command]
pub fn wc_propdel(path: String, name: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let args = vec!["propdel".to_string(), name.clone(), path.clone()];
    let out = run_svn(&args, None, "删除属性")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已删除属性 {name}（{path}）"),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 锁定（svn lock -F comment paths；注释可为空）
#[tauri::command]
pub fn wc_lock(paths: Vec<String>, comment: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new("usage", "未选择要锁定的路径", "", ""));
    }
    let mut full = vec!["lock".into()];
    let tmp = if comment.trim().is_empty() {
        None
    } else {
        let t = write_msg_file(&comment)?;
        full.push("-F".into());
        full.push(t.to_string_lossy().into_owned());
        Some(t)
    };
    let result = (|| -> Result<TaskResult, SvnError> {
        full.extend(paths.iter().cloned());
        let out = run_svn(&full, None, "锁定")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已锁定 {} 个路径", paths.len()),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    if let Some(t) = tmp {
        let _ = std::fs::remove_file(&t);
    }
    result
}

/// 解锁（svn unlock [--force] paths；force 用于解除他人锁定的文件）
#[tauri::command]
pub fn wc_unlock(paths: Vec<String>, force: bool) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new("usage", "未选择要解锁的路径", "", ""));
    }
    let mut full = vec!["unlock".into()];
    if force {
        full.push("--force".into());
    }
    full.extend(paths.iter().cloned());
    let out = run_svn(&full, None, "解锁")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已解锁 {} 个路径", paths.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 本地移动/重命名（保留历史）：svn move src dst
#[tauri::command]
pub fn wc_move(src: String, dst: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if src.trim().is_empty() || dst.trim().is_empty() {
        return Err(SvnError::new("usage", "源路径与目标路径不能为空", "", ""));
    }
    let out = run_svn(&["move".into(), src.clone(), dst.clone()], None, "移动/重命名")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已移动 {src} → {dst}（提交后生效）"),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 编辑提交说明（revprop）：svn propset --revprop -r REV svn:log -F tmp URL
/// 需要服务端 pre-revprop-change hook 允许，否则返回 E175005 类错误
#[tauri::command]
pub fn wc_set_log(url: String, rev: i64, message: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let tmp = write_msg_file(&message)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let args = vec![
            "propset".into(),
            "--revprop".into(),
            "-r".into(),
            rev.to_string(),
            "svn:log".into(),
            "-F".into(),
            tmp.to_string_lossy().into_owned(),
            url.clone(),
        ];
        let out = run_svn(&args, None, "修改提交说明")?;
        Ok(TaskResult {
            ok: true,
            summary: format!("已修改 r{rev} 的提交说明"),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 切换分支/URL：svn switch [--depth D] URL PATH
#[tauri::command]
pub fn wc_switch(path: String, target_url: String, depth: Option<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if target_url.trim().is_empty() {
        return Err(SvnError::new("usage", "目标 URL 不能为空", "", ""));
    }
    let mut args = vec!["switch".into()];
    if let Some(d) = depth {
        if !d.trim().is_empty() {
            args.push("--depth".into());
            args.push(d);
        }
    }
    args.push(target_url.clone());
    args.push(path.clone());
    let out = run_svn(&args, Some(std::path::Path::new(&path)), "切换分支/URL")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已切换到 {target_url}"),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 重定位：svn relocate [--from OLD] NEW URL PATH
#[tauri::command]
pub fn wc_relocate(path: String, new_url: String, from_url: Option<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if new_url.trim().is_empty() {
        return Err(SvnError::new("usage", "新仓库地址不能为空", "", ""));
    }
    let mut args = vec!["relocate".into()];
    if let Some(f) = from_url {
        if !f.trim().is_empty() {
            // svn 1.14 语法：relocate FROM-PREFIX TO-PREFIX [PATH...]
            args.push(f);
        }
    }
    args.push(new_url.clone());
    args.push(path.clone());
    let out = run_svn(&args, Some(std::path::Path::new(&path)), "重定位")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已重定位到 {new_url}"),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 分支间合并：svn merge [-r F:T] SOURCE TARGET（无 -r 时按 mergeinfo 同步合并）
#[tauri::command]
pub fn wc_merge(
    target: String,
    source_url: String,
    rev_from: Option<i64>,
    rev_to: Option<i64>,
    dry_run: bool,
) -> Result<TaskResult, SvnError> {
    if source_url.trim().is_empty() {
        return Err(SvnError::new("usage", "源 URL 不能为空", "", ""));
    }
    let mut args = vec!["merge".into()];
    if dry_run {
        args.push("--dry-run".into());
    }
    if rev_from.is_some() || rev_to.is_some() {
        let f = rev_from.unwrap_or(1).to_string();
        let t = rev_to
            .map(|v| v.to_string())
            .unwrap_or_else(|| "HEAD".to_string());
        args.push("-r".into());
        args.push(format!("{f}:{t}"));
    }
    args.push(source_url.clone());
    args.push(target.clone());
    // dry-run 只读不写，不加写锁
    if !dry_run {
        let _g = write_lock();
    }
    let out = run_svn(&args, Some(std::path::Path::new(&target)), "合并")?;
    Ok(TaskResult {
        ok: true,
        summary: if dry_run {
            "合并预览完成（未写入工作副本）".to_string()
        } else {
            format!("已合并 {source_url} 到工作副本，请检查后提交")
        },
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 创建补丁：svn diff（工作副本相对 BASE 的完整 diff 文本）
#[tauri::command]
pub fn wc_diff_text(path: String) -> Result<String, SvnError> {
    let out = run_svn(&["diff".into(), path.clone()], Some(std::path::Path::new(&path)), "创建补丁")?;
    Ok(out.stdout)
}

/// 应用补丁：svn patch（patch_text 写临时文件）
#[tauri::command]
pub fn wc_patch_apply(path: String, patch_text: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if patch_text.trim().is_empty() {
        return Err(SvnError::new("usage", "补丁内容为空", "", ""));
    }
    let tmp = write_patch_file(&patch_text)?;
    let result = (|| -> Result<TaskResult, SvnError> {
        let args = vec![
            "patch".into(),
            tmp.to_string_lossy().into_owned(),
            path.clone(),
        ];
        let out = run_svn(&args, Some(std::path::Path::new(&path)), "应用补丁")?;
        Ok(TaskResult {
            ok: true,
            summary: "补丁已应用".to_string(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 更新到最新
#[tauri::command]
pub fn wc_update(path: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let out = run_svn(&["update".into(), path.clone()], None, "更新")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("更新完成，已更新到 revision {}", last_line(&out.stdout)),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Checkout 到本地
#[tauri::command]
pub fn wc_checkout(url: String, dest: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let out = run_svn(
        &["checkout".into(), url, dest],
        None,
        "检出工作副本",
    )?;
    Ok(TaskResult {
        ok: true,
        summary: "检出完成".to_string(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 添加文件到版本控制（svn add --parents，忽略已版本化项）
#[tauri::command]
pub fn wc_add(paths: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let mut args = vec!["add".into(), "--parents".into()];
    args.extend(paths.clone());
    let out = run_svn(&args, None, "添加")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已添加 {} 个路径", paths.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 删除文件/文件夹（svn delete --force，标记待提交；提交后服务端生效）
#[tauri::command]
pub fn wc_delete(paths: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new(
            "io",
            "未选择要删除的路径",
            "",
            "请先勾选要删除的文件或文件夹。",
        ));
    }
    let mut args = vec!["delete".into(), "--force".into()];
    args.extend(paths.clone());
    let out = run_svn(&args, None, "删除")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已标记删除 {} 个路径（提交后生效）", paths.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 解析冲突文件（三方合并）：返回逐块 mine/base/theirs 文本
#[tauri::command]
pub fn wc_conflict_parse(path: String) -> Result<ConflictInfo, SvnError> {
    let bytes = std::fs::read(&path).map_err(|e| {
        SvnError::new(
            "io",
            "读取冲突文件失败",
            &e.to_string(),
            "文件可能已被删除，请使用「还原」恢复。",
        )
    })?;
    let doc = parse_conflict_bytes(&bytes).map_err(|e| {
        SvnError::new(
            "internal",
            "冲突标记解析失败",
            &e,
            "文件可能被外部工具修改，请重新打开三方合并。",
        )
    })?;
    let line_ending = match doc.line_ending {
        crate::svn::parser::LineEnding::Crlf => "\r\n",
        crate::svn::parser::LineEnding::Lf => "\n",
    };
    Ok(ConflictInfo {
        blocks: doc.ui_blocks(),
        has_markers: doc.has_markers,
        line_ending: line_ending.to_string(),
    })
}

/// 三方合并保存：按逐块选择组装写回 + svn resolve --accept working
/// choices 每项："mine" | "theirs" | "both" | "none"，顺序与 parse 返回的 blocks 一致
#[tauri::command]
pub fn wc_conflict_resolve(path: String, choices: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let bytes = std::fs::read(&path).map_err(|e| {
        SvnError::new(
            "io",
            "读取冲突文件失败",
            &e.to_string(),
            "文件可能已被删除，请使用「还原」恢复。",
        )
    })?;
    let doc = parse_conflict_bytes(&bytes).map_err(|e| {
        SvnError::new(
            "internal",
            "冲突标记解析失败",
            &e,
            "文件可能被外部工具修改，请重新打开三方合并。",
        )
    })?;
    if !doc.has_markers {
        return Err(SvnError::new(
            "io",
            "该文件没有文本冲突标记",
            "",
            "此冲突可能是属性冲突或二进制冲突，请使用「解决冲突」策略（mine-full / theirs-full / working / base）。",
        ));
    }
    if choices.len() != doc.ui_blocks().len() {
        return Err(SvnError::new(
            "io",
            "冲突块数量已变化",
            &format!("原 {} 块，现在 {} 块", choices.len(), doc.ui_blocks().len()),
            "文件可能在编辑期间被修改，请重新打开三方合并。",
        ));
    }
    let mut choice_list = Vec::with_capacity(choices.len());
    for c in &choices {
        let ch = match c.as_str() {
            "mine" => Choice::Mine,
            "theirs" => Choice::Theirs,
            "both" => Choice::Both,
            "none" => Choice::None,
            other => {
                return Err(SvnError::new(
                    "usage",
                    "无效的块选择",
                    other,
                    "只支持 mine / theirs / both / none。",
                ))
            }
        };
        choice_list.push(ch);
    }
    let out_bytes = assemble_conflict(&doc, &choice_list);
    std::fs::write(&path, &out_bytes).map_err(|e| {
        SvnError::new(
            "io",
            "写回合并结果失败",
            &e.to_string(),
            "请检查文件权限后重试。",
        )
    })?;
    // 内容已无冲突标记，svn resolve --accept working 将状态从 conflicted 清除
    let out = run_svn(
        &["resolve".into(), "--accept".into(), "working".into(), path],
        None,
        "解决冲突",
    )?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已按逐块选择解决 {} 个冲突块", choice_list.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 解决冲突（svn resolve --accept POLICY；策略：working/base/mine-full/theirs-full/...）
#[tauri::command]
pub fn wc_resolve(paths: Vec<String>, accept: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    if paths.is_empty() {
        return Err(SvnError::new(
            "io",
            "未选择要解决的路径",
            "",
            "请先勾选 conflicted 状态的路径。",
        ));
    }
    let mut args = vec!["resolve".into(), "--accept".into(), accept.clone()];
    args.extend(paths.clone());
    let out = run_svn(&args, None, "解决冲突")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已按「{}」策略解决 {} 个路径", accept, paths.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 清理工作副本（svn cleanup，修复中断操作留下的管理锁）
#[tauri::command]
pub fn wc_cleanup(path: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let out = run_svn(&["cleanup".into(), path], None, "清理")?;
    Ok(TaskResult {
        ok: true,
        summary: "清理完成".to_string(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 升级旧格式工作副本（svn upgrade）
#[tauri::command]
pub fn wc_upgrade(path: String) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let out = run_svn(&["upgrade".into(), path], None, "升级")?;
    Ok(TaskResult {
        ok: true,
        summary: "工作副本已升级".to_string(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// 还原本地修改（revert）
#[tauri::command]
pub fn wc_revert(paths: Vec<String>) -> Result<TaskResult, SvnError> {
    let _g = write_lock();
    let mut args = vec!["revert".into()];
    args.extend(paths.clone());
    let out = run_svn(&args, None, "还原")?;
    Ok(TaskResult {
        ok: true,
        summary: format!("已还原 {} 个路径", paths.len()),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

// —— 工具函数 ——

/// URL@PEG 构造：有 rev 时 peg 到该 rev，否则 URL 原样（HEAD）
fn peg_url(url: &str, rev: Option<i64>) -> String {
    match rev {
        Some(r) => format!("{url}@{r}"),
        None => url.to_string(),
    }
}

/// 提交说明写入 0600 临时文件
fn write_msg_file(message: &str) -> Result<std::path::PathBuf, SvnError> {
    let tmp = std::env::temp_dir().join(format!(
        "svn-commit-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)        .map_err(|e| SvnError::new("io", "无法创建提交说明临时文件", &e.to_string(), ""))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    f.write_all(message.as_bytes())
        .map_err(|e| SvnError::new("io", "写入提交说明临时文件失败", &e.to_string(), ""))?;
    Ok(tmp)
}

/// 补丁内容写临时文件（0600），供 svn patch 读取
fn write_patch_file(patch: &str) -> Result<std::path::PathBuf, SvnError> {
    let tmp = std::env::temp_dir().join(format!(
        "svn-patch-{}-{}.diff",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| SvnError::new("io", "无法创建补丁临时文件", &e.to_string(), ""))?;
    f.write_all(patch.as_bytes())
        .map_err(|e| SvnError::new("io", "写入补丁临时文件失败", &e.to_string(), ""))?;
    Ok(tmp)
}

/// 从 commit 输出提取 "Committed revision N."
fn extract_committed_revision(stdout: &str, stderr: &str) -> String {
    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(idx) = line.find("Committed revision") {
            let rest = line[idx + "Committed revision".len()..].trim_start();
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() {
                return format!("提交成功（revision {num}）");
            }
        }
    }
    "提交成功".to_string()
}

fn last_line(s: &str) -> String {
    s.lines().last().unwrap_or("").trim().to_string()
}

fn diff_result(out: &crate::svn::runner::SvnOutput) -> DiffResult {
    let text = utf8_escape(&out.stdout_bytes);
    // 二进制判定覆盖两种文案：unidiff 的 "(Binary files differ)" 与
    // 被 mime 标记时的 "Cannot display: file marked as a binary type."
    let is_binary = text.contains("(Binary files differ)")
        || text.contains("Cannot display: file marked as a binary type")
        || is_binary(&out.stdout_bytes);
    DiffResult {
        is_empty: text.trim().is_empty(),
        is_binary,
        unversioned: false,
        text,
    }
}

// ═══════════════════════════════════════════════════════════════════
// TaskManager（批次 8）：长任务异步化。
// - 前端通过 task_* 命令提交重操作，立即拿到任务 id，后台线程执行
// - 前端轮询 task_list 显示进度；task_cancel 置取消标志 → runner kill 子进程
// - 写任务在后台线程内拿 write_lock（不阻塞主进程事件循环）
// ═══════════════════════════════════════════════════════════════════

use crate::svn::task::{
    cancel_task as tm_cancel, create_task_with_retry, finish_task, get_retry, list_tasks, RetrySpec,
};
use crate::svn::task::TaskState;

/// 成功输出 → 结果摘要（取最后非空行，同 remote_export）
fn task_summary(out: &crate::svn::runner::SvnOutput) -> String {
    let stdout = utf8_escape(&out.stdout_bytes);
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("完成")
        .to_string()
}

/// 通用后台任务：`build` 生成 (args, cwd)，后台线程内以可取消方式执行并收尾。
/// `writable` 决定是否在后台线程获取写串行锁。返回任务 id。
fn spawn_svn_task(
    desc: String,
    writable: bool,
    retry: Option<RetrySpec>,
    build: impl FnOnce() -> Result<(Vec<String>, Option<PathBuf>), SvnError> + Send + 'static,
) -> Result<u64, SvnError> {
    let (id, cancel) = create_task_with_retry(desc, retry);
    std::thread::spawn(move || {
        let _g = writable.then(write_lock);
        let r = build().and_then(|(args, cwd)| {
            run_svn_cancellable_long(&args, cwd.as_deref(), "后台任务", &cancel)
        });
        let (state, output, result) = match r {
            Ok(out) => (TaskState::Done, String::new(), Some(task_summary(&out))),
            Err(e) if e.category == "cancelled" => (TaskState::Cancelled, String::new(), None),
            Err(e) => (
                TaskState::Failed,
                format!("{}：{}", e.summary, e.detail),
                None,
            ),
        };
        finish_task(id, state, output, result);
    });
    Ok(id)
}

/// 后台检出工作副本
#[tauri::command]
pub fn task_checkout(url: String, dest: String) -> Result<u64, SvnError> {
    let retry = Some(RetrySpec::Checkout { url: url.clone(), dest: dest.clone() });
    spawn_svn_task(
        format!("检出 {url}"),
        true,
        retry,
        move || Ok((vec!["checkout".into(), url, dest], None)),
    )
}

/// 后台更新工作副本
#[tauri::command]
pub fn task_update(path: String) -> Result<u64, SvnError> {
    let retry = Some(RetrySpec::Update { path: path.clone() });
    spawn_svn_task(
        format!("更新 {path}"),
        true,
        retry,
        move || Ok((vec!["update".into(), path], None)),
    )
}

/// 目录体检（导入前提示）：文件数 / 总大小 / 大文件(>5MB) / 垃圾文件(.DS_Store/*.pyc/*.err 等)
#[tauri::command]
pub fn dir_stats(path: String) -> Result<DirStats, SvnError> {
    dir_stats_inner(&path)
}

fn dir_stats_inner(path: &str) -> Result<DirStats, SvnError> {
    fn is_junk(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n == ".ds_store"
            || n == "thumbs.db"
            || n.ends_with(".pyc")
            || n.ends_with(".err")
            || n.ends_with(".tmp")
            || n.ends_with(".swp")
            || n.ends_with("~")
    }
    fn walk(
        dir: &Path,
        big: &mut Vec<String>,
        junk: &mut Vec<String>,
    ) -> std::io::Result<(usize, u64)> {
        let mut count = 0usize;
        let mut size = 0u64;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let p = entry.path();
            if ft.is_dir() {
                let (c, s) = walk(&p, big, junk)?;
                count += c;
                size += s;
            } else if ft.is_file() {
                count += 1;
                let s = entry.metadata()?.len();
                size += s;
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_junk(&name) {
                    junk.push(p.display().to_string());
                }
                if s > 5 * 1024 * 1024 {
                    big.push(format!("{} ({:.1} MB)", p.display(), s as f64 / 1024.0 / 1024.0));
                }
            }
        }
        Ok((count, size))
    }
    let root = Path::new(&path);
    if !root.is_dir() {
        return Err(SvnError::new("usage", "不是有效的目录", &path, "请选择要导入的本地目录。"));
    }
    let mut big_files = Vec::new();
    let mut junk_files = Vec::new();
    let (file_count, total_size) = walk(root, &mut big_files, &mut junk_files).map_err(|e| {
        SvnError::new("io", "统计目录失败", &e.to_string(), "请检查目录权限。")
    })?;
    Ok(DirStats {
        file_count,
        total_size,
        big_files,
        junk_files,
    })
}

/// 后台导入目录到仓库（-F 临时消息文件在后台线程创建/清理）
#[tauri::command]
pub fn task_import(local: String, url: String, message: String) -> Result<u64, SvnError> {
    let desc = format!("导入 {local} → {url}");
    let retry = Some(RetrySpec::Import {
        local: local.clone(),
        url: url.clone(),
        message: message.clone(),
    });
    let (id, cancel) = create_task_with_retry(desc, retry);
    std::thread::spawn(move || {
        let _g = write_lock();
        let r = (|| -> Result<crate::svn::runner::SvnOutput, SvnError> {
            let (mut args, tmp) = remote_msg_args(&message)?;
            let mut full = vec!["import".into()];
            full.append(&mut args);
            full.push(local.clone());
            full.push(url.clone());
            let out = run_svn_cancellable_long(&full, None, "导入", &cancel);
            let _ = std::fs::remove_file(&tmp);
            out
        })();
        let (state, output, result) = match r {
            Ok(out) => (TaskState::Done, String::new(), Some(task_summary(&out))),
            Err(e) if e.category == "cancelled" => (TaskState::Cancelled, String::new(), None),
            Err(e) => (
                TaskState::Failed,
                format!("{}：{}", e.summary, e.detail),
                None,
            ),
        };
        finish_task(id, state, output, result);
    });
    Ok(id)
}

/// 后台导出远程文件/目录到本地
#[tauri::command]
pub fn task_export(url: String, dest: String, rev: Option<i64>) -> Result<u64, SvnError> {
    if dest.trim().is_empty() {
        return Err(SvnError::new(
            "io",
            "目标路径为空",
            "",
            "请选择要保存到的本地路径。",
        ));
    }
    let desc = format!("导出 {url}");
    let retry = Some(RetrySpec::Export {
        url: url.clone(),
        dest: dest.clone(),
        rev,
    });
    spawn_svn_task(desc, false, retry, move || {
        let mut args = vec!["export".into(), "--force".into()];
        if let Some(r) = rev {
            args.push("-r".into());
            args.push(r.to_string());
        }
        args.push(peg_url(&url, rev));
        args.push(dest);
        Ok((args, None))
    })
}

/// 任务列表（含历史，新→旧）
#[tauri::command]
pub fn task_list() -> Vec<crate::svn::task::TaskInfo> {
    list_tasks()
}

/// 取消任务：仅 Running 任务可取消；已结束返回 false
#[tauri::command]
pub fn task_cancel(id: u64) -> bool {
    tm_cancel(id)
}

/// 重试失败/取消的任务：取原参数重新创建新任务，返回新任务 id
#[tauri::command]
pub fn task_retry(id: u64) -> Result<u64, SvnError> {
    let spec = get_retry(id).ok_or_else(|| {
        SvnError::new("usage", "无法重试该任务", "", "仅失败或已取消的任务可重试。")
    })?;
    match spec {
        RetrySpec::Import { local, url, message } => task_import(local, url, message),
        RetrySpec::Checkout { url, dest } => task_checkout(url, dest),
        RetrySpec::Update { path } => task_update(path),
        RetrySpec::Export { url, dest, rev } => task_export(url, dest, rev),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peg_url_works() {
        assert_eq!(peg_url("https://h/svn/trunk", None), "https://h/svn/trunk");
        assert_eq!(peg_url("https://h/svn/trunk", Some(5)), "https://h/svn/trunk@5");
    }

    #[test]
    fn extract_revision_works() {
        assert_eq!(
            extract_committed_revision("Committed revision 42.\n", ""),
            "提交成功（revision 42）"
        );
        assert_eq!(extract_committed_revision("", ""), "提交成功");
    }

    #[test]
    fn msg_file_perms() {
        let p = write_msg_file("测试提交说明").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let content = std::fs::read_to_string(&p).unwrap();
        assert_eq!(content, "测试提交说明");
        let _ = std::fs::remove_file(&p);
    }
}

#[cfg(test)]
mod diff_chunks_tests {
    use super::{diff_chunks, dir_stats_inner, line_offsets};
    use crate::svn::models::DiffChunk;

    fn pair(from_a: usize, to_a: usize, from_b: usize, to_b: usize) -> DiffChunk {
        DiffChunk {
            from_a,
            to_a,
            from_b,
            to_b,
        }
    }

    #[test]
    fn line_offsets_basic() {
        assert_eq!(line_offsets("a\nb\nc"), vec![0, 2, 4, 5]);
        assert_eq!(line_offsets(""), vec![0]);
        assert_eq!(line_offsets("abc"), vec![0, 3]);
    }

    #[test]
    fn replace_one_line() {
        // b → B：第 1 行替换（字符区间 [2,4)）
        let chunks = diff_chunks("a\nb\nc\n".into(), "a\nB\nc\n".into());
        assert_eq!(chunks, vec![pair(2, 4, 2, 4)]);
    }

    #[test]
    fn insert_line_middle() {
        let chunks = diff_chunks("a\nc\n".into(), "a\nb\nc\n".into());
        assert_eq!(chunks, vec![pair(2, 2, 2, 4)]);
    }

    #[test]
    fn delete_line_middle() {
        let chunks = diff_chunks("a\nb\nc\n".into(), "a\nc\n".into());
        assert_eq!(chunks, vec![pair(2, 4, 2, 2)]);
    }

    #[test]
    fn equal_text_no_chunks() {
        let chunks = diff_chunks("same\ncontent\n".into(), "same\ncontent\n".into());
        assert!(chunks.is_empty());
    }

    #[test]
    fn append_at_end() {
        // 末尾新增：fromB/toB 应落在新文本行起始
        let chunks = diff_chunks("a\n".into(), "a\nb\nc\n".into());
        assert_eq!(chunks, vec![pair(2, 2, 2, 6)]);
    }

    #[test]
    fn dir_stats_works() {
        let base = std::env::temp_dir().join(format!("svn-dirstats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.txt"), "hello").unwrap();
        std::fs::write(base.join(".DS_Store"), "junk").unwrap();
        std::fs::write(base.join("b.pyc"), "junk2").unwrap();
        std::fs::write(base.join("sub/c.bin"), vec![0u8; 6 * 1024 * 1024]).unwrap(); // >5MB
        let s = dir_stats_inner(&base.display().to_string()).unwrap();
        assert_eq!(s.file_count, 4);
        assert_eq!(s.total_size, 5 + 4 + 5 + 6 * 1024 * 1024);
        assert_eq!(s.big_files.len(), 1);
        assert!(s.big_files[0].contains("c.bin"));
        assert_eq!(s.junk_files.len(), 2);
        let err = dir_stats_inner(&base.join("nope").display().to_string()).unwrap_err();
        assert_eq!(err.category, "usage");
        let _ = std::fs::remove_dir_all(&base);
    }
}
