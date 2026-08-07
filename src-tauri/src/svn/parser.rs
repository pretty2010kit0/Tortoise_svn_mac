//! SVN XML 输出解析器（quick-xml 事件 API）。
//! 设计要点（计划阶段 B 修订）：
//! - 输入为原始字节，先按 UTF-8 解码（XML 声明保证）；解析失败时返回 Err，
//!   调用方保留原始输出供诊断与降级展示
//! - 结构容错：未知元素跳过，缺失字段取默认值，不因单个坏字段整体失败

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::svn::models::{
    BlameLine, ChangePath, ListEntry, LogEntry, PropEntry, RepoInfo, StatusEntry, WcInfo,
};
use crate::svn::runner::utf8_escape;

/// 从 XML 原始字节解析；失败返回包含原始输出的错误描述
fn reader_from(bytes: &[u8]) -> Result<Reader<&[u8]>, String> {
    let mut r = Reader::from_reader(bytes);
    r.config_mut().trim_text(true);
    Ok(r)
}

fn attr<'a>(e: &BytesStart<'a>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name.as_bytes())
        .map(|a| utf8_escape(&a.value))
}

// —— svn info --xml ——

/// 解析 svn info --xml（远程 URL 或工作副本目录）
pub fn parse_info(xml: &[u8]) -> Result<RepoInfo, String> {
    let mut r = reader_from(xml)?;
    let mut info = RepoInfo {
        root_url: String::new(),
        url: String::new(),
        relative_url: String::new(),
        uuid: String::new(),
        revision: None,
        last_author: String::new(),
        last_date: String::new(),
        is_wc: false,
        wc_root: String::new(),
        entry_count: 0,
    };
    // 元素栈：记录当前嵌套路径，用于把文本挂到正确字段
    let mut stack: Vec<String> = Vec::new();
    let mut in_commit = false;
    let mut in_wcinfo = false;

    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "entry" => {
                        info.revision = attr(&e, "revision").and_then(|s| s.parse().ok());
                    }
                    "commit" => {
                        in_commit = true;
                        if let Some(rev) = attr(&e, "revision") {
                            info.revision = rev.parse().ok().or(info.revision);
                        }
                    }
                    "wc-info" => {
                        in_wcinfo = true;
                        info.is_wc = true;
                    }
                    _ => {}
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                // list 条目里的空 commit 等无需处理
                let _ = e;
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().into_owned();
                let cur = stack.last().map(|s| s.as_str()).unwrap_or("");
                match cur {
                    "root" if stack.len() >= 2 => info.root_url = text,
                    "uuid" => info.uuid = text,
                    "url" => info.url = text,
                    "relative-url" => info.relative_url = text,
                    "wcroot-abspath" if in_wcinfo => info.wc_root = text,
                    "author" if in_commit => info.last_author = text,
                    "date" if in_commit => info.last_date = text,
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "commit" => in_commit = false,
                    "wc-info" => in_wcinfo = false,
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("info XML 解析失败：{e}\n原始输出：\n{}", utf8_escape(xml))),
            _ => {}
        }
    }
    Ok(info)
}

// —— svn list --xml -v ——

pub fn parse_list(xml: &[u8]) -> Result<Vec<ListEntry>, String> {
    let mut r = reader_from(xml)?;
    let mut entries: Vec<ListEntry> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    // 当前正在收集的条目
    let mut cur: Option<ListEntry> = None;
    let mut in_commit = false;

    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "entry" => {
                        cur = Some(ListEntry {
                            name: String::new(),
                            kind: attr(&e, "kind").unwrap_or_default(),
                            size: None,
                            revision: None,
                            author: String::new(),
                            date: String::new(),
                        });
                    }
                    "commit" => {
                        in_commit = true;
                        if let Some(rev) = attr(&e, "revision") {
                            if let Some(c) = cur.as_mut() {
                                c.revision = rev.parse().ok();
                            }
                        }
                    }
                    _ => {}
                }
                stack.push(name);
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().into_owned();
                let cur_name = stack.last().map(|s| s.as_str()).unwrap_or("");
                if let Some(c) = cur.as_mut() {
                    match cur_name {
                        "name" => c.name = text,
                        "size" => c.size = text.parse().ok(),
                        "author" if in_commit => c.author = text,
                        "date" if in_commit => c.date = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "commit" => in_commit = false,
                    "entry" => {
                        if let Some(c) = cur.take() {
                            entries.push(c);
                        }
                    }
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!("list XML 解析失败：{e}\n原始输出：\n{}", utf8_escape(xml)))
            }
            _ => {}
        }
    }
    Ok(entries)
}

