<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { usePrompt } from "../prompt";
import VirtualList from "./VirtualList.vue";
import PromptDialog from "./PromptDialog.vue";
import {
  normalizeError,
  type DiffChunk,
  type Favorite,
  type FileContent,
  type FilePair,
  type HistoryEntry,
  type ListEntry,
  type LogEntry,
  type RepoInfo,
  type RepoLayout,
  type TaskInfo,
  type TaskResult,
  type UiError,
} from "../types";
import DiffView from "./DiffView.vue";

const emit = defineEmits<{ "checked-out": [path: string] }>();
const { promptState, uiPrompt, onPromptOk, onPromptCancel } = usePrompt();

const url = ref("");
// 访问历史：全部记录，按 kind 过滤展示
const history = ref<HistoryEntry[]>([]);
const remoteHistory = computed(() => history.value.filter((h) => h.kind === "remote"));
onMounted(async () => {
  try {
    history.value = await api.historyList();
  } catch {
    // 历史不可用不影响主流程
  }
  void loadFavs();
  // 自动恢复上次访问的远程仓库（修复刷新/切页后空白页：组件重建时重新连接最近仓库）
  try {
    const last = history.value.find((h) => h.kind === "remote");
    if (last && !info.value) {
      url.value = last.value;
      await open(last.value);
    }
  } catch {
    // 自动恢复失败不阻塞（输入框已保留 URL，可手动重试）
  }
});
const loading = ref(false);
const info = ref<RepoInfo | null>(null);
const entries = ref<ListEntry[]>([]);
const crumbs = ref<{ url: string; name: string }[]>([]);
const fileView = ref<{ name: string; text: string; isBinary: boolean; size: number } | null>(
  null,
);
const logs = ref<LogEntry[]>([]);
const logSearch = ref("");
const logDateFrom = ref("");
const logDateTo = ref("");
const logsLoading = ref(false);
const selectedLog = ref<LogEntry | null>(null);
const diff = ref<FilePair | null>(null);
const diffChunks = ref<DiffChunk[] | null>(null);
const diffOldRev = ref<number | null>(null);
const diffNote = ref("");
// 大文件阈值：两侧总字符数超过该值，diff 计算下沉 Rust（similar）
const LARGE_DIFF_THRESHOLD = 400_000;
const diffLoading = ref(false);
const err = ref<UiError | null>(null);
const notice = ref("");
const exporting = ref(false);
const writeMsg = ref("");
const writing = ref(false);
// 分支/标签布局（批次 15）
const layout = ref<RepoLayout | null>(null);
// 认证失败重试弹窗（批次 11 + P0-1 全局化）
const authPrompt = ref(false);
const authUrl = ref("");
const authUser = ref("");
const authPass = ref("");
const authErr = ref("");
const authBusy = ref(false);
let authResolve: ((ok: boolean) => void) | null = null;
let authPendingOp: (() => Promise<void>) | null = null;

/** 弹认证窗：用户提交成功后执行 pendingOp（重建会话/重试），返回是否成功 */
function promptAuth(pendingOp: (() => Promise<void>) | null, url: string): Promise<boolean> {
  authUrl.value = url;
  authUser.value = "";
  authPass.value = "";
  authErr.value = "";
  authPendingOp = pendingOp;
  authPrompt.value = true;
  return new Promise((res) => {
    authResolve = res;
  });
}

function cancelAuth(): void {
  authPrompt.value = false;
  authPendingOp = null;
  authResolve?.(false);
  authResolve = null;
}

/**
 * 通用认证保护：操作抛 E170001 → 弹认证窗（remoteOpenAuth 写入凭据缓存）→
 * pendingOp 重试原操作；覆盖浏览目录/日志/文件/写操作，避免「只能重新打开仓库」。
 */
async function authGuard<T>(op: () => Promise<T>): Promise<T> {
  try {
    return await op();
  } catch (e) {
    const ce = normalizeError(e);
    if (ce.category !== "auth") throw e;
    const authTarget = currentUrl() || url.value;
    const ok = await promptAuth(async () => {
      await op(); // 凭据已缓存，重试原操作（无返回值用法）
    }, authTarget);
    if (!ok) throw e;
    return undefined as T;
  }
}
// 收藏（批次 14）
const favorites = ref<Favorite[]>([]);
const favBusy = ref(false);

function currentUrl(): string {
  const last = crumbs.value[crumbs.value.length - 1];
  return last ? last.url : info.value?.url ?? "";
}

/** 当前浏览位置是否已收藏 */
const faved = computed(() => favorites.value.some((f) => f.url === currentUrl()));

async function loadFavs(): Promise<void> {
  try {
    favorites.value = await api.favList();
  } catch {
    // 收藏不可用不影响主流程
  }
}

/** 收藏/取消收藏当前目录 */
async function toggleFav(): Promise<void> {
  const u = currentUrl();
  if (!u || favBusy.value) return;
  favBusy.value = true;
  try {
    if (faved.value) {
      await api.favRemove(u);
    } else {
      const seg = decodeURIComponent(u.split("/").filter(Boolean).pop() ?? u);
      const name = (await uiPrompt("收藏名称", seg || "远程仓库")) ?? "";
      if (!name.trim()) return;
      await api.favAdd(name.trim(), u);
    }
    await loadFavs();
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    favBusy.value = false;
  }
}

