// 与 Rust 侧模型一一对应的类型（serde camelCase）

export interface SvnVersion {
  bin: string;
  version: string;
}

export interface RepoInfo {
  rootUrl: string;
  url: string;
  relativeUrl: string;
  uuid: string;
  revision: number | null;
  lastAuthor: string;
  lastDate: string;
  isWc: boolean;
  wcRoot: string;
  entryCount: number;
}

export interface ListEntry {
  name: string;
  kind: string; // file | dir
  size: number | null;
  revision: number | null;
  author: string;
  date: string;
}

export interface ChangePath {
  path: string;
  kind: string;
  action: string; // A / D / M / R
  copyfromPath: string;
  copyfromRev: number | null;
}

export interface LogEntry {
  revision: number;
  author: string;
  date: string;
  msg: string;
  changedPaths: ChangePath[];
}

export interface FileContent {
  dataBase64: string;
  size: number;
  isUtf8: boolean;
  isBinary: boolean;
}

export interface FilePair {
  oldText: string;
  newText: string;
  isBinary: boolean;
  isUnversioned: boolean;
}

/** 变更块（对应 @codemirror/merge Chunk）：字符位置的行区间 */
export interface DiffChunk {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

export interface StatusEntry {
  path: string;
  item: string;
  props: string;
  wcRevision: number | null;
  commitRevision: number | null;
  commitAuthor: string;
  wcLocked: boolean;
  lockOwner: string;
  lockComment: string;
  reposItem: string;
}

export interface BlameLine {
  lineNo: number;
  revision: number;
  author: string;
  text: string;
}

export interface StatusU {
  entries: StatusEntry[];
  against: number | null;
}

export interface PropEntry {
  name: string;
  value: string;
}

export interface WcInfo {
  rootUrl: string;
  url: string;
  uuid: string;
  revision: number | null;
  wcRoot: string;
  schedule: string;
  depth: string;
  status: StatusEntry[];
}

export interface DiffResult {
  text: string;
  isBinary: boolean;
  isEmpty: boolean;
  unversioned: boolean;
}

export interface TaskResult {
  ok: boolean;
  summary: string;
  stdout: string;
  stderr: string;
}

/** TaskManager 任务状态 */
export type TaskState = "running" | "done" | "failed" | "cancelled";

/** TaskManager 任务信息（后台长任务） */
export interface TaskInfo {
  id: number;
  desc: string;
  state: TaskState;
  startedAt: number;
  finishedAt: number | null;
  output: string;
  result: string | null;
}

export interface SvnError {
  category: string;
  summary: string;
  detail: string;
  recoveryHint: string;
}

/** 前端统一的错误展示对象 */
export interface UiError {
  summary: string;
  detail: string;
  hint: string;
  category: string;
}

/** 把 invoke 抛出的错误规范化为 UiError（Tauri 2 序列化错误可能是对象或字符串） */
export function normalizeError(e: unknown): UiError {
  if (typeof e === "object" && e !== null) {
    const o = e as Record<string, unknown>;
    if (typeof o.summary === "string") {
      return {
        summary: o.summary,
        detail: typeof o.detail === "string" ? o.detail : "",
        hint: typeof o.recoveryHint === "string" ? o.recoveryHint : "",
        category: typeof o.category === "string" ? o.category : "unknown",
      };
    }
  }
  return { summary: "操作失败", detail: String(e), hint: "", category: "unknown" };
}

export interface HistoryEntry {
  kind: "remote" | "local";
  value: string;
  lastUsed: number;
}

/** 三方合并：单个冲突块三栏文本 */
export interface ConflictBlock {
  mine: string[];
  base: string[];
  theirs: string[];
}

/** 三方合并：wcConflictParse 返回 */
export interface ConflictInfo {
  blocks: ConflictBlock[];
  hasMarkers: boolean;
  lineEnding: string;
}

/** 三方合并：逐块选择 */
export type ConflictChoice = "mine" | "theirs" | "both" | "none";

/** 认证缓存中的一条凭据（svn auth 输出解析） */
export interface AuthCred {
  kind: string;
  realm: string;
  username: string;
  subject: string;
  fingerprint: string;
  passwordCache: string;
  raw: string;
}

/** 远程仓库收藏 */
export interface Favorite {
  name: string;
  url: string;
  createdAt: number;
}

/** 导入前目录体检 */
export interface DirStats {
  fileCount: number;
  totalSize: number;
  bigFiles: string[];
  junkFiles: string[];
}

/** 仓库标准布局（分支/标签管理） */
export interface RepoLayout {
  trunk: string | null;
  branchesDir: string | null;
  tagsDir: string | null;
  branches: string[];
  tags: string[];
}

/** 合并信息（merged = 已合入，eligible = 可合入未合入） */
export interface MergeInfo {
  merged: number[];
  eligible: number[];
}

/** 状态条目中文标签 */
export const STATUS_LABEL: Record<string, string> = {
  none: "正常",
  normal: "正常",
  add: "已添加",
  delete: "已删除",
  modified: "已修改",
  conflicted: "冲突",
  obstructed: "被阻隔",
  missing: "缺失",
  unversioned: "未版本化",
  external: "外部",
  incomplete: "不完整",
  ignored: "已忽略",
};