// —— svn log --xml -v ——

pub fn parse_log(xml: &[u8]) -> Result<Vec<LogEntry>, String> {
    let mut r = reader_from(xml)?;
    let mut entries: Vec<LogEntry> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cur: Option<LogEntry> = None;
    let mut cur_path: Option<ChangePath> = None;

    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "logentry" => {
                        let rev = attr(&e, "revision").and_then(|s| s.parse().ok());
                        cur = Some(LogEntry {
                            revision: rev.unwrap_or(0),
                            author: String::new(),
                            date: String::new(),
                            msg: String::new(),
                            changed_paths: Vec::new(),
                        });
                    }
                    "path" => {
                        cur_path = Some(ChangePath {
                            path: String::new(),
                            kind: attr(&e, "kind").unwrap_or_default(),
                            action: attr(&e, "action").unwrap_or_default(),
                            copyfrom_path: attr(&e, "copyfrom-path").unwrap_or_default(),
                            copyfrom_rev: attr(&e, "copyfrom-rev").and_then(|s| s.parse().ok()),
                        });
                    }
                    _ => {}
                }
                stack.push(name);
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().into_owned();
                let cur_name = stack.last().map(|s| s.as_str()).unwrap_or("");
                if let Some(p) = cur_path.as_mut() {
                    if cur_name == "path" {
                        p.path = text;
                        continue;
                    }
                }
                if let Some(c) = cur.as_mut() {
                    match cur_name {
                        "author" => c.author = text,
                        "date" => c.date = text,
                        "msg" => c.msg = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "path" => {
                        if let (Some(p), Some(c)) = (cur_path.take(), cur.as_mut()) {
                            c.changed_paths.push(p);
                        }
                    }
                    "logentry" => {
                        if let Some(c) = cur.take() {
                            entries.push(c);
                        }
                    }
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!("log XML 解析失败：{e}\n原始输出：\n{}", utf8_escape(xml)))
            }
            _ => {}
        }
    }
    Ok(entries)
}

// —— svn status --xml ——

pub fn parse_status(xml: &[u8]) -> Result<(Vec<StatusEntry>, Option<i64>), String> {
    let mut r = reader_from(xml)?;
    let mut entries: Vec<StatusEntry> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cur: Option<StatusEntry> = None;
    let mut in_commit = false;
    let mut against: Option<i64> = None;

    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "entry" => {
                        cur = Some(StatusEntry {
                            path: attr(&e, "path").unwrap_or_default(),
                            item: String::new(),
                            props: String::new(),
                            wc_revision: None,
                            commit_revision: None,
                            commit_author: String::new(),
                            wc_locked: false,
                            lock_owner: String::new(),
                            lock_comment: String::new(),
                            repos_item: String::new(),
                        });
                    }
                    "wc-status" => read_wc_status(&e, cur.as_mut()),
                    "repos-status" => {
                        if let Some(c) = cur.as_mut() {
                            c.repos_item = attr(&e, "item").unwrap_or_default();
                        }
                    }
                    "commit" => {
                        in_commit = true;
                        if let Some(c) = cur.as_mut() {
                            c.commit_revision =
                                attr(&e, "revision").and_then(|s| s.parse().ok());
                        }
                    }
                    _ => {}
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                // 自闭合 <wc-status item="unversioned" .../> 也要读属性
                if name == "wc-status" {
                    read_wc_status(&e, cur.as_mut());
                }
                // status -u 的 <against revision="N"/>：服务器最新 revision
                if name == "against" {
                    against = attr(&e, "revision").and_then(|s| s.parse().ok());
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().into_owned();
                let cur_el = stack.last().map(|s| s.as_str()).unwrap_or("");
                if in_commit && cur_el == "author" {
                    if let Some(c) = cur.as_mut() {
                        c.commit_author = text.clone();
                    }
                }
                // <lock> 块内的 owner / comment（文件锁信息）
                if let Some(c) = cur.as_mut() {
                    match cur_el {
                        "owner" => c.lock_owner = text,
                        "comment" => c.lock_comment = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "commit" => in_commit = false,
                    "entry" => {
                        if let Some(c) = cur.take() {
                            entries.push(c);
                        }
                    }
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(format!(
                    "status XML 解析失败：{e}\n原始输出：\n{}",
                    utf8_escape(xml)
                ))
            }
            _ => {}
        }
    }
    Ok((entries, against))
}