/** 从收藏切换过去 */
async function gotoFav(f: Favorite): Promise<void> {
  if (f.url === currentUrl()) return;
  url.value = f.url;
  await open(f.url);
}

/** 取消收藏 */
async function removeFav(f: Favorite): Promise<void> {
  try {
    await api.favRemove(f.url);
    await loadFavs();
  } catch (e) {
    err.value = normalizeError(e);
  }
}

function openUrl(): void {
  const u = url.value.trim();
  if (!u) return;
  open(u);
}

async function open(u: string): Promise<void> {
  loading.value = true;
  err.value = null;
  notice.value = "";
  try {
    info.value = await api.remoteOpen(u);
    void api.historyAdd("remote", u); // 打开成功才记录
    url.value = info.value.rootUrl ? u : u;
    crumbs.value = [];
    if (info.value.url) crumbs.value.push({ url: info.value.url, name: "" });
    await Promise.all([loadDir(), loadLogs()]);
  } catch (e) {
    const ce = normalizeError(e);
    if (ce.category === "auth") {
      // 认证失败：弹窗收集凭据 → 成功后重新 open（凭据已写入缓存）
      const ok = await promptAuth(null, u);
      if (ok) {
        await open(u);
        return;
      }
    } else if (ce.category === "certificate") {
      // 证书不受信任：询问临时接受（本次会话）
      const ok = window.confirm(
        `服务器证书不受信任：\n${ce.summary}\n${ce.detail}\n\n是否临时接受该证书并连接？（仅本次，不写入信任缓存）`,
      );
      if (ok) {
        await trustOpen(u);
        return;
      }
    }
    err.value = ce;
  } finally {
    loading.value = false;
  }
}

/** 探测仓库标准布局（分支/标签） */
async function loadLayout(): Promise<void> {
  if (!info.value?.rootUrl) return;
  try {
    layout.value = await api.remoteRepoLayout(info.value.rootUrl);
  } catch {
    layout.value = null;
  }
}

/** 临时信任证书重连 */
async function trustOpen(u: string): Promise<void> {
  loading.value = true;
  err.value = null;
  try {
    const info2 = await api.remoteOpenTrust(u);
    info.value = info2;
    void api.historyAdd("remote", u);
    url.value = info2.url || u;
    crumbs.value = [];
    if (info2.url) crumbs.value.push({ url: info2.url, name: "" });
    await Promise.all([loadDir(), loadLogs()]);
    void loadLayout();
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    loading.value = false;
  }
}

/** 从布局跳转（trunk / 分支 / 标签） */
function gotoLayoutUrl(target: string): void {
  if (!target) return;
  url.value = target;
  void open(target);
}

/** 创建分支/标签：复制 trunk（或当前）到 根/branches(或tags)/名称 */
async function createBranchTag(): Promise<void> {
  if (!layout.value || !info.value?.rootUrl) return;
  const asBranch = window.confirm("创建分支（确定）/ 标签（取消）？");
  const kind = asBranch ? "branches" : "tags";
  const name = await uiPrompt(asBranch ? "新分支名称" : "新标签名称");
  if (!name?.trim()) return;
  const root = info.value.rootUrl.replace(/\/+$/, "");
  const src = layout.value.trunk ?? currentUrl();
  const dst = `${root}/${kind}/${encodeURIComponent(name.trim())}`;
  if (!writeMsg.value.trim()) {
    writeMsg.value = `创建${asBranch ? "分支" : "标签"} ${name.trim()}`;
  }
  void doWrite(() => api.remoteCopy(src, dst, writeMsg.value), (r) => r.summary);
}

/** 认证弹窗提交：带用户名/密码连接（成功后执行待重试操作） */
async function submitAuth(): Promise<void> {
  if (!authUser.value.trim() || authBusy.value) return;
  authBusy.value = true;
  authErr.value = "";
  try {
    await api.remoteOpenAuth(authUrl.value, authUser.value.trim(), authPass.value);
    authPrompt.value = false;
    const pending = authPendingOp;
    authPendingOp = null;
    authResolve?.(true);
    authResolve = null;
    if (pending) await pending();
  } catch (e) {
    authErr.value = normalizeError(e).summary;
  } finally {
    authBusy.value = false;
  }
}

function entryUrl(name: string): string {
  const base = currentUrl();
  const sep = base.endsWith("/") ? "" : "/";
  return `${base}${sep}${encodeURIComponent(name)}`;
}

async function loadDir(): Promise<void> {
  const u = currentUrl();
  if (!u) return;
  entries.value = await authGuard(() => api.remoteList(u, null));
}

async function enterDir(e: ListEntry): Promise<void> {
  crumbs.value.push({ url: entryUrl(e.name), name: e.name });
  await loadDir();
  fileView.value = null;
  selectedLog.value = null;
  diff.value = null;
  diffOldRev.value = null;
  diffNote.value = "";
  diffChunks.value = null;
  await loadLogs();
}

