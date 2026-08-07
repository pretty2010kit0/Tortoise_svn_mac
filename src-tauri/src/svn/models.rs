//! 数据模型与 SVN 错误分类。
//! 错误分类原则（参考计划 8.3 修订）：先匹配 svn 错误码（E17xxxx 等稳定标识，不受 locale 影响），
//! 再匹配稳定输出模式；禁止使用 "conflict"/"lock" 等泛子串（路径名含这些字样会误分类）。

use serde::Serialize;

/// 统一错误结构（前端按 category 展示中文说明）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnError {
    /// 机器可读分类：auth / network / certificate / wc-locked / conflict / wc-too-old /
    /// not-working-copy / not-found / permission / wc-error / io / internal / unknown
    pub category: String,
    pub summary: String,
    pub detail: String,
    pub recovery_hint: String,
}

impl SvnError {
    pub fn new(category: &str, summary: &str, detail: &str, recovery_hint: &str) -> Self {
        Self {
            category: category.to_string(),
            summary: summary.to_string(),
            detail: detail.to_string(),
            recovery_hint: recovery_hint.to_string(),
        }
    }

    /// 根据 svn stderr 原文做分类（svn 错误码稳定于消息语言；
    /// 同时提取 E/W 前缀错误码，warning 用 W 前缀）
    pub fn from_svn(stderr: &str, command_desc: &str) -> Self {
        let e = stderr.trim();
        let codes = extract_codes(e);
        // W 前缀错误码（warning）与对应 E 码同义，统一匹配
        let has = |c: &str| {
            codes.iter().any(|x| x == c)
                || (c.starts_with('E') && codes.iter().any(|x| x == &format!("W{}", &c[1..])))
        };
        let (category, hint) = if has("E170001") {
            (
                "auth",
                "认证失败。请检查用户名/密码、访问权限，或先在终端用 svn 命令验证凭据。",
            )
        } else if has("E170106")
            || e.contains("SSL certificate")
            || e.contains("Server certificate")
            || e.contains("certificate has expired")
            || e.contains("certificate is not yet valid")
            || e.contains("hostname mismatch")
        {
            (
                "certificate",
                "服务器证书不受信任。请按应用设置中的“证书信任”指引处理（见计划 8.1），
                或先用命令行 svn 访问一次以接受证书。",
            )
        } else if has("E170013")
            || has("E170012")
            || has("E000111")
            || has("E000061")
            || has("E000002")
        {
            (
                "network",
                "无法连接服务器。请检查网络连接、仓库地址和代理配置。",
            )
        } else if has("E155004") {
            (
                "wc-locked",
                "工作副本被其他 SVN 进程锁定。请先执行 cleanup 清理管理锁。",
            )
        } else if has("E155037") {
            (
                "conflict",
                "操作与当前状态冲突。请先处理冲突或未完成的操作（update/merge/switch）。",
            )
        } else if has("E155036") {
            (
                "wc-too-old",
                "工作副本格式过旧。请先执行 upgrade 升级工作副本。",
            )
        } else if has("E155007") {
            (
                "not-working-copy",
                "该目录不是 SVN 工作副本（或不在工作副本内）。",
            )
        } else if has("E160013") || has("E160020") || has("E155010") {
            (
                "not-found",
                "路径在指定 revision 不存在。请检查路径拼写或 revision 是否正确。",
            )
        } else if has("E175002") || e.contains("Forbidden") || e.contains("Access denied") {
            (
                "permission",
                "没有访问权限。请联系仓库管理员确认账号权限。",
            )
        } else if has("E155000") || has("E200009") || has("E200012") {
            (
                "wc-error",
                "工作副本状态异常。可尝试 cleanup，必要时重做 checkout。",
            )
        } else if has("E205000") || has("E205001") || has("E200004") {
            (
                "usage",
                "参数或 URL 格式有误。请检查输入内容。",
            )
        } else {
            ("unknown", "")
        };
        Self {
            category: category.to_string(),
            summary: format!("{command_desc}失败"),
            detail: e.to_string(),
            recovery_hint: hint.to_string(),
        }
    }
}

/// 从 svn 输出中提取全部错误码（E/W + 6 位数字），按出现顺序去重保留
pub(crate) fn extract_codes(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut codes = Vec::new();
    let mut i = 0;
    while i + 7 <= b.len() {
        if (b[i] == b'E' || b[i] == b'W')
            && b[i + 1..i + 7].iter().all(|c| c.is_ascii_digit())
        {
            let code = s[i..i + 7].to_string();
            if !codes.contains(&code) {
                codes.push(code);
            }
            i += 7;
        } else {
            i += 1;
        }
    }
    codes
}

// —— 远程仓库模型 ——

/// 远程 URL 打开结果（svn info --xml）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    /// 仓库根 URL
    pub root_url: String,
    /// 当前目录 URL
    pub url: String,
    /// 相对仓库根路径（^/ 形式），缺失时为空
    pub relative_url: String,
    pub uuid: String,
    /// 该路径当前 revision（远程 URL 即 HEAD）
    pub revision: Option<i64>,
    /// 最近提交信息（远程 info 有效）
    pub last_author: String,
    pub last_date: String,
    /// 是否为工作副本目录（wc-info 存在时）
    pub is_wc: bool,
    pub wc_root: String,
    /// 打开时 list 预检得到的条目数
    pub entry_count: usize,
}