/// 解析 `svn blame` 纯文本输出：`r<rev> <author> <text>`（按行）
pub fn parse_blame(text: &[u8]) -> Result<Vec<BlameLine>, String> {
    let s = String::from_utf8_lossy(text);
    let mut out: Vec<BlameLine> = Vec::new();
    for (i, raw) in s.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("svn:") {
            continue;
        }
        let mut it = line.split_whitespace();
        let rev_tok = it.next().ok_or_else(|| format!("无法解析 blame 行：{line}"))?;
        let rev_tok = match rev_tok.strip_prefix('r') {
            Some(r) => r,
            None => rev_tok,
        };
        let revision: i64 = rev_tok
            .parse()
            .map_err(|_| format!("无法解析 blame revision：{line}"))?;
        let author = it.next().unwrap_or("").to_string();
        let text = it.collect::<Vec<_>>().join(" ");
        out.push(BlameLine {
            line_no: i + 1,
            revision,
            author,
            text,
        });
    }
    Ok(out)
}

/// 从 <wc-status> 元素读取属性（Start 与 Empty 事件共用）
fn read_wc_status(e: &BytesStart<'_>, cur: Option<&mut StatusEntry>) {
    if let Some(c) = cur {
        c.item = attr(e, "item").unwrap_or_default();
        c.props = attr(e, "props").unwrap_or_default();
        c.wc_revision = attr(e, "revision").and_then(|s| s.parse().ok());
        c.wc_locked = attr(e, "wc-locked").as_deref() == Some("true");
    }
}