function closeDiff(): void {
  diff.value = null;
  diffOldRev.value = null;
  diffNote.value = "";
  diffChunks.value = null;
}

async function goCrumb(i: number): Promise<void> {
  crumbs.value = crumbs.value.slice(0, i + 1);
  await loadDir();
  fileView.value = null;
  selectedLog.value = null;
  diff.value = null;
  diffOldRev.value = null;
  diffNote.value = "";
  diffChunks.value = null;
  await loadLogs();
}

async function showFile(e: ListEntry): Promise<void> {
  err.value = null;
  fileView.value = null;
  try {
    const fc = await authGuard(() => api.remoteCat(entryUrl(e.name), null));
    fileView.value = {
      name: e.name,
      text: fc.isBinary ? "" : decodeContent(fc.dataBase64, fc.isUtf8),
      isBinary: fc.isBinary,
      size: fc.size,
    };
  } catch (e2) {
    err.value = normalizeError(e2);
  }
}

function decodeContent(b64: string, isUtf8: boolean): string {
  try {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    if (isUtf8) return new TextDecoder("utf-8").decode(bytes);
    try {
      return new TextDecoder("gbk").decode(bytes);
    } catch {
      return new TextDecoder("latin1").decode(bytes);
    }
  } catch {
    return "(内容解码失败)";
  }
}

async function loadLogs(): Promise<void> {
  const u = currentUrl();
  if (!u) return;
  logsLoading.value = true;
  try {
    logs.value = await authGuard(() =>
      api.remoteLog(
        u,
        50,
        null,
        logSearch.value.trim() || null,
        logDateFrom.value.trim() || null,
        logDateTo.value.trim() || null,
      ),
    );
  } catch (e) {
    err.value = normalizeError(e);
    logs.value = [];
  } finally {
    logsLoading.value = false;
  }
}

async function applyLogFilter(): Promise<void> {
  selectedLog.value = null;
  diff.value = null;
  await loadLogs();
}

function clearLogFilter(): void {
  logSearch.value = "";
  logDateFrom.value = "";
  logDateTo.value = "";
  void applyLogFilter();
}

async function editLogMessage(l: LogEntry): Promise<void> {
  const msg = await uiPrompt(
    "编辑提交说明",
    l.msg,
    `编辑 r${l.revision} 的提交说明（需服务端 pre-revprop-change hook 允许）`,
  );
  if (msg === null) return;
  writing.value = true;
  err.value = null;
  notice.value = "";
  try {
    const r = await api.wcSetLog(currentUrl(), l.revision, msg);
    notice.value = r.summary;
    await loadLogs();
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    writing.value = false;
  }
}

/** 导出列表条目（文件或目录）到用户选择的文件夹：svn export URL 目标/名称 */
async function exportEntry(e: ListEntry): Promise<void> {
  const dir = await dialogOpen({ directory: true, title: "选择导出目标文件夹" });
  if (!dir) return;
  exporting.value = true;
  err.value = null;
  notice.value = "";
  try {
    const dest = `${dir.replace(/\/$/, "")}/${e.name}`;
    const id = await api.taskExport(entryUrl(e.name), dest, null);
    notice.value = `导出已加入后台任务 #${id}，见底部任务栏（目标：${dest}）`;
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    exporting.value = false;
  }
}

/** 检出列表条目（文件或目录）到本地：svn checkout URL 目标/名称 → 通知 App 切到工作副本页 */
async function checkoutEntry(e: ListEntry): Promise<void> {
  const dir = await dialogOpen({ directory: true, title: "选择检出目标文件夹" });
  if (!dir) return;
  exporting.value = true;
  err.value = null;
  notice.value = "";
  try {
    const dest = `${dir.replace(/\/$/, "")}/${e.name}`;
    const id = await api.taskCheckout(entryUrl(e.name), dest);
    notice.value = `检出已加入后台任务 #${id}，完成后可在工作副本页打开（${dest}）`;
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    exporting.value = false;
  }
}

/** 远程写操作共用：执行 → 提示 → 清空消息 → 刷新目录与日志 */
async function doWrite(fn: () => Promise<TaskResult>, okNote: (r: TaskResult) => string): Promise<void> {
  writing.value = true;
  err.value = null;
  notice.value = "";
  try {
    const r = await authGuard(fn);
    notice.value = okNote(r);
    writeMsg.value = "";
    await loadDir();
    await loadLogs();
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    writing.value = false;
  }
}

/** 新建目录：svn mkdir 当前目录/名称 */
async function promptMkdir(): Promise<void> {
  const name = await uiPrompt("新建目录", "", "输入要创建的目录名");
  if (!name?.trim()) return;
  void doWrite(() => api.remoteMkdir(entryUrl(name.trim()), writeMsg.value), (r) => r.summary);
}

/** 创建分支/标签：svn copy 当前目录 → 目标 URL */
async function promptCopy(): Promise<void> {
  const dst = await uiPrompt(
    "创建分支/标签",
    `${currentUrl()}/branches/`,
    "目标 URL（默认：在当前目录下创建分支）",
  );
  if (!dst?.trim()) return;
  void doWrite(() => api.remoteCopy(currentUrl(), dst.trim(), writeMsg.value), (r) => r.summary);
}

