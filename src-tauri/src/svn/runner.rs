//! Command Runner：以参数数组方式启动 svn，禁止 shell 拼接。
//! 关键约束（计划 4.2 修订）：
//! - svn 二进制发现：/opt/homebrew/bin/svn → /usr/bin/svn → PATH → 用户设置（暂未提供）
//! - 子进程 locale：LC_MESSAGES=C 保证输出语言稳定（英文错误消息），
//!   LC_CTYPE=UTF-8 保证中文 URL/路径参数正确编码（LC_ALL=C 会导致 E000022/双重编码）
//! - stdout/stderr 一律按原始字节接收，编码处理在解析层
//! - 保留完整输出供诊断；错误分类在 models::SvnError

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use crate::svn::models::SvnError;

/// svn 命令超时（对齐计划：只读任务可 kill）
pub const SVN_TIMEOUT: Duration = Duration::from_secs(120);
/// 批量/重操作（import/checkout/update/export）超时：30 分钟（大目录/大文件上传需要更长时间）
pub const SVN_TIMEOUT_LONG: Duration = Duration::from_secs(30 * 60);

/// svn 二进制全局配置（默认发现顺序，可通过设置命令覆盖）
static SVN_BIN: OnceLock<RwLock<PathBuf>> = OnceLock::new();

fn svn_bin_lock() -> &'static RwLock<PathBuf> {
    SVN_BIN.get_or_init(|| RwLock::new(discover_svn()))
}

/// 探测 svn 二进制路径（硬编码顺序 → PATH 回退）
fn discover_svn() -> PathBuf {
    let candidates = [
        PathBuf::from("/opt/homebrew/bin/svn"),
        PathBuf::from("/usr/bin/svn"),
    ];
    for c in candidates {
        if c.exists() {
            return c;
        }
    }
    PathBuf::from("svn") // 最后回退 PATH 查找
}

/// 当前 svn 二进制路径
pub fn svn_bin() -> PathBuf {
    svn_bin_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 设置 svn 二进制（需先验证可用性，见 commands::set_svn_executable）
pub fn set_svn_bin(path: PathBuf) {
    *svn_bin_lock().write().unwrap_or_else(|e| e.into_inner()) = path;
}

/// 将字节转为 String：合法 UTF-8 原样保留，非法字节转义为 `\xNN` 可见形式。
/// 避免 from_utf8_lossy 产生 �（U+FFFD）导致显示乱码。
pub fn utf8_escape(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    out.push_str(std::str::from_utf8(&rest[..valid]).unwrap());
                }
                let end = e.error_len().map(|len| valid + len).unwrap_or(rest.len());
                for &b in &rest[valid..end] {
                    out.push_str(&format!("\\x{b:02x}"));
                }
                rest = &rest[end..];
            }
        }
    }
    out
}

/// svn 命令执行结果（保留原始字节）
#[derive(Debug, Clone)]
pub struct SvnOutput {
    pub code: Option<i32>,
    pub stdout_bytes: Vec<u8>,
    pub stderr_bytes: Vec<u8>,
    pub stdout: String,
    pub stderr: String,
}

impl SvnOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// 执行 svn（指定二进制），不检查退出码；120 秒超时自动 kill（双线程读管道防阻塞）。
/// `cancel` 非空时：循环内检测到取消标志即 kill 子进程并返回 cancelled 错误。
fn run_svn_inner(
    bin: &Path,
    args: &[String],
    cwd: Option<&Path>,
    cancel: Option<&AtomicBool>,
    stdin_data: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<SvnOutput, SvnError> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    // 编码关键：svn 按 LC_CTYPE 解释命令行参数（URL/路径）。
    // LC_ALL=C 时非 ASCII 参数（中文 URL/路径）会转换失败（E000022）或双重编码。
    // 方案：删除 LC_ALL/LANG，显式 LC_MESSAGES=C（错误消息稳定英文，便于分类）
    // + LC_CTYPE=UTF-8（参数按 UTF-8 正确处理；macOS 均有 en_US.UTF-8 locale）。
    cmd.env_remove("LC_ALL");
    cmd.env_remove("LANG");
    cmd.env("LC_MESSAGES", "C");
    cmd.env("LC_CTYPE", "en_US.UTF-8");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if stdin_data.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            SvnError::new(
                "io",
                "无法启动 svn 进程",
                &format!("尝试执行 {}：{e}", bin.display()),
                "请确认已安装 subversion（brew install subversion），并在设置中检查 svn 路径。",
            )
        })?;
    let mut stdout = child.stdout.take().expect("stdout 管道");
    let mut stderr = child.stderr.take().expect("stderr 管道");
    // 写入 stdin（密码等）；svn 提前退出时写失败可忽略（EPIPE）
    if let Some(data) = stdin_data {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(&data);
            drop(si); // 关闭 stdin，svn 读到 EOF
        }
    }
    let stdout_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stdout.read_to_end(&mut v);
        v
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(SvnError::new(
                    "timeout",
                    "svn 命令执行超时",
                    &format!("超过 {} 秒：{}", timeout.as_secs(), args.join(" ")),
                    "请重试；浏览大目录/大日志时请缩小范围。",
                ));
            }
            Ok(None) => {
                if let Some(c) = cancel {
                    if c.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(SvnError::new(
                            "cancelled",
                            "任务已取消",
                            &format!("已终止命令：{}", args.join(" ")),
                            "可在任务栏重新发起。",
                        ));
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(SvnError::new(
                    "io",
                    "svn 进程异常",
                    &e.to_string(),
                    "请重试。",
                ))
            }
        }
    };
    let stdout_bytes = stdout_reader.join().unwrap_or_default();
    let stderr_bytes = stderr_reader.join().unwrap_or_default();
    let stdout = utf8_escape(&stdout_bytes);
    let stderr = utf8_escape(&stderr_bytes);
    Ok(SvnOutput {
        code: status.code(),
        stdout_bytes,
        stderr_bytes,
        stdout,
        stderr,
    })
}

