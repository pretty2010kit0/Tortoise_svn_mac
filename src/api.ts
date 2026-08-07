// Tauri invoke 封装：所有命令按 Rust 侧签名调用
import { invoke } from "@tauri-apps/api/core";
import type {
  AuthCred,
  BlameLine,
  ConflictChoice,
  ConflictInfo,
  DiffChunk,
  DiffResult,
  DirStats,
  Favorite,
  FileContent,
  FilePair,
  HistoryEntry,
  ListEntry,
  LogEntry,
  PropEntry,
  RepoInfo,
  RepoLayout,
  StatusEntry,
  StatusU,
  SvnVersion,
  TaskInfo,
  TaskResult,
  WcInfo,
} from "./types";

export const api = {
  svnVersion: () => invoke<SvnVersion>("svn_version"),
  setSvnExecutable: (path: string) =>
    invoke<SvnVersion>("set_svn_executable", { path }),

  // ── 访问历史（批次 10）：远程 URL / 本地路径下拉 ──
  historyList: () => invoke<HistoryEntry[]>("history_list"),
  historyAdd: (kind: string, value: string) =>
    invoke<void>("history_add", { kind, value }),

  remoteOpen: (url: string) => invoke<RepoInfo>("remote_open", { url }),
  remoteList: (url: string, rev?: number | null) =>
    invoke<ListEntry[]>("remote_list", { url, rev: rev ?? null }),
  remoteCat: (url: string, rev?: number | null) =>
    invoke<FileContent>("remote_cat", { url, rev: rev ?? null }),
  remoteExport: (url: string, dest: string, rev?: number | null) =>
    invoke<TaskResult>("remote_export", { url, dest, rev: rev ?? null }),
  remoteMkdir: (url: string, message: string) =>
    invoke<TaskResult>("remote_mkdir", { url, message }),
  remoteDelete: (url: string, message: string) =>
    invoke<TaskResult>("remote_delete", { url, message }),
  remoteCopy: (src: string, dst: string, message: string) =>
    invoke<TaskResult>("remote_copy", { src, dst, message }),
  remoteMove: (src: string, dst: string, message: string) =>
    invoke<TaskResult>("remote_move", { src, dst, message }),
  remoteImport: (local: string, url: string, message: string) =>
    invoke<TaskResult>("remote_import", { local, url, message }),
  remoteLog: (
    url: string,
    limit?: number | null,
    rev?: number | null,
    search?: string | null,
    dateFrom?: string | null,
    dateTo?: string | null,
  ) =>
    invoke<LogEntry[]>("remote_log", {
      url,
      limit: limit ?? null,
      rev: rev ?? null,
      search: search ?? null,
      dateFrom: dateFrom ?? null,
      dateTo: dateTo ?? null,
    }),
  remoteDiff: (url: string, rev1: number, rev2: number) =>
    invoke<DiffResult>("remote_diff", { url, rev1, rev2 }),
  diffChunks: (oldText: string, newText: string) =>
    invoke<DiffChunk[]>("diff_chunks", { oldText, newText }),

  wcOpen: (path: string) => invoke<WcInfo>("wc_open", { path }),
  wcStatus: (path: string) => invoke<StatusEntry[]>("wc_status", { path }),
  wcDiff: (path: string) => invoke<DiffResult>("wc_diff", { path }),
  wcFilePair: (path: string) => invoke<FilePair>("wc_file_pair", { path }),
  wcDiffExternal: (path: string) => invoke<TaskResult>("wc_diff_external", { path }),
  wcCommit: (paths: string[], message: string) =>
    invoke<TaskResult>("wc_commit", { paths, message }),
  wcUpdate: (path: string) => invoke<TaskResult>("wc_update", { path }),
  wcCheckout: (url: string, dest: string) =>
    invoke<TaskResult>("wc_checkout", { url, dest }),
  wcAdd: (paths: string[]) => invoke<TaskResult>("wc_add", { paths }),
  wcDelete: (paths: string[]) => invoke<TaskResult>("wc_delete", { paths }),
  wcResolve: (paths: string[], accept: string) =>
    invoke<TaskResult>("wc_resolve", { paths, accept }),
  wcCleanup: (path: string) => invoke<TaskResult>("wc_cleanup", { path }),
  wcUpgrade: (path: string) => invoke<TaskResult>("wc_upgrade", { path }),
  wcProplist: (path: string) => invoke<PropEntry[]>("wc_proplist", { path }),
  wcPropset: (path: string, name: string, value: string) =>
    invoke<TaskResult>("wc_propset", { path, name, value }),
  wcPropdel: (path: string, name: string) =>
    invoke<TaskResult>("wc_propdel", { path, name }),
  wcLock: (paths: string[], comment: string) =>
    invoke<TaskResult>("wc_lock", { paths, comment }),
  wcUnlock: (paths: string[], force: boolean) =>
    invoke<TaskResult>("wc_unlock", { paths, force }),
  wcStatusU: (path: string) => invoke<StatusU>("wc_status_u", { path }),
  wcBlame: (path: string, rev?: number | null) =>
    invoke<BlameLine[]>("wc_blame", { path, rev: rev ?? null }),
  wcChangelist: (name: string, paths: string[], remove: boolean) =>
    invoke<TaskResult>("wc_changelist", { name, paths, remove }),
  wcCommitCl: (name: string, message: string, wcPath: string) =>
    invoke<TaskResult>("wc_commit_cl", { name, message, wcPath }),
  wcSwitch: (path: string, targetUrl: string, depth?: string | null) =>
    invoke<TaskResult>("wc_switch", { path, targetUrl, depth: depth ?? null }),
  wcRelocate: (path: string, newUrl: string, fromUrl?: string | null) =>
    invoke<TaskResult>("wc_relocate", { path, newUrl, fromUrl: fromUrl ?? null }),
  wcMerge: (
    target: string,
    sourceUrl: string,
    revFrom?: number | null,
    revTo?: number | null,
    dryRun?: boolean,
  ) =>
    invoke<TaskResult>("wc_merge", {
      target,
      sourceUrl,
      revFrom: revFrom ?? null,
      revTo: revTo ?? null,
      dryRun: dryRun ?? false,
    }),
  wcDiffText: (path: string) => invoke<string>("wc_diff_text", { path }),
  wcPatchApply: (path: string, patchText: string) =>
    invoke<TaskResult>("wc_patch_apply", { path, patchText }),
  wcMove: (src: string, dst: string) =>
    invoke<TaskResult>("wc_move", { src, dst }),
  wcSetLog: (url: string, rev: number, message: string) =>
    invoke<TaskResult>("wc_set_log", { url, rev, message }),
  wcRevert: (paths: string[]) => invoke<TaskResult>("wc_revert", { paths }),

  // ── 三方合并（批次 9）──
  wcConflictParse: (path: string) => invoke<ConflictInfo>("wc_conflict_parse", { path }),
  wcConflictResolve: (path: string, choices: ConflictChoice[]) =>
    invoke<TaskResult>("wc_conflict_resolve", { path, choices }),

  // ── 认证（批次 11）──
  authList: () => invoke<AuthCred[]>("svn_auth_list"),
  authRemove: (patterns: string[]) => invoke<TaskResult>("svn_auth_remove", { patterns }),
  remoteOpenAuth: (url: string, username: string, password: string) =>
    invoke<RepoInfo>("remote_open_auth", { url, username, password }),

  // ── 收藏（批次 14）──
  favAdd: (name: string, url: string) => invoke<void>("fav_add", { name, url }),
  favList: () => invoke<Favorite[]>("fav_list"),
  favRemove: (url: string) => invoke<void>("fav_remove", { url }),
  favClear: () => invoke<void>("fav_clear"),

  // ── 目录体检（导入前提示）──
  dirStats: (path: string) => invoke<DirStats>("dir_stats", { path }),

  // ── 分支/标签管理（批次 15）──
  remoteRepoLayout: (rootUrl: string) => invoke<RepoLayout>("remote_repo_layout", { rootUrl }),

  // ── 证书信任（批次 15）──
  remoteOpenTrust: (url: string) => invoke<RepoInfo>("remote_open_trust", { url }),

  // ── svn:ignore 批量（批次 15）──
  wcIgnoreAdd: (paths: string[]) => invoke<TaskResult>("wc_ignore_add", { paths }),

  // ── 外部 diff 工具（批次 15）──
  getExternalDiff: () => invoke<string>("get_external_diff"),
  setExternalDiff: (cmd: string) => invoke<void>("set_external_diff", { cmd }),

  // ── TaskManager（批次 8）：后台长任务 ──
  taskList: () => invoke<TaskInfo[]>("task_list"),
  taskCancel: (id: number) => invoke<boolean>("task_cancel", { id }),
  taskRetry: (id: number) => invoke<number>("task_retry", { id }),
  taskCheckout: (url: string, dest: string) =>
    invoke<number>("task_checkout", { url, dest }),
  taskUpdate: (path: string) => invoke<number>("task_update", { path }),
  taskImport: (local: string, url: string, message: string) =>
    invoke<number>("task_import", { local, url, message }),
  taskExport: (url: string, dest: string, rev?: number | null) =>
    invoke<number>("task_export", { url, dest, rev: rev ?? null }),
};