/// 目录条目（svn list --xml -v）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    pub name: String,
    /// file | dir
    pub kind: String,
    pub size: Option<i64>,
    pub revision: Option<i64>,
    pub author: String,
    pub date: String,
}

/// 变更路径（svn log -v）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePath {
    pub path: String,
    /// file | dir
    pub kind: String,
    /// A / D / M / R
    pub action: String,
    pub copyfrom_path: String,
    pub copyfrom_rev: Option<i64>,
}

/// 日志条目（svn log --xml -v）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub revision: i64,
    pub author: String,
    pub date: String,
    pub msg: String,
    pub changed_paths: Vec<ChangePath>,
}

/// 文件内容（svn cat）：原始字节以 base64 传输，前端按 encoding_hint 解码
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub data_base64: String,
    pub size: usize,
    /// true 表示整体为合法 UTF-8（前端可直接解码）
    pub is_utf8: bool,
    /// 是否判定为二进制（内容含 NUL）
    pub is_binary: bool,
}

/// 工作副本状态条目（svn status --xml）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub path: String,
    /// none/normal/add/delete/modified/conflicted/obstructed/missing/
    /// unversioned/external/incomplete/ignored
    pub item: String,
    /// none | modified（属性是否修改）
    pub props: String,
    /// 工作副本基准 revision
    pub wc_revision: Option<i64>,
    /// 最后提交 revision 与作者（commit 子元素）
    pub commit_revision: Option<i64>,
    pub commit_author: String,
    /// 本地是否被锁（needs-lock 属性未包含；锁信息需另查）
    pub wc_locked: bool,
    /// 文件锁信息（svn status --xml 的 <lock> 块；为空表示未锁定）
    #[serde(default)]
    pub lock_owner: String,
    #[serde(default)]
    pub lock_comment: String,
    /// status -u 时服务器端的条目状态（<repos-status>；空表示与服务器一致）
    #[serde(default)]
    pub repos_item: String,
}

/// svn blame 单行归属
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    pub line_no: usize,
    pub revision: i64,
    pub author: String,
    pub text: String,
}

/// svn status -u 结果：条目 + 服务器最新 revision
#[derive(Serialize, Clone, Debug)]
pub struct StatusU {
    pub entries: Vec<StatusEntry>,
    pub against: Option<i64>,
}

/// 属性条目（svn proplist -v --xml）
#[derive(Debug, Clone, Serialize)]
pub struct PropEntry {
    pub name: String,
    pub value: String,
}

/// 工作副本打开结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WcInfo {
    pub root_url: String,
    pub url: String,
    pub uuid: String,
    pub revision: Option<i64>,
    pub wc_root: String,
    pub schedule: String,
    pub depth: String,
    pub status: Vec<StatusEntry>,
}

/// Diff 结果：文本 diff（utf8_escape 兜底）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub text: String,
    /// 二进制或空（svn 无法生成文本 diff）
    pub is_binary: bool,
    pub is_empty: bool,
    /// 未版本化文件（无基线）：diff 内容为全新增
    #[serde(default)]
    pub unversioned: bool,
}

/// 并排 diff 用：文件两侧内容（BASE 基线 vs 当前工作区）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FilePair {
    pub old_text: String,
    pub new_text: String,
    pub is_binary: bool,
    pub is_unversioned: bool,
}

/// 变更块（对应 @codemirror/merge 的 Chunk）：字符位置的行区间
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffChunk {
    pub from_a: usize,
    pub to_a: usize,
    pub from_b: usize,
    pub to_b: usize,
}

/// 三方合并：单个冲突块（三栏展示用，行级文本）
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictBlock {
    pub mine: Vec<String>,
    pub base: Vec<String>,
    pub theirs: Vec<String>,
}

/// 三方合并：wc_conflict_parse 的返回
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    pub blocks: Vec<ConflictBlock>,
    pub has_markers: bool,
    /// "\n" 或 "\r\n"（展示用；组装由后端按原行尾处理）
    pub line_ending: String,
}

/// 认证缓存中的一条凭据（svn auth 输出解析）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuthCred {
    /// svn.simple（用户名密码） | svn.ssl.server（SSL 证书）| 其他
    pub kind: String,
    /// 认证领域（服务器地址 + 领域名）
    pub realm: String,
    pub username: String,
    pub subject: String,
    pub fingerprint: String,
    /// keychain / file / 空
    pub password_cache: String,
    /// 原始块文本（诊断用）
    pub raw: String,
}

/// 导入前目录体检
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DirStats {
    pub file_count: usize,
    pub total_size: u64,
    pub big_files: Vec<String>,
    pub junk_files: Vec<String>,
}

/// 仓库标准布局（分支/标签管理）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RepoLayout {
    pub trunk: Option<String>,
    pub branches_dir: Option<String>,
    pub tags_dir: Option<String>,
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

/// 合并信息（merged = 已合入，eligible = 可合入未合入）
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MergeInfo {
    pub merged: Vec<i64>,
    pub eligible: Vec<i64>,
}

/// 写操作结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub ok: bool,
    pub summary: String,
    pub stdout: String,
    pub stderr: String,
}