/** 导入本地目录：svn import 目录内容到当前目录（后台任务） */
async function promptImport(): Promise<void> {
  const dir = await dialogOpen({ directory: true, title: "选择要导入的本地目录" });
  if (!dir) return;
  // svn import 导入"目录内容"到目标 URL（目录名不自动出现），
  // 期望效果 = 当前浏览目录下新建同名子目录：目标 = currentUrl() + "/" + 本地目录名
  const name = dir.split(/[\\/]/).pop() || "";
  const base = currentUrl();
  if (!name || !base) return;
  const target = `${base}${base.endsWith("/") ? "" : "/"}${encodeURIComponent(name)}`;
  // 导入前体检：文件数 / 总大小 / 大文件 / 垃圾文件
  let statsText = "";
  try {
    const st = await api.dirStats(dir);
    const sizeMb = (st.totalSize / 1024 / 1024).toFixed(1);
    statsText = `文件数：${st.fileCount}，总大小：${sizeMb} MB`;
    if (st.bigFiles.length > 0) {
      statsText += `\n⚠ 大文件 ${st.bigFiles.length} 个（>5MB），上传耗时较长：\n${st.bigFiles
        .slice(0, 5)
        .join("\n")}${st.bigFiles.length > 5 ? "\n…" : ""}`;
    }
    if (st.junkFiles.length > 0) {
      statsText += `\n⚠ 含 ${st.junkFiles.length} 个垃圾文件（.DS_Store/*.pyc/*.err 等），建议先清理再导入：\n${st.junkFiles
        .slice(0, 3)
        .join("\n")}${st.junkFiles.length > 3 ? "\n…" : ""}`;
    }
  } catch {
    statsText = "（目录统计失败，将直接导入）";
  }
  if (
    !window.confirm(
      `确定将本地目录「${name}」\n导入到远程：\n${target}\n？\n${statsText}\n提交说明：${writeMsg.value.trim() || "（空）"}`,
    )
  ) {
    return;
  }
  err.value = null;
  try {
    const id = await api.taskImport(dir, target, writeMsg.value);
    notice.value = `导入已加入后台任务 #${id}，见底部任务栏`;
    writeMsg.value = "";
    // 后台等待任务完成，完成后自动刷新目录与日志并提示结果
    void waitImportTask(id);
  } catch (e2) {
    err.value = normalizeError(e2);
  }
}

/** 轮询等待导入任务结束（最长约 240s），结束后自动刷新目录/日志并提示 */
async function waitImportTask(id: number): Promise<void> {
  let finalResult: string | null = null;
  let state = "";
  for (let i = 0; i < 240; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    const tasks = await api.taskList().catch(() => [] as TaskInfo[]);
    const t = tasks.find((x) => x.id === id);
    if (!t) continue;
    if (t.state === "done") {
      finalResult = t.result;
      state = t.state;
      break;
    }
    if (t.state === "failed" || t.state === "cancelled") {
      state = t.state;
      break;
    }
  }
  try {
    await loadDir();
    await loadLogs();
  } catch {
    // 目录刷新失败不阻塞提示
  }
  if (state === "done" && finalResult) {
    notice.value = `导入完成：${finalResult}`;
  } else if (state === "done") {
    notice.value = "导入完成，目录已刷新";
  } else if (state === "cancelled") {
    notice.value = "导入已取消，目录已刷新";
  } else {
    notice.value = "导入任务已结束（未完成），目录已刷新";
  }
}

/** 远程删除（强确认，立即提交） */
function deleteEntry(e: ListEntry): void {
  const target = entryUrl(e.name);
  if (!window.confirm(`确定远程删除 ${target}？\n该操作立即提交，不可撤销。`)) return;
  void doWrite(() => api.remoteDelete(target, writeMsg.value), (r) => r.summary);
}

/** 远程重命名：svn move 同目录内 */
async function renameEntry(e: ListEntry): Promise<void> {
  const name = await uiPrompt(`重命名 ${e.name}`, e.name);
  if (!name?.trim() || name.trim() === e.name) return;
  void doWrite(
    () => api.remoteMove(entryUrl(e.name), entryUrl(name.trim()), writeMsg.value),
    (r) => r.summary,
  );
}

async function downloadCurrent(): Promise<void> {
  if (!fileView.value) return;
  const p = await dialogSave({ defaultPath: fileView.value.name, title: "保存文件" });
  if (!p) return;
  exporting.value = true;
  err.value = null;
  notice.value = "";
  try {
    const id = await api.taskExport(entryUrl(fileView.value.name), p, null);
    notice.value = `下载已加入后台任务 #${id}，见底部任务栏（保存到：${p}）`;
  } catch (e2) {
    err.value = normalizeError(e2);
  } finally {
    exporting.value = false;
  }
}