/// 解析 svn proplist -v --xml：properties > target > property{name}>text</property>
pub fn parse_proplist(xml: &[u8]) -> Result<Vec<PropEntry>, String> {
    let mut r = reader_from(xml)?;
    let mut props = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_value = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "property" {
                    cur_name = attr(&e, "name");
                    cur_value = String::new();
                }
            }
            Ok(Event::Text(t)) => {
                if cur_name.is_some() {
                    cur_value.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "property" {
                    if let Some(n) = cur_name.take() {
                        props.push(PropEntry {
                            name: n,
                            value: cur_value.trim_end_matches('\n').to_string(),
                        });
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "property" {
                    props.push(PropEntry {
                        name: attr(&e, "name").unwrap_or_default(),
                        value: String::new(),
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("属性 XML 解析失败：{e}")),
            _ => {}
        }
    }
    Ok(props)
}

// —— WcInfo 组装（info + status）——

pub fn assemble_wc(info: RepoInfo, status: Vec<StatusEntry>) -> WcInfo {
    WcInfo {
        root_url: info.root_url,
        url: info.url,
        uuid: info.uuid,
        revision: info.revision,
        wc_root: info.wc_root,
        schedule: String::new(),
        depth: String::new(),
        status,
    }
}

/// 简单字节级二进制判定（内容含 NUL 视为二进制）
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0u8)
}

/// 还原 svn 在非 UTF-8 locale 下的 `{U+XXXX}` 转义（如 {U+8BA4} → 认）
pub fn unescape_u_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 3 <= bytes.len() && bytes[i + 1] == b'U' && bytes[i + 2] == b'+' {
            // 收集 {U+ 后的十六进制直到 }
            let mut j = i + 3;
            let mut hex = String::new();
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() && hex.len() < 6 {
                hex.push(bytes[j] as char);
                j += 1;
            }
            if !hex.is_empty() && j < bytes.len() && bytes[j] == b'}' {
                if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(cp) {
                        out.push(c);
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// 解析 `svn auth` 输出（LC_MESSAGES=C 英文键名，`---` 分隔块）
pub fn parse_auth_creds(text: &str) -> Vec<crate::svn::models::AuthCred> {
    use crate::svn::models::AuthCred;
    let mut creds = Vec::new();
    let mut cur: Option<AuthCred> = None;
    let mut raw_lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = unescape_u_escapes(line);
        if line.trim().chars().all(|c| c == '-') && line.trim().len() >= 10 {
            // 分隔线：收尾当前块
            if let Some(mut c) = cur.take() {
                c.raw = raw_lines.join("\n");
                creds.push(c);
            }
            raw_lines.clear();
            continue;
        }
        raw_lines.push(line.clone());
        if let Some((key, value)) = line.split_once(':') {
            let v = value.trim().to_string();
            match key.trim() {
                "Credential kind" => {
                    // 新块开始（若上一块未收尾，先收）
                    if let Some(mut c) = cur.take() {
                        c.raw = raw_lines.join("\n");
                        creds.push(c);
                    }
                    cur = Some(AuthCred {
                        kind: v,
                        realm: String::new(),
                        username: String::new(),
                        subject: String::new(),
                        fingerprint: String::new(),
                        password_cache: String::new(),
                        raw: String::new(),
                    });
                }
                "Authentication realm" => {
                    if let Some(c) = cur.as_mut() {
                        c.realm = v;
                    }
                }
                "Password cache" => {
                    if let Some(c) = cur.as_mut() {
                        c.password_cache = v;
                    }
                }
                "Username" => {
                    if let Some(c) = cur.as_mut() {
                        c.username = v;
                    }
                }
                "Subject" => {
                    if let Some(c) = cur.as_mut() {
                        c.subject = v;
                    }
                }
                "Fingerprint" => {
                    if let Some(c) = cur.as_mut() {
                        c.fingerprint = v;
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(mut c) = cur.take() {
        c.raw = raw_lines.join("\n");
        creds.push(c);
    }
    creds
}

// —— 三方合并冲突解析（批次 9）——
//
// svn 冲突标记格式（本机 svn 1.14.5 实测）：
//   <<<<<<< .mine      ← 本地工作副本修改
//   （本地行）
//   ||||||| .r3        ← base 共同祖先段（后缀为 revision，可能缺失）
//   （base 行）
//   =======
//   （远端行）
//   >>>>>>> .r4        ← 远端/服务器版本
// 标记行 = 7 个 < | | = > + 空格 + 标签；解析只认前缀，忽略后缀标签。
// 防御：======= 仅在已开块内识别（markdown 分隔线不误判）；未闭合块报错。

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineEnding {
    Lf,
    Crlf,
}

/// 逐块选择（前端传 "mine" | "theirs" | "both" | "none"）
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Choice {
    Mine,
    Theirs,
    Both,
    None,
}

pub struct ConflictBlockBytes {
    pub mine: Vec<Vec<u8>>,
    pub base: Vec<Vec<u8>>,
    pub theirs: Vec<Vec<u8>>,
}

pub enum ConflictPart {
    /// 块外的普通行（原样保留）
    Context(Vec<Vec<u8>>),
    Block(ConflictBlockBytes),
}

pub struct ConflictDoc {
    pub parts: Vec<ConflictPart>,
    pub has_markers: bool,
    pub line_ending: LineEnding,
    pub trailing_newline: bool,
}

impl std::fmt::Debug for ConflictDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConflictDoc")
            .field("parts", &self.parts.len())
            .field("has_markers", &self.has_markers)
            .field("line_ending", &self.line_ending)
            .field("trailing_newline", &self.trailing_newline)
            .finish()
    }
}

impl ConflictDoc {
    /// 块列表（供前端展示，非 UTF-8 字节容错替换）
    pub fn ui_blocks(&self) -> Vec<crate::svn::models::ConflictBlock> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                ConflictPart::Context(_) => None,
                ConflictPart::Block(b) => Some(crate::svn::models::ConflictBlock {
                    mine: b.mine.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect(),
                    base: b.base.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect(),
                    theirs: b.theirs.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect(),
                }),
            })
            .collect()
    }
}

fn is_marker(line: &[u8], prefix: &[u8]) -> bool {
    line.starts_with(prefix)
        && (line.len() == prefix.len() || line[prefix.len()] == b' ')
}

const MINE_OPEN: &[u8] = b"<<<<<<<";
const BASE_SEP: &[u8] = b"|||||||";
const MID_SEP: &[u8] = b"=======";
const THEIRS_CLOSE: &[u8] = b">>>>>>>";

/// 解析冲突文件字节。行按 \n 分割（去掉 \r），保留块/上下文结构与行尾信息。
/// 无任何冲突标记时返回 has_markers=false 的空文档（不报错）。
pub fn parse_conflict_bytes(raw: &[u8]) -> Result<ConflictDoc, String> {
    // 1. 行尾检测：出现 \r\n 即视为 CRLF（写回时还原）
    let line_ending = if raw.windows(2).any(|w| w == b"\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    // 2. 按 \n 切行，去尾部 \r；末尾换行产生的空串不作为内容行
    let mut raw_lines: Vec<Vec<u8>> = raw
        .split(|&b| b == b'\n')
        .map(|l| {
            let mut v = l.to_vec();
            if v.last() == Some(&b'\r') {
                v.pop();
            }
            v
        })
        .collect();
    if raw_lines.last().map(|l| l.is_empty()) == Some(true) && raw.ends_with(b"\n") {
        raw_lines.pop();
    }
    if raw.is_empty() {
        raw_lines.clear();
    }
    let trailing_newline = raw.ends_with(b"\n");

    // 3. 状态机
    #[derive(PartialEq)]
    enum State {
        Context,
        Mine,
        Base,
        Theirs,
    }
    let mut state = State::Context;
    let mut parts: Vec<ConflictPart> = Vec::new();
    let mut context: Vec<Vec<u8>> = Vec::new();
    let mut cur_mine: Vec<Vec<u8>> = Vec::new();
    let mut cur_base: Vec<Vec<u8>> = Vec::new();
    let mut cur_theirs: Vec<Vec<u8>> = Vec::new();
    let mut blocks = 0usize;

    let close_block = |parts: &mut Vec<ConflictPart>,
                       context: &mut Vec<Vec<u8>>,
                       mine: Vec<Vec<u8>>,
                       base: Vec<Vec<u8>>,
                       theirs: Vec<Vec<u8>>| {
        if !context.is_empty() {
            parts.push(ConflictPart::Context(std::mem::take(context)));
        }
        parts.push(ConflictPart::Block(ConflictBlockBytes { mine, base, theirs }));
    };

    for line in &raw_lines {
        match state {
            State::Context => {
                if is_marker(line, MINE_OPEN) {
                    state = State::Mine;
                    cur_mine.clear();
                    cur_base.clear();
                    cur_theirs.clear();
                } else {
                    context.push(line.clone());
                }
            }
            State::Mine => {
                if is_marker(line, BASE_SEP) {
                    state = State::Base;
                } else if is_marker(line, MID_SEP) {
                    state = State::Theirs;
                } else if is_marker(line, THEIRS_CLOSE) {
                    // 无 base/theirs 的异常块：按空 theirs 收尾（防御）
                    close_block(&mut parts, &mut context, std::mem::take(&mut cur_mine), std::mem::take(&mut cur_base), std::mem::take(&mut cur_theirs));
                    blocks += 1;
                    state = State::Context;
                } else {
                    cur_mine.push(line.clone());
                }
            }
            State::Base => {
                if is_marker(line, MID_SEP) {
                    state = State::Theirs;
                } else if is_marker(line, THEIRS_CLOSE) {
                    close_block(&mut parts, &mut context, std::mem::take(&mut cur_mine), std::mem::take(&mut cur_base), std::mem::take(&mut cur_theirs));
                    blocks += 1;
                    state = State::Context;
                } else {
                    cur_base.push(line.clone());
                }
            }
            State::Theirs => {
                if is_marker(line, THEIRS_CLOSE) {
                    close_block(&mut parts, &mut context, std::mem::take(&mut cur_mine), std::mem::take(&mut cur_base), std::mem::take(&mut cur_theirs));
                    blocks += 1;
                    state = State::Context;
                } else {
                    cur_theirs.push(line.clone());
                }
            }
        }
    }
    if state != State::Context {
        return Err("冲突标记未闭合：文件以冲突块结尾，缺少 >>>>>>> 结束行（文件可能被外部工具损坏）".into());
    }
    if !context.is_empty() {
        parts.push(ConflictPart::Context(context));
    }
    Ok(ConflictDoc {
        parts,
        has_markers: blocks > 0,
        line_ending,
        trailing_newline,
    })
}

/// 按逐块选择组装结果（上下文行原样保留；块按选择输出；按原行尾还原）
pub fn assemble_conflict(doc: &ConflictDoc, choices: &[Choice]) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut bi = 0usize;
    for part in &doc.parts {
        match part {
            ConflictPart::Context(ls) => lines.extend(ls.iter().cloned()),
            ConflictPart::Block(b) => {
                let ch = choices.get(bi).copied().unwrap_or(Choice::Mine);
                bi += 1;
                match ch {
                    Choice::Mine => lines.extend(b.mine.iter().cloned()),
                    Choice::Theirs => lines.extend(b.theirs.iter().cloned()),
                    Choice::Both => {
                        lines.extend(b.mine.iter().cloned());
                        lines.extend(b.theirs.iter().cloned());
                    }
                    Choice::None => {}
                }
            }
        }
    }
    let sep: &[u8] = match doc.line_ending {
        LineEnding::Lf => b"\n",
        LineEnding::Crlf => b"\r\n",
    };
    let mut out: Vec<u8> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        out.extend_from_slice(l);
    }
    if doc.trailing_newline && !lines.is_empty() {
        out.extend_from_slice(sep);
    }
    out
}

// —— 单元测试 ——

#[cfg(test)]
mod tests {
    use super::*;

    const INFO_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<info>
<entry kind="dir" path="." revision="9">
<url>https://example.com/svn/trunk</url>
<relative-url>^/trunk</relative-url>
<repository><root>https://example.com/svn</root><uuid>aaaa-bbbb</uuid></repository>
<wc-info><wcroot-abspath>/Users/u/wc</wcroot-abspath><schedule>normal</schedule><depth>infinity</depth></wc-info>
<commit revision="9"><author>zhou</author><date>2026-01-02T03:04:05.000000Z</date></commit>
</entry>
</info>"#;

    const LIST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<lists>
<list path="/trunk">
<entry kind="dir"><name>src</name><commit revision="8"><author>zhou</author><date>2026-01-01T00:00:00.000000Z</date></commit></entry>
<entry kind="file"><name>a.txt</name><size>123</size><commit revision="9"><author>li</author><date>2026-01-02T00:00:00.000000Z</date></commit></entry>
</list>
</lists>"#;

    const LOG_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<log>
<logentry revision="9">
<author>zhou</author><date>2026-01-02T00:00:00.000000Z</date>
<msg>修复问题</msg>
<paths>
<path kind="file" action="M">/trunk/a.txt</path>
<path kind="dir" action="A" copyfrom-path="/branches/v1" copyfrom-rev="7">/trunk</path>
</paths>
</logentry>
<logentry revision="8"><author>li</author><date>2026-01-01T00:00:00.000000Z</date><msg/></logentry>
</log>"#;

    const STATUS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target path="/Users/u/wc">
<entry path="/Users/u/wc/a.txt">
<wc-status item="modified" props="none" revision="9">
<commit revision="8"><author>li</author></commit>
</wc-status>
</entry>
<entry path="/Users/u/wc/new.txt">
<wc-status item="unversioned" props="none" revision="0"/>
</entry>
</target>
</status>"#;

    #[test]
    fn parse_info_works() {
        let info = parse_info(INFO_XML.as_bytes()).unwrap();
        assert_eq!(info.root_url, "https://example.com/svn");
        assert_eq!(info.url, "https://example.com/svn/trunk");
        assert_eq!(info.relative_url, "^/trunk");
        assert_eq!(info.uuid, "aaaa-bbbb");
        assert_eq!(info.revision, Some(9));
        assert_eq!(info.last_author, "testuser");
        assert!(info.is_wc);
        assert_eq!(info.wc_root, "/Users/u/wc");
    }

    #[test]
    fn parse_list_works() {
        let entries = parse_list(LIST_XML.as_bytes()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "src");
        assert_eq!(entries[0].kind, "dir");
        assert_eq!(entries[0].revision, Some(8));
        assert_eq!(entries[1].name, "a.txt");
        assert_eq!(entries[1].size, Some(123));
        assert_eq!(entries[1].author, "li");
    }

    #[test]
    fn parse_log_works() {
        let logs = parse_log(LOG_XML.as_bytes()).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].revision, 9);
        assert_eq!(logs[0].msg, "修复问题");
        assert_eq!(logs[0].changed_paths.len(), 2);
        assert_eq!(logs[0].changed_paths[0].action, "M");
        assert_eq!(logs[0].changed_paths[1].copyfrom_path, "/branches/v1");
        assert_eq!(logs[0].changed_paths[1].copyfrom_rev, Some(7));
        // 空 <msg/> 不报错
        assert_eq!(logs[1].msg, "");
    }

    #[test]
    fn parse_status_works() {
        let (status, against) = parse_status(STATUS_XML.as_bytes()).unwrap();
        assert_eq!(against, None);
        assert_eq!(status.len(), 2);
        assert_eq!(status[0].path, "/Users/u/wc/a.txt");
        assert_eq!(status[0].item, "modified");
        assert_eq!(status[0].wc_revision, Some(9));
        assert_eq!(status[0].commit_revision, Some(8));
        assert_eq!(status[0].commit_author, "li");
        assert_eq!(status[1].item, "unversioned");
    }

    #[test]
    fn binary_detect() {
        assert!(!is_binary(b"hello world"));
        assert!(is_binary(b"a\x00b"));
    }

    // —— 三方合并冲突解析 ——

    const CONFLICT_1: &str = "line1\n<<<<<<< .mine\nLOCAL-LINE\n||||||| .r3\nline2\n=======\nREMOTE-LINE\n>>>>>>> .r4\nline3\nline4\n";

    fn block_of(doc: &ConflictDoc, i: usize) -> &ConflictBlockBytes {
        match &doc.parts[i] {
            ConflictPart::Block(b) => b,
            ConflictPart::Context(_) => panic!("parts[{i}] 不是块"),
        }
    }

    #[test]
    fn parse_single_block_direction() {
        let doc = parse_conflict_bytes(CONFLICT_1.as_bytes()).unwrap();
        assert!(doc.has_markers);
        assert_eq!(doc.line_ending, LineEnding::Lf);
        assert!(doc.trailing_newline);
        // parts: Context(line1) + Block + Context(line3,line4)
        assert_eq!(doc.parts.len(), 3);
        let b = block_of(&doc, 1);
        assert_eq!(b.mine, vec![b"LOCAL-LINE".to_vec()], "mine = 本地修改");
        assert_eq!(b.base, vec![b"line2".to_vec()], "base = 共同祖先");
        assert_eq!(b.theirs, vec![b"REMOTE-LINE".to_vec()], "theirs = 远端修改");
        // 上下文
        match &doc.parts[0] {
            ConflictPart::Context(c) => assert_eq!(c, &vec![b"line1".to_vec()]),
            _ => panic!(),
        }
        match &doc.parts[2] {
            ConflictPart::Context(c) => assert_eq!(c, &vec![b"line3".to_vec(), b"line4".to_vec()]),
            _ => panic!(),
        }
        // UI 展示
        let ui = doc.ui_blocks();
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].mine, vec!["LOCAL-LINE".to_string()]);
        assert_eq!(ui[0].theirs, vec!["REMOTE-LINE".to_string()]);
    }

    #[test]
    fn parse_block_without_base() {
        let text = "a\n<<<<<<< .mine\nM1\n=======\nT1\n>>>>>>> .r2\nb\n";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        let b = block_of(&doc, 1);
        assert!(b.base.is_empty(), "无 base 段");
        assert_eq!(b.mine, vec![b"M1".to_vec()]);
        assert_eq!(b.theirs, vec![b"T1".to_vec()]);
    }

    #[test]
    fn parse_multiple_blocks_with_context() {
        let text = "h\n<<<<<<< .mine\nA\n=======\nB\n>>>>>>> .r2\nmid\n<<<<<<< .mine\nC\n||||||| .r2\nD\n=======\nE\n>>>>>>> .r3\ntail\n";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        let blocks: Vec<&ConflictBlockBytes> = doc
            .parts
            .iter()
            .filter_map(|p| match p {
                ConflictPart::Block(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].mine, vec![b"A".to_vec()]);
        assert!(blocks[0].base.is_empty());
        assert_eq!(blocks[0].theirs, vec![b"B".to_vec()]);
        assert_eq!(blocks[1].mine, vec![b"C".to_vec()]);
        assert_eq!(blocks[1].base, vec![b"D".to_vec()]);
        assert_eq!(blocks[1].theirs, vec![b"E".to_vec()]);
        // 上下文行 mid / h / tail 数量
        let ctxs: Vec<usize> = doc
            .parts
            .iter()
            .filter_map(|p| match p {
                ConflictPart::Context(c) => Some(c.len()),
                _ => None,
            })
            .collect();
        assert_eq!(ctxs, vec![1, 1, 1]);
    }

    #[test]
    fn markdown_separator_not_misread() {
        // 块外的 =======（markdown setext / 分隔线）不得被当作块分隔符
        let text = "title\n=======\nbody\n";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        assert!(!doc.has_markers, "无冲突标记");
        assert_eq!(doc.parts.len(), 1);
        match &doc.parts[0] {
            ConflictPart::Context(c) => assert_eq!(c.len(), 3),
            _ => panic!(),
        }
    }

    #[test]
    fn unclosed_block_errors() {
        let text = "a\n<<<<<<< .mine\nM1\n";
        let err = parse_conflict_bytes(text.as_bytes()).unwrap_err();
        assert!(err.contains("未闭合"), "错误信息: {}", err);
    }

    #[test]
    fn crlf_preserved() {
        let text = "a\r\n<<<<<<< .mine\r\nM\r\n=======\r\nT\r\n>>>>>>> .r2\r\nb\r\n";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        assert_eq!(doc.line_ending, LineEnding::Crlf);
        let out = assemble_conflict(&doc, &[Choice::Mine]);
        assert_eq!(String::from_utf8(out).unwrap(), "a\r\nM\r\nb\r\n");
    }

    #[test]
    fn assemble_choices_matrix() {
        let doc = parse_conflict_bytes(CONFLICT_1.as_bytes()).unwrap();
        let mine = assemble_conflict(&doc, &[Choice::Mine]);
        assert_eq!(
            String::from_utf8(mine).unwrap(),
            "line1\nLOCAL-LINE\nline3\nline4\n"
        );
        let theirs = assemble_conflict(&doc, &[Choice::Theirs]);
        assert_eq!(
            String::from_utf8(theirs).unwrap(),
            "line1\nREMOTE-LINE\nline3\nline4\n"
        );
        let both = assemble_conflict(&doc, &[Choice::Both]);
        assert_eq!(
            String::from_utf8(both).unwrap(),
            "line1\nLOCAL-LINE\nREMOTE-LINE\nline3\nline4\n"
        );
        let none = assemble_conflict(&doc, &[Choice::None]);
        assert_eq!(String::from_utf8(none).unwrap(), "line1\nline3\nline4\n");
    }

    #[test]
    fn assemble_context_preserved_multi_block() {
        let text = "h\n<<<<<<< .mine\nA\n=======\nB\n>>>>>>> .r2\nmid\n<<<<<<< .mine\nC\n=======\nD\n>>>>>>> .r3\ntail\n";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        let out = assemble_conflict(&doc, &[Choice::Theirs, Choice::Mine]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "h\nB\nmid\nC\ntail\n"
        );
    }

    #[test]
    fn assemble_keeps_bytes_untouched_for_non_utf8() {
        // 非 UTF-8 字节（\xff）在块内应原样保留（不经 escape/unescape）
        let text = vec![b'a', b'\n'];
        let mut conflict = b"<<<<<<< .mine\n\xff\n=======\nT\n>>>>>>> .r2\n".to_vec();
        conflict.extend_from_slice(&text);
        let doc = parse_conflict_bytes(&conflict).unwrap();
        let out = assemble_conflict(&doc, &[Choice::Mine]);
        assert!(out.contains(&0xff), "非 UTF-8 字节应原样保留: {:?}", out);
    }

    #[test]
    fn empty_and_no_trailing_newline() {
        let text = "a\n<<<<<<< .mine\nM\n=======\nT\n>>>>>>> .r2";
        let doc = parse_conflict_bytes(text.as_bytes()).unwrap();
        assert!(!doc.trailing_newline);
        let out = assemble_conflict(&doc, &[Choice::Mine]);
        assert_eq!(String::from_utf8(out).unwrap(), "a\nM");
    }

    // —— 认证缓存解析（批次 11）——

    const AUTH_XML: &str = "------------------------------------------------------------------------\nCredential kind: svn.simple\nAuthentication realm: <https://svn.example.com:443> My Repo\nPassword cache: keychain\nUsername: zhangsan\n\n------------------------------------------------------------------------\nCredential kind: svn.ssl.server\nAuthentication realm: https://svn.example.com:443\nSubject: CN=MY-SERVER\nFingerprint: abc123def456\n\nCredentials cache in '/Users/x/.subversion' contains 2 credentials\n";

    #[test]
    fn parse_auth_creds_works() {
        let creds = parse_auth_creds(AUTH_XML);
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0].kind, "svn.simple");
        assert_eq!(creds[0].realm, "<https://svn.example.com:443> My Repo");
        assert_eq!(creds[0].username, "zhangsan");
        assert_eq!(creds[0].password_cache, "keychain");
        assert!(creds[0].raw.contains("Credential kind"));
        assert_eq!(creds[1].kind, "svn.ssl.server");
        assert_eq!(creds[1].subject, "CN=MY-SERVER");
        assert_eq!(creds[1].fingerprint, "abc123def456");
        assert!(creds[1].username.is_empty());
    }

    #[test]
    fn parse_auth_creds_empty() {
        assert!(parse_auth_creds("Credentials cache in '/x' contains 0 credentials\n").is_empty());
        assert!(parse_auth_creds("").is_empty());
    }

    #[test]
    fn unescape_u_escapes_works() {
        assert_eq!(unescape_u_escapes("{U+8BA4}{U+8BC1}{U+9886}{U+57DF}"), "认证领域");
        assert_eq!(unescape_u_escapes("plain ascii"), "plain ascii");
        // 非转义花括号保持原样
        assert_eq!(unescape_u_escapes("{not-escape}"), "{not-escape}");
        // 短十六进制边界
        assert_eq!(unescape_u_escapes("{U+41}"), "A");
    }
}