/// 执行 svn（指定二进制），不检查退出码
pub fn run_svn_any_with_bin(
    bin: &Path,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<SvnOutput, SvnError> {
    run_svn_inner(bin, args, cwd, None, None, SVN_TIMEOUT)
}

/// 执行 svn（当前配置的二进制），不检查退出码
pub fn run_svn_any(args: &[String], cwd: Option<&Path>) -> Result<SvnOutput, SvnError> {
    run_svn_inner(&svn_bin(), args, cwd, None, None, SVN_TIMEOUT)
}

/// 可取消执行 svn：检测到取消标志立即 kill 子进程（用于 TaskManager 后台任务）
pub fn run_svn_cancellable(
    args: &[String],
    cwd: Option<&Path>,
    desc: &str,
    cancel: &AtomicBool,
) -> Result<SvnOutput, SvnError> {
    let bin = svn_bin();
    match run_svn_inner(&bin, args, cwd, Some(cancel), None, SVN_TIMEOUT) {
        Ok(out) if !out.success() => Err(SvnError::from_svn(&out.stderr, desc)),
        r => r,
    }
}

/// 可取消执行 svn（长任务超时 30 分钟）：用于 import/checkout/update/export 等批量操作
pub fn run_svn_cancellable_long(
    args: &[String],
    cwd: Option<&Path>,
    desc: &str,
    cancel: &AtomicBool,
) -> Result<SvnOutput, SvnError> {
    let bin = svn_bin();
    match run_svn_inner(&bin, args, cwd, Some(cancel), None, SVN_TIMEOUT_LONG) {
        Ok(out) if !out.success() => Err(SvnError::from_svn(&out.stderr, desc)),
        r => r,
    }
}

/// 执行 svn，退出码非零时按 stderr 分类报错
pub fn run_svn(args: &[String], cwd: Option<&Path>, desc: &str) -> Result<SvnOutput, SvnError> {
    let out = run_svn_any(args, cwd)?;
    if !out.success() {
        return Err(SvnError::from_svn(&out.stderr, desc));
    }
    Ok(out)
}

/// 带用户名/密码执行 svn：`--non-interactive --username U --password-from-stdin`，
/// 密码仅写入子进程 stdin（不进命令行参数、URL、日志）。
pub fn run_svn_auth(
    args: &[String],
    cwd: Option<&Path>,
    desc: &str,
    username: &str,
    password: &str,
) -> Result<SvnOutput, SvnError> {
    let mut full = Vec::with_capacity(args.len() + 4);
    full.push("--non-interactive".to_string());
    full.push("--username".to_string());
    full.push(username.to_string());
    full.push("--password-from-stdin".to_string());
    full.extend(args.iter().cloned());
    let out = run_svn_inner(&svn_bin(), &full, cwd, None, Some(password.as_bytes().to_vec()), SVN_TIMEOUT)?;
    if !out.success() {
        return Err(SvnError::from_svn(&out.stderr, desc));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svn_version_works() {
        let out = run_svn_any(&["--version".into(), "--quiet".into()], None).unwrap();
        assert!(out.success());
        assert!(out.stdout.starts_with("1."));
    }

    #[test]
    fn svn_bin_found() {
        let bin = svn_bin();
        assert!(bin.exists() || bin.to_string_lossy() == "svn");
    }

    #[test]
    fn utf8_escape_keeps_valid_and_escapes_invalid() {
        assert_eq!(utf8_escape("中文 ab".as_bytes()), "中文 ab");
        // "坚" = E5 9D 9A；切掉首字节后非法，应转义为 \x9d\x9a
        assert_eq!(utf8_escape(b"\xe5\x9d\x9a\x9d\x9a"), "坚\\x9d\\x9a");
        assert_eq!(utf8_escape(&[0xff]), "\\xff");
        assert_eq!(utf8_escape(b""), "");
    }

    #[test]
    fn error_classify_by_code() {
        let e = SvnError::from_svn("svn: E155004: Working copy 'x' locked", "更新");
        assert_eq!(e.category, "wc-locked");
        let e = SvnError::from_svn("svn: E170001: Authentication failed", "连接");
        assert_eq!(e.category, "auth");
        // 路径名含 lock/conflict 字样不应误分类
        let e = SvnError::from_svn("svn: E155010: '/repo/locked_dir' not found", "浏览");
        assert_eq!(e.category, "not-found");
        // warning 用 W 前缀，也应参与分类（W160013 + 包装错误 E200009）
        let e = SvnError::from_svn(
            "svn: warning: W160013: '/work/x' path not found\nsvn: E200009: some targets don't exist",
            "读取",
        );
        assert_eq!(e.category, "not-found", "detail: {}", e.detail);
        // E200009 单独出现（无具体错误码）归 wc-error
        let e = SvnError::from_svn("svn: E200009: Could not cat all targets", "读取");
        assert_eq!(e.category, "wc-error");
    }

    #[test]
    fn extract_codes_works() {
        use crate::svn::models::extract_codes;
        let s = "svn: warning: W160013: a\nsvn: E200009: b\nE160013 again";
        let codes = extract_codes(s);
        assert_eq!(codes, vec!["W160013", "E200009", "E160013"]);
        assert!(extract_codes("no codes here 12345").is_empty());
    }
}