async function pickLog(log: LogEntry, index: number): Promise<void> {
  selectedLog.value = log;
  diffLoading.value = true;
  err.value = null;
  diffNote.value = "";
  try {
    const next = logs.value[index + 1];
    if (!next) {
      diff.value = null;
      diffOldRev.value = null;
      diffChunks.value = null;
      diffNote.value = "这是最早的已加载记录，无法自动计算变更。";
      return;
    }
    // 并排视图：两侧分别按各自版本拉取（旧版本 peg 到 next.revision）
    let oldFc: FileContent | null = null;
    let oldFail = "";
    try {
      oldFc = await authGuard(() => api.remoteCat(currentUrl(), next.revision));
    } catch (e) {
      oldFail = normalizeError(e).summary;
    }
    const newFc = await authGuard(() => api.remoteCat(currentUrl(), log.revision));
    if (oldFail) {
      diffNote.value = `旧版本（r${next.revision}）读取失败：${oldFail}（该路径可能在此版本中尚不存在）`;
    }
    if (oldFc?.isBinary || newFc.isBinary) {
      diff.value = { oldText: "", newText: "", isBinary: true, isUnversioned: false };
      diffOldRev.value = next.revision;
      diffChunks.value = null;
      return;
    }
    diff.value = {
      oldText: oldFc ? decodeContent(oldFc.dataBase64, oldFc.isUtf8) : "",
      newText: decodeContent(newFc.dataBase64, newFc.isUtf8),
      isBinary: false,
      isUnversioned: false,
    };
    diffOldRev.value = next.revision;
    if (
      diff.value.oldText.length + diff.value.newText.length >
      LARGE_DIFF_THRESHOLD
    ) {
      diffChunks.value = await api.diffChunks(diff.value.oldText, diff.value.newText);
    } else {
      diffChunks.value = null;
    }
  } catch (e) {
    err.value = normalizeError(e);
    diff.value = null;
    diffOldRev.value = null;
  } finally {
    diffLoading.value = false;
  }
}

function fmtDate(d: string): string {
  if (!d) return "";
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(d);
  return m ? `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]}` : d;
}

function fmtSize(n: number | null): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

const currentPath = computed(() => {
  if (crumbs.value.length <= 1) return "/";
  return crumbs.value
    .slice(1)
    .map((c) => c.name)
    .join("/");
});

const diffTitle = computed(() => {
  const rev = selectedLog.value?.revision ?? "?";
  const old = diffOldRev.value ?? "?";
  return `${currentPath.value} r${old} → r${rev}`;
});
</script>

<template>
  <section class="remote">
    <div class="openbar">
      <input
        v-model="url"
        list="remote-history"
        placeholder="仓库 URL，如 https://example.com/svn/trunk 或 file:///path/to/repo"
        spellcheck="false"
        @keyup.enter="openUrl"
      />
      <datalist id="remote-history">
        <option v-for="h in remoteHistory" :key="h.value" :value="h.value" />
      </datalist>
      <button :disabled="loading" @click="openUrl">{{ loading ? "连接中…" : "打开" }}</button>
      <button v-if="info" :disabled="loading" @click="loadDir()">刷新目录</button>
      <button
        v-if="info"
        class="favbtn"
        :class="{ on: faved }"
        :disabled="favBusy"
        :title="faved ? '取消收藏当前目录' : '收藏当前目录'"
        @click="toggleFav"
      >
        {{ faved ? "★ 已收藏" : "☆ 收藏" }}
      </button>
    </div>

    <!-- 收藏夹（批次 14）：点击切换，× 删除 -->
    <div v-if="favorites.length > 0" class="favbar">
      <span class="favlabel">收藏</span>
      <button
        v-for="f in favorites"
        :key="f.url"
        class="favchip"
        :class="{ on: f.url === currentUrl() }"
        :title="f.url"
        @click="gotoFav(f)"
      >
        <span class="favname">{{ f.name }}</span>
        <span class="favdel" title="取消收藏" @click.stop="removeFav(f)">×</span>
      </button>
    </div>

    <!-- 认证失败重试弹窗（批次 11） -->
    <div v-if="authPrompt" class="mask" @click.self="cancelAuth">
      <div class="authdialog">
        <h3>需要认证</h3>
        <p class="authurl">{{ authUrl }}</p>
        <label class="authrow">
          <span>用户名</span>
          <input v-model="authUser" placeholder="svn 用户名" spellcheck="false" />
        </label>
        <label class="authrow">
          <span>密码</span>
          <input v-model="authPass" type="password" placeholder="密码" @keyup.enter="submitAuth" />
        </label>
        <p v-if="authErr" class="autherr">{{ authErr }}</p>
        <div class="authbtns">
          <button class="primary" :disabled="authBusy || !authUser.trim()" @click="submitAuth">
            {{ authBusy ? "连接中…" : "连接" }}
          </button>
          <button @click="cancelAuth">取消</button>
        </div>
        <p class="authhint">密码仅本次会话传入 svn，不会保存到应用或日志。</p>
      </div>
    </div>

    <div v-if="err" class="errbox">
      <span class="errcat">{{ err.category }}</span>
      <b>{{ err.summary }}</b>
      <span class="errhint">{{ err.hint }}</span>
      <button class="link" @click="err = null">关闭</button>
    </div>

    <template v-if="info">
      <div class="infobar">
        <span title="HEAD revision">revision {{ info.revision ?? "?" }}</span>
        <span v-if="info.uuid" title="UUID">UUID：{{ info.uuid }}</span>
        <span v-if="info.lastAuthor" title="最近提交">
          {{ info.lastAuthor }} @ {{ fmtDate(info.lastDate) }}
        </span>
        <span class="notice">{{ notice }}</span>
      </div>

      <!-- 路径导航（唯一路径展示，横跨全宽；⤴ 根悬停显示完整 rootUrl） -->
      <div class="crumb" :title="info.rootUrl">
        <button class="link" @click="goCrumb(0)">⤴ 根</button>
        <template v-for="(c, i) in crumbs.slice(1)" :key="i">
          <span class="sep">/</span>
          <button class="link" @click="goCrumb(i + 1)">{{ c.name }}</button>
        </template>
        <span class="path">{{ currentPath }}</span>
        <span v-if="layout" class="layoutbar">
          <template v-if="layout.branchesDir || layout.branches.length">
            <select
              class="layoutsel"
              :value="layout.branchesDir ?? ''"
              @change="gotoLayoutUrl(($event.target as HTMLSelectElement).value)"
            >
              <option :value="layout.branchesDir ?? ''">— 分支 —</option>
              <option v-for="b in layout.branches" :key="b" :value="`${layout.branchesDir}/${b}`">
                {{ b }}
              </option>
            </select>
          </template>
          <template v-if="layout.tagsDir || layout.tags.length">
            <select
              class="layoutsel"
              :value="layout.tagsDir ?? ''"
              @change="gotoLayoutUrl(($event.target as HTMLSelectElement).value)"
            >
              <option :value="layout.tagsDir ?? ''">— 标签 —</option>
              <option v-for="t in layout.tags" :key="t" :value="`${layout.tagsDir}/${t}`">
                {{ t }}
              </option>
            </select>
          </template>
          <button v-if="layout.branchesDir || layout.tagsDir" class="link" @click="createBranchTag">
            + 创建分支/标签
          </button>
        </span>
      </div>

      <div class="columns">
        <div class="left">
          <div class="writebar">
            <input
              v-model="writeMsg"
              class="msg"
              placeholder="提交说明（远程写操作必填）"
              :disabled="writing"
            />
            <button :disabled="!writeMsg.trim() || writing" @click="promptMkdir">新建目录</button>
            <button :disabled="!writeMsg.trim() || writing" @click="promptCopy">创建分支/标签</button>
            <button :disabled="!writeMsg.trim() || writing" @click="promptImport">导入目录</button>
          </div>
          <div class="listhead">
            <span>名称</span><span>类型</span><span>大小</span><span>revision</span><span>作者</span><span class="ops">操作</span>
          </div>
          <VirtualList
            :items="entries"
            :row-height="30"
            min-height="0"
            class="vtree"
          >
            <template #row="{ item: e }">
              <div
                class="vrow"
                :class="{ file: e.kind === 'file' }"
                @dblclick="e.kind === 'dir' ? enterDir(e) : showFile(e)"
              >
                <span class="c-name">
                  <span class="icon">{{ e.kind === "dir" ? "📁" : "📄" }}</span>
                  <button class="link" @click="e.kind === 'dir' ? enterDir(e) : showFile(e)">
                    {{ e.name }}
                  </button>
                </span>
                <span class="c-kind">{{ e.kind }}</span>
                <span class="c-size">{{ fmtSize(e.size) }}</span>
                <span class="c-rev">{{ e.revision ?? "" }}</span>
                <span class="c-author">{{ e.author }}</span>
                <span class="c-ops">
                  <button class="link icobtn" title="导出" :disabled="exporting" @click="exportEntry(e)">⤓</button>
                  <button class="link icobtn" title="检出到本地" :disabled="exporting" @click="checkoutEntry(e)">⤵</button>
                  <button class="link icobtn" title="重命名/移动" :disabled="writing || exporting" @click="renameEntry(e)">✎</button>
                  <button class="link icobtn danger" title="删除（立即提交）" :disabled="writing || exporting" @click="deleteEntry(e)">🗑</button>
                </span>
              </div>
            </template>
          </VirtualList>
        </div>

        <div class="right">
          <div class="pane preview">
            <div class="pane-title">
              预览
              <span v-if="fileView" class="pane-sub">{{ fileView.name }}（{{ fmtSize(fileView.size) }}）</span>
              <button
                v-if="fileView"
                class="link"
                :disabled="exporting"
                @click="downloadCurrent"
              >
                {{ exporting ? "导出中…" : "下载" }}
              </button>
            </div>
            <pre v-if="fileView && !fileView.isBinary" class="filetext">{{ fileView.text }}</pre>
            <div v-else-if="fileView && fileView.isBinary" class="binary">二进制文件，不支持预览。</div>
            <div v-else class="hint">双击左侧文件查看内容；双击目录进入。</div>
          </div>

          <div class="pane log">
            <div class="pane-title">日志</div>
            <div class="logfilter">
              <input
                v-model="logSearch"
                class="msg"
                placeholder="搜索：说明 / 作者 / 路径 / 日期"
                spellcheck="false"
                @keyup.enter="applyLogFilter"
              />
              <input
                v-model="logDateFrom"
                class="msg date"
                placeholder="起始日期 2026-01-01"
                spellcheck="false"
              />
              <input
                v-model="logDateTo"
                class="msg date"
                placeholder="截止日期 2026-12-31"
                spellcheck="false"
              />
              <button :disabled="logsLoading" @click="applyLogFilter">过滤</button>
              <button
                v-if="logSearch || logDateFrom || logDateTo"
                class="link"
                :disabled="logsLoading"
                @click="clearLogFilter"
              >
                清除
              </button>
            </div>
            <div v-if="logsLoading" class="hint">加载中…</div>
            <div v-else-if="logs.length === 0" class="hint">无日志。</div>
            <VirtualList
              v-else
              :items="logs"
              :row-height="30"
              min-height="0"
              class="loglist"
            >
              <template #row="{ item: l, index: li }">
                <div
                  class="vlogrow"
                  :class="{ on: selectedLog?.revision === l.revision }"
                  @click="pickLog(l, li)"
                >
                  <b>r{{ l.revision }}</b>
                  <span class="auth">{{ l.author }}</span>
                  <span class="date">{{ fmtDate(l.date) }}</span>
                  <span class="msg">{{ l.msg || "(空提交说明)" }}</span>
                  <span v-if="l.changedPaths.length" class="cnt">{{ l.changedPaths.length }} 路径</span>
                  <button
                    class="link"
                    :disabled="logsLoading"
                    @click.stop="editLogMessage(l)"
                  >
                    编辑说明
                  </button>
                </div>
              </template>
            </VirtualList>
          </div>

          <div class="pane diff">
            <div class="pane-title">
              变更
              <span v-if="selectedLog" class="pane-sub">r{{ selectedLog.revision }} 的变更</span>
            </div>
            <div v-if="diffLoading" class="hint">计算中…</div>
            <div v-else-if="diff?.isBinary" class="hint">二进制文件，无法并排显示差异。</div>
            <div v-else-if="diffNote" class="hint">{{ diffNote }}</div>
            <div v-else-if="diff" class="diffbox">
              <DiffView
                :title="diffTitle"
                :old-text="diff.oldText"
                :new-text="diff.newText"
                :chunks="diffChunks"
                @close="closeDiff"
              />
            </div>
            <div v-else class="hint">点击日志行查看该 revision 的变更。</div>
          </div>
        </div>
      </div>
    </template>

    <div v-else class="empty">输入仓库 URL 并点击“打开”。无需 checkout 即可浏览目录、文件、日志与变更。</div>

    <!-- 通用输入弹窗（替代 window.prompt） -->
    <PromptDialog
      v-if="promptState.visible"
      :title="promptState.title"
      :message="promptState.message"
      :initial="promptState.initial"
      :placeholder="promptState.placeholder"
      @ok="onPromptOk"
      @cancel="onPromptCancel"
    />
  </section>
</template>

<style scoped>
.remote {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 10px 14px;
  gap: 8px;
}
.openbar {
  display: flex;
  gap: 8px;
}
.favbtn {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 6px 14px;
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
}
.favbtn.on {
  background: #fff8e1;
  border-color: #e0a800;
  color: #9a6700;
}
.favbar {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  padding: 4px 2px 0;
}
.favlabel {
  font-size: 12px;
  color: #8b949e;
  white-space: nowrap;
}
.favchip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid #d0d7de;
  background: #f6f8fa;
  border-radius: 12px;
  padding: 2px 6px 2px 10px;
  font-size: 12px;
  cursor: pointer;
  max-width: 220px;
}
.favchip:hover {
  background: #eaeef2;
}
.favchip.on {
  background: #ddf4ff;
  border-color: #54aeff;
}
.favname {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.favdel {
  color: #8b949e;
  font-size: 13px;
  line-height: 1;
  padding: 0 2px;
  border-radius: 50%;
}
.favdel:hover {
  color: #c0392b;
  background: #fff0f0;
}
.openbar input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
}
.openbar button {
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid #d0d7de;
  background: #fff;
  cursor: pointer;
  font-size: 13px;
}
.infobar {
  display: flex;
  gap: 16px;
  font-size: 12px;
  color: #57606a;
  flex-wrap: wrap;
  padding: 6px 8px;
  background: #f6f8fa;
  border-radius: 6px;
}
.notice {
  color: #9a6700;
}
.columns {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(300px, 2fr) minmax(420px, 3fr);
  gap: 10px;
}
.left,
.right {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
}
.writebar {
  display: flex;
  gap: 6px;
  align-items: center;
  padding-bottom: 6px;
}
.writebar input.msg {
  flex: 1;
  min-width: 0;
}
.danger {
  color: #c0392b !important;
}
.crumb {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 13px;
  padding-bottom: 6px;
  flex-wrap: wrap;
}
.layoutbar {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: 8px;
}
.layoutsel {
  font-size: 12px;
  padding: 2px 6px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  background: #fff;
  max-width: 160px;
}
.crumb .path {
  color: #57606a;
}
.sep {
  color: #8b949e;
}
.listhead,
.vrow {
  display: grid;
  grid-template-columns: minmax(120px, 1fr) 44px 48px 48px 80px 116px;
  font-size: 13px;
  align-items: center;
}
.listhead {
  position: sticky;
  top: 0;
  z-index: 1;
  background: #f6f8fa;
  border-bottom: 1px solid #eaeef2;
  padding: 6px 10px;
  font-weight: 600;
}
.listhead span,
.vrow span {
  padding: 0 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 窄窗口：隐藏次要列（类型/大小/revision/作者），保住名称列 */
@media (max-width: 1250px) {
  .listhead,
  .vrow {
    grid-template-columns: minmax(140px, 1fr) 116px;
  }
  .listhead span:nth-child(2),
  .listhead span:nth-child(3),
  .listhead span:nth-child(4),
  .listhead span:nth-child(5),
  .vrow span:nth-child(2),
  .vrow span:nth-child(3),
  .vrow span:nth-child(4),
  .vrow span:nth-child(5) {
    display: none;
  }
}
.vrow {
  height: 30px;
  padding: 0 10px;
  border-bottom: 1px solid #f0f2f5;
  cursor: default;
}
.vrow:hover {
  background: #f0f6ff;
}
.vrow .c-name {
  display: flex;
  align-items: center;
  gap: 2px;
  min-width: 0;
}
.vrow .c-name .link {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.vrow .c-ops {
  display: flex;
  gap: 6px;
}
.icobtn {
  font-size: 13px;
  line-height: 1;
  padding: 3px 4px;
  border-radius: 4px;
}
.icobtn:hover {
  background: #eaeef2;
}
.icobtn.danger:hover {
  background: #fff0f0;
}
.vtree {
  min-height: 0;
}
.icon {
  margin-right: 4px;
}
.link {
  background: none;
  border: none;
  color: #0969da;
  cursor: pointer;
  font-size: 13px;
  padding: 0;
  text-align: left;
}
.right {
  gap: 8px;
}
.pane {
  display: flex;
  flex-direction: column;
  border: 1px solid #d8dee4;
  border-radius: 6px;
  min-height: 0;
  background: #fff;
}
.pane-title {
  font-size: 12px;
  font-weight: 600;
  color: #57606a;
  padding: 6px 10px;
  border-bottom: 1px solid #eaeef2;
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.pane-sub {
  font-weight: 400;
  color: #8b949e;
}
.preview {
  flex: 2;
}
.log {
  flex: 2;
}
.diff {
  flex: 3;
}
.filetext {
  flex: 1;
  min-height: 0;
  overflow: auto;
  margin: 0;
  padding: 8px 10px;
  font-size: 12px;
  font-family: ui-monospace, "SF Mono", Menlo, monospace;
  white-space: pre-wrap;
  word-break: break-all;
}
.diffbox {
  flex: 1;
  min-height: 0;
  display: flex;
}
.diffbox > * {
  flex: 1;
  min-width: 0;
}
.binary,
.hint {
  padding: 12px;
  font-size: 12px;
  color: #8b949e;
}
.logfilter {
  display: flex;
  gap: 6px;
  padding: 8px 10px 6px;
  flex-wrap: wrap;
  align-items: center;
}
.logfilter input.msg {
  flex: 1;
  min-width: 120px;
}
.logfilter input.date {
  flex: 0 1 150px;
}
.logfilter button.link {
  font-size: 12px;
}
.loglist {
  min-height: 0;
}
.vlogrow {
  display: flex;
  gap: 8px;
  padding: 5px 10px;
  font-size: 12px;
  border-bottom: 1px solid #f0f2f5;
  cursor: pointer;
  align-items: baseline;
  height: 30px;
  box-sizing: border-box;
  overflow: hidden;
  white-space: nowrap;
}
.vlogrow:hover {
  background: #f0f6ff;
}
.vlogrow.on {
  background: #ddf4ff;
}
.vlogrow .auth {
  color: #0969da;
  min-width: 60px;
}
.vlogrow .date {
  color: #57606a;
  min-width: 100px;
}
.vlogrow .msg {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.vlogrow .cnt {
  color: #8b949e;
}
.empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #8b949e;
  font-size: 14px;
}
.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 60;
}
.authdialog {
  background: #fff;
  color: #1f2328;
  border-radius: 8px;
  padding: 16px 20px;
  width: 380px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.3);
}
.authdialog h3 {
  margin: 0 0 8px;
  font-size: 15px;
}
.authurl {
  font-size: 12px;
  color: #57606a;
  word-break: break-all;
  margin: 0 0 12px;
}
.authrow {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  margin-bottom: 8px;
}
.authrow span {
  width: 52px;
  color: #57606a;
}
.authrow input {
  flex: 1;
  padding: 6px 8px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
}
.autherr {
  color: #c0392b;
  font-size: 12px;
  margin: 0 0 8px;
}
.authbtns {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 10px;
}
.authbtns button {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 5px 14px;
  cursor: pointer;
  font-size: 13px;
}
.authbtns button.primary {
  background: #1f883d;
  border-color: #1f883d;
  color: #fff;
}
.authhint {
  font-size: 11px;
  color: #8b949e;
  margin: 8px 0 0;
}
</style>
