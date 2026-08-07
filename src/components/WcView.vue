<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { api } from "../api";
import {
  normalizeError,
  STATUS_LABEL,
  type BlameLine,
  type DiffChunk,
  type FilePair,
  type HistoryEntry,
  type PropEntry,
  type StatusEntry,
  type TaskResult,
  type UiError,
  type WcInfo,
} from "../types";
import DiffView from "./DiffView.vue";
import MergeDialog from "./MergeDialog.vue";
import PromptDialog from "./PromptDialog.vue";
import { usePrompt } from "../prompt";

const props = defineProps<{ initialPath?: string }>();

const path = ref("");
// 访问历史：全部记录，按 kind 过滤展示
const history = ref<HistoryEntry[]>([]);
const localHistory = computed(() => history.value.filter((h) => h.kind === "local"));
onMounted(async () => {
  try {
    history.value = await api.historyList();
  } catch {
    // 历史不可用不影响主流程
  }
});
const loading = ref(false);
const info = ref<WcInfo | null>(null);
const checked = ref<string[]>([]);
const selected = ref<StatusEntry | null>(null);
const diff = ref<FilePair | null>(null);
const diffChunks = ref<DiffChunk[] | null>(null);
// 大文件阈值：两侧总字符数超过该值，diff 计算下沉 Rust（similar）
const LARGE_DIFF_THRESHOLD = 400_000;
const diffLoading = ref(false);
const commitMsg = ref("");
const busy = ref(false);
const result = ref<TaskResult | null>(null);
const err = ref<UiError | null>(null);
const showDetail = ref(false);
const remoteAgainst = ref<number | null>(null);
const blamePanel = ref(false);
const blameTitle = ref("");
const blameLines = ref<BlameLine[]>([]);
// 三方合并弹层：非 null 时展示
const mergePath = ref<string | null>(null);
// 通用输入弹窗（替代 window.prompt）
const { promptState, uiPrompt, onPromptOk, onPromptCancel } = usePrompt();
const autoRefreshSec = ref(0);
const patchPanel = ref(false);
const patchMode = ref<"create" | "apply">("create");
const patchText = ref("");

onUnmounted(() => {
  if (autoTimer !== undefined) window.clearInterval(autoTimer);
});

function open(): void {
  const p = path.value.trim();
  if (!p) return;
  load(p);
}

// 远程页检出成功后自动打开（App 切换页并传入目标路径）
watch(
  () => props.initialPath,
  (p) => {
    if (p) {
      path.value = p;
      open();
    }
  },
  { immediate: true },
);

async function load(p: string): Promise<void> {
  loading.value = true;
  err.value = null;
  result.value = null;
  try {
    info.value = await api.wcOpen(p);
    path.value = info.value.wcRoot || p;
    void api.historyAdd("local", path.value); // 打开成功才记录
    checked.value = [];
    selected.value = null;
    diff.value = null;
  } catch (e) {
    info.value = null;
    err.value = normalizeError(e);
  } finally {
    loading.value = false;
  }
}

async function refresh(): Promise<void> {
  if (!info.value) return;
  const p = info.value.wcRoot;
  loading.value = true;
  try {
    info.value = await api.wcOpen(p);
    // 保留仍存在的勾选
    const alive = new Set(info.value.status.map((s) => s.path));
    checked.value = checked.value.filter((c) => alive.has(c));
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    loading.value = false;
  }
}

function relPath(s: StatusEntry): string {
  if (!info.value) return s.path;
  const root = info.value.wcRoot;
  return s.path.startsWith(root) ? s.path.slice(root.length).replace(/^\//, "") : s.path;
}

function toggle(s: StatusEntry): void {
  const i = checked.value.indexOf(s.path);
  if (i >= 0) checked.value.splice(i, 1);
  else checked.value.push(s.path);
}

async function showDiff(s: StatusEntry): Promise<void> {
  selected.value = s;
  diffLoading.value = true;
  err.value = null;
  try {
    const pair = await api.wcFilePair(s.path);
    diffChunks.value =
      pair.oldText.length + pair.newText.length > LARGE_DIFF_THRESHOLD
        ? await api.diffChunks(pair.oldText, pair.newText)
        : null;
    diff.value = pair;
  } catch (e) {
    err.value = normalizeError(e);
    diff.value = null;
    diffChunks.value = null;
  } finally {
    diffLoading.value = false;
  }
}

function closeDiff(): void {
  selected.value = null;
  diff.value = null;
  diffChunks.value = null;
}

async function commit(): Promise<void> {
  if (!info.value || commitMsg.value.trim() === "" || checked.value.length === 0) return;
  if (conflictCount.value > 0) {
    err.value = {
      summary: `有 ${conflictCount.value} 个冲突未解决，无法提交`,
      detail: "请先用「三方合并」或「解决冲突」处理全部冲突。",
      hint: "",
      category: "conflict",
    };
    return;
  }
  busy.value = true;
  err.value = null;
  try {
    result.value = await api.wcCommit(checked.value, commitMsg.value);
    commitMsg.value = "";
    await refresh();
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    busy.value = false;
  }
}

async function run(
  fn: () => Promise<TaskResult>,
  confirmText?: string,
): Promise<void> {
  if (confirmText && !window.confirm(confirmText)) return;
  busy.value = true;
  err.value = null;
  try {
    result.value = await fn();
    await refresh();
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    busy.value = false;
  }
}

/** 更新工作副本（后台任务，不阻塞界面）；完成后自动刷新并提示冲突 */
async function doUpdate(): Promise<void> {
  if (!info.value) return;
  if (!window.confirm("确定更新工作副本？")) return;
  err.value = null;
  try {
    const id = await api.taskUpdate(info.value.wcRoot);
    result.value = {
      ok: true,
      summary: `更新已加入后台任务 #${id}，见底部任务栏`,
      stdout: "",
      stderr: "",
    };
    // 轮询等待任务完成（最长约 120s），完成后刷新并检测冲突
    for (let i = 0; i < 240; i++) {
      await new Promise((r) => setTimeout(r, 500));
      const tasks = await api.taskList();
      const t = tasks.find((x) => x.id === id);
      if (t && (t.state === "done" || t.state === "failed" || t.state === "cancelled")) break;
    }
    await refresh();
    if (conflictCount.value > 0) {
      showConflictGuide();
    }
  } catch (e) {
    err.value = normalizeError(e);
  }
}

function selectedUnversioned(): string[] {
  return checked.value.filter((p) => {
    const s = info.value?.status.find((x) => x.path === p);
    return s?.item === "unversioned";
  });
}

const selectedCount = computed(() => checked.value.length);
// —— 属性弹层 ——
const propsPanel = ref(false);
const propsTarget = ref("");
const propsList = ref<PropEntry[]>([]);
const newPropName = ref("");
const newPropValue = ref("");

async function openProps(): Promise<void> {
  if (checked.value.length !== 1) return;
  propsTarget.value = checked.value[0];
  propsPanel.value = true;
  newPropName.value = "";
  newPropValue.value = "";
  await refreshProps();
}

async function refreshProps(): Promise<void> {
  propsList.value = await api.wcProplist(propsTarget.value);
}

async function saveProp(): Promise<void> {
  const name = newPropName.value.trim();
  if (!name) return;
  await run(
    () => api.wcPropset(propsTarget.value, name, newPropValue.value),
    `设置属性 ${name}？`,
  );
  await refreshProps();
}

async function delProp(name: string): Promise<void> {
  await run(() => api.wcPropdel(propsTarget.value, name), `确定删除属性 ${name}？`);
  await refreshProps();
}

async function lockChecked(): Promise<void> {
  const comment = await uiPrompt("锁定文件", "", "锁定注释（可留空）");
  if (comment === null) return;
  await run(
    () => api.wcLock(checked.value, comment),
    `确定锁定选中的 ${checked.value.length} 个路径？`,
  );
}

async function unlockChecked(force: boolean): Promise<void> {
  await run(
    () => api.wcUnlock(checked.value, force),
    force ? "强制解锁？（用于解除他人锁定的文件）" : "确定解锁选中路径？",
  );
}

async function moveChecked(): Promise<void> {
  if (checked.value.length !== 1) return;
  const src = checked.value[0];
  const name = await uiPrompt(
    `移动/重命名（保留历史）：\n${src}`,
    src.split("/").pop() ?? "",
    "输入新文件名（同目录重命名）或完整目标路径",
  );
  if (!name?.trim() || name.trim() === src) return;
  const dst = name.trim().includes("/")
    ? name.trim()
    : `${src.slice(0, src.lastIndexOf("/") + 1)}${name.trim()}`;
  await run(() => api.wcMove(src, dst), `确定移动 ${src}\n → ${dst}？`);
}

// —— 批次 5：浏览辅助 ——

/** 与服务器比较：svn status -u（标记过期条目 + 显示服务器最新 revision） */
async function compareRemote(): Promise<void> {
  if (!info.value) return;
  busy.value = true;
  err.value = null;
  try {
    const su = await api.wcStatusU(info.value.wcRoot);
    remoteAgainst.value = su.against;
    info.value.status = su.entries;
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    busy.value = false;
  }
}

/** 查看逐行归属 */
async function openBlame(s: StatusEntry): Promise<void> {
  busy.value = true;
  err.value = null;
  try {
    blameLines.value = await api.wcBlame(s.path, null);
    blameTitle.value = relPath(s);
    blamePanel.value = true;
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    busy.value = false;
  }
}

/** 加入变更集 */
async function addToChangelist(): Promise<void> {
  if (checked.value.length === 0) return;
  const name = await uiPrompt("变更集名称");
  if (!name?.trim()) return;
  await run(
    () => api.wcChangelist(name.trim(), checked.value, false),
    `将选中的 ${checked.value.length} 个路径加入变更集「${name.trim()}」？`,
  );
}

/** 从变更集移除 */
async function removeFromChangelist(): Promise<void> {
  if (checked.value.length === 0) return;
  await run(
    () => api.wcChangelist("", checked.value, true),
    `从变更集移除选中的 ${checked.value.length} 个路径？`,
  );
}

/** 按变更集提交 */
async function commitChangelist(): Promise<void> {
  const name = await uiPrompt("要提交的变更集名称");
  if (!name?.trim()) return;
  const msg = await uiPrompt(`变更集「${name.trim()}」的提交说明`);
  if (msg === null || !msg.trim()) return;
  await run(
    () => api.wcCommitCl(name.trim(), msg.trim(), info.value!.wcRoot),
    `确定按变更集「${name.trim()}」提交？`,
  );
}

/** 自动刷新定时器 */
let autoTimer: number | undefined;
function applyAutoRefresh(): void {
  if (autoTimer !== undefined) {
    window.clearInterval(autoTimer);
    autoTimer = undefined;
  }
  const sec = Number(autoRefreshSec.value);
  if (sec > 0 && info.value) {
    autoTimer = window.setInterval(() => void refresh(), sec * 1000);
  }
}

// —— 批次 6：switch / relocate / merge / 补丁 ——

/** 切换分支/URL */
async function switchTo(): Promise<void> {
  if (!info.value) return;
  const url = await uiPrompt(
    "切换分支/URL",
    "",
    `当前：${info.value.url}\n\n目标 URL（分支/标签/其他目录）：`,
  );
  if (!url?.trim()) return;
  const depth = await uiPrompt(
    "稀疏目录深度",
    "",
    "可选，回车跳过：\nempty / files / immediates / infinity",
  );
  const d = depth?.trim() || null;
  await run(
    () => api.wcSwitch(info.value!.wcRoot, url.trim(), d),
    `确定切换到 ${url.trim()}？`,
  );
  await refresh();
}

/** 重定位（服务器地址变更） */
async function relocateWc(): Promise<void> {
  if (!info.value) return;
  const newUrl = await uiPrompt(
    "重定位",
    "",
    `当前：${info.value.url}\n\n新仓库地址（repository root URL 或完整 URL）：`,
  );
  if (!newUrl?.trim()) return;
  const fromUrl = await uiPrompt("旧地址前缀（可选，回车跳过）");
  await run(
    () => api.wcRelocate(info.value!.wcRoot, newUrl.trim(), fromUrl?.trim() || null),
    `确定重定位到 ${newUrl.trim()}？`,
  );
  await refresh();
}

/** 分支间合并 */
async function mergeBranch(): Promise<void> {
  if (!info.value) return;
  const src = await uiPrompt(
    "合并源",
    "",
    `合并到当前工作副本（${info.value.url}）\n\n源 URL（分支/标签）：`,
  );
  if (!src?.trim()) return;
  const range = await uiPrompt(
    "版本范围",
    "",
    "可选，格式 F:T 或直接回车用 mergeinfo 自动合并",
  );
  let revFrom: number | null = null;
  let revTo: number | null = null;
  if (range?.trim()) {
    const m = range.trim().match(/^(\d+):(\d+)$/);
    if (!m) {
      err.value = {
        summary: "版本范围格式错误",
        detail: "应为 F:T（如 1:5）",
        hint: "",
        category: "usage",
      };
      return;
    }
    revFrom = Number(m[1]);
    revTo = Number(m[2]);
  }
  await run(
    () => api.wcMerge(info.value!.wcRoot, src.trim(), revFrom, revTo, false),
    `确定从 ${src.trim()} 合并到当前工作副本？\n（合并结果请检查后再提交）`,
  );
}

/** 创建补丁：svn diff 文本展示 */
async function createPatch(): Promise<void> {
  if (!info.value) return;
  busy.value = true;
  err.value = null;
  try {
    patchText.value = await api.wcDiffText(info.value.wcRoot);
    patchMode.value = "create";
    patchPanel.value = true;
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    busy.value = false;
  }
}

async function copyPatch(): Promise<void> {
  try {
    await navigator.clipboard.writeText(patchText.value);
  } catch {
    // 剪贴板不可用时静默（用户可手动选择复制）
  }
}

/** 应用补丁 */
async function applyPatch(): Promise<void> {
  if (!info.value) return;
  patchText.value = "";
  patchMode.value = "apply";
  patchPanel.value = true;
}

async function doApplyPatch(): Promise<void> {
  if (!info.value) return;
  await run(
    () => api.wcPatchApply(info.value!.wcRoot, patchText.value),
    "确定应用此补丁到工作副本？",
  );
}
const acceptPolicy = ref("working");
// 勾选中的 conflicted 路径（决定“解决冲突”按钮可用性，resolve 只对这些路径执行）
const conflictedChecked = computed(
  () =>
    info.value?.status
      .filter((s) => s.item === "conflicted" && checked.value.includes(s.path))
      .map((s) => s.path) ?? [],
);
const canCommit = computed(
  () =>
    info.value !== null &&
    selectedCount.value > 0 &&
    commitMsg.value.trim() !== "" &&
    conflictCount.value === 0,
);
// 未解决冲突数量（存在任何冲突时禁止提交）
const conflictCount = computed(
  () => (info.value?.status ?? []).filter((s) => s.item === "conflicted").length,
);
// 更新后冲突引导：路径 + 展示名
const conflictGuide = ref<{ path: string; label: string }[]>([]);
function showConflictGuide(): void {
  conflictGuide.value = (info.value?.status ?? [])
    .filter((s) => s.item === "conflicted")
    .map((s) => ({ path: s.path, label: relPath(s) }));
}

function fmtDate(d: string): string {
  if (!d) return "";
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(d);
  return m ? `${m[1]}-${m[2]}-${m[3]} ${m[4]}:${m[5]}` : d;
}
</script>

<template>
  <section class="wc">
    <div class="openbar">
      <input
        v-model="path"
        list="local-history"
        placeholder="工作副本目录路径，如 /Users/you/work/project"
        spellcheck="false"
        @keyup.enter="open"
      />
      <datalist id="local-history">
        <option v-for="h in localHistory" :key="h.value" :value="h.value" />
      </datalist>
      <button :disabled="loading" @click="open">{{ loading ? "打开中…" : "打开" }}</button>
    </div>

    <div v-if="err" class="errbox">
      <span class="errcat">{{ err.category }}</span>
      <b>{{ err.summary }}</b>
      <span class="errhint">{{ err.hint }}</span>
      <button class="link" @click="err = null">关闭</button>
    </div>

    <template v-if="info">
      <div class="infobar">
        <span title="工作副本根">工作副本：{{ info.wcRoot }}</span>
        <span title="仓库 URL">URL：{{ info.url }}</span>
        <span title="基准 revision">revision {{ info.revision ?? "?" }}</span>
        <span v-if="remoteAgainst != null" title="服务器最新 revision">
          服务器最新 r{{ remoteAgainst }}
        </span>
        <span v-if="info.uuid" title="UUID">UUID：{{ info.uuid }}</span>
        <button class="small" :disabled="loading || busy" @click="refresh()">刷新</button>
        <button class="small" :disabled="loading || busy" @click="compareRemote()">
          与服务器比较
        </button>
        <select
          v-model="autoRefreshSec"
          class="small"
          :disabled="busy"
          title="自动刷新间隔"
          @change="applyAutoRefresh"
        >
          <option :value="0">自动刷新：关</option>
          <option :value="15">自动刷新：15 秒</option>
          <option :value="30">自动刷新：30 秒</option>
          <option :value="60">自动刷新：60 秒</option>
        </select>
      </div>

      <div class="columns">
        <div class="left">
          <table class="list">
            <thead>
              <tr>
                <th></th><th>路径</th><th>状态</th><th>属性</th><th>基准</th><th>最后提交</th><th></th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="s in info.status"
                :key="s.path"
                :class="{
                  sel: selected?.path === s.path,
                  conflicted: s.item === 'conflicted',
                  unver: s.item === 'unversioned' || s.item === 'ignored',
                }"
              >
                <td>
                  <input
                    type="checkbox"
                    :checked="checked.includes(s.path)"
                    :disabled="s.item === 'ignored'"
                    @change="toggle(s)"
                  />
                </td>
                <td>
                  <button class="link" @click="showDiff(s)">{{ relPath(s) }}</button>
                  <span v-if="s.wcLocked" title="本地锁定">🔒</span>
                </td>
                <td>{{ STATUS_LABEL[s.item] ?? s.item }}
                  <span
                    v-if="s.lockOwner"
                    class="locktag"
                    :title="`锁定者：${s.lockOwner}\n注释：${s.lockComment || '（无）'}`"
                  >
                    🔒
                  </span>
                  <span
                    v-if="s.reposItem && s.reposItem !== 'none' && s.reposItem !== 'normal'"
                    class="outofdate"
                    title="服务器端状态（与服务器比较后）"
                  >
                    ↻ {{ s.reposItem }}
                  </span>
                </td>                <td>{{ s.props === "modified" ? "属性修改" : "" }}</td>
                <td>{{ s.wcRevision ?? "" }}</td>
                <td>
                  <template v-if="s.commitRevision != null">
                    r{{ s.commitRevision }}{{ s.commitAuthor ? " " + s.commitAuthor : "" }}
                  </template>
                </td>
                <td>
                  <button
                    class="link"
                    :disabled="busy"
                    @click="openBlame(s)"
                  >
                    Blame
                  </button>
                  <button
                    v-if="s.item === 'conflicted'"
                    class="link"
                    :disabled="busy"
                    title="逐块选择本地/远端内容后写回并解决冲突"
                    @click="mergePath = s.path"
                  >
                    三方合并
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
          <div class="actions">
            <span v-if="conflictCount > 0" class="conflictwarn" title="有冲突未解决，提交已被禁用">
              ⚠ {{ conflictCount }} 个冲突未解决
            </span>
            <input
              v-model="commitMsg"
              placeholder="提交说明（必填）"
              class="msg"
              @keyup.enter="commit"
            />
            <button :disabled="!canCommit || busy" @click="commit">
              提交选中（{{ selectedCount }}）
            </button>
            <button
              v-if="selectedUnversioned().length"
              :disabled="busy"
              @click="run(() => api.wcAdd(selectedUnversioned()), `添加 ${selectedUnversioned().length} 个未版本化路径？`)"
            >
              添加选中
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="run(() => api.wcRevert(checked), '确定还原选中路径的本地修改？')"
            >
              还原选中
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="run(() => api.wcDelete(checked), `确定删除选中的 ${selectedCount} 个路径？（本地标记删除，提交后服务端生效）`)"
            >
              删除选中
            </button>
            <button
              :disabled="selectedCount !== 1 || busy"
              @click="openProps"
            >
              属性
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="lockChecked"
            >
              锁定选中
            </button>
            <button
              :disabled="selectedCount !== 1 || busy"
              @click="moveChecked"
            >
              移动/重命名
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="addToChangelist"
            >
              加入变更集
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="removeFromChangelist"
            >
              移除变更集
            </button>
            <button :disabled="busy" @click="commitChangelist">
              提交变更集
            </button>
            <button :disabled="busy" @click="switchTo">
              切换分支/URL
            </button>
            <button :disabled="busy" @click="relocateWc">
              重定位
            </button>
            <button :disabled="busy" @click="mergeBranch">
              合并
            </button>
            <button :disabled="busy" @click="createPatch">
              创建补丁
            </button>
            <button :disabled="busy" @click="applyPatch">
              应用补丁
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="unlockChecked(false)"
            >
              解锁选中
            </button>
            <button
              :disabled="selectedCount === 0 || busy"
              @click="unlockChecked(true)"
            >
              强制解锁
            </button>
            <button
              v-if="conflictedChecked.length > 0"
              :disabled="busy"
              @click="run(() => api.wcResolve(conflictedChecked, acceptPolicy), `确定按「${acceptPolicy}」策略解决选中的 ${conflictedChecked.length} 个冲突路径？`)"
            >
              解决冲突
            </button>
            <select
              v-if="conflictedChecked.length > 0"
              v-model="acceptPolicy"
              :disabled="busy"
              class="msg"
            >
              <option value="working">working（以工作副本为准）</option>
              <option value="mine-full">mine-full（本地版本优先）</option>
              <option value="theirs-full">theirs-full（服务器版本优先）</option>
              <option value="base">base（以 BASE 版本为准）</option>
            </select>
            <button
              :disabled="busy"
              @click="run(() => api.wcCleanup(info!.wcRoot), '确定清理工作副本？（修复中断操作留下的管理锁）')"
            >
              清理
            </button>
            <button
              :disabled="busy"
              @click="run(() => api.wcUpgrade(info!.wcRoot), '确定升级工作副本格式？（仅旧格式工作副本需要）')"
            >
              升级
            </button>
            <button :disabled="busy" @click="doUpdate">更新</button>
          </div>

          <!-- 更新后冲突引导（批次 13） -->
          <div v-if="conflictGuide.length > 0" class="conflictguide">
            <div class="cg-head">
              <b>⚠ 更新后 {{ conflictGuide.length }} 个文件冲突，请先解决再提交</b>
              <button class="small" @click="conflictGuide = []">关闭</button>
            </div>
            <ul class="cg-list">
              <li v-for="g in conflictGuide" :key="g.path">
                <span class="cg-label" :title="g.path">{{ g.label }}</span>
                <button
                  class="small"
                  @click="
                    mergePath = g.path;
                    conflictGuide = [];
                  "
                >
                  三方合并
                </button>
              </li>
            </ul>
            <p class="cg-hint">
              也可在下方针选冲突路径后使用「解决冲突」策略（working / mine-full / theirs-full / base）。
            </p>
          </div>

          <div v-if="result" class="result">
            {{ result.summary }}
            <button class="small" @click="showDetail = !showDetail">
              {{ showDetail ? "收起" : "输出" }}
            </button>
            <pre v-if="showDetail" class="out">{{ result.stdout }}{{ result.stderr }}</pre>
          </div>
        </div>

        <div class="right">
          <div class="pane">
            <div class="pane-title">
              差异
              <span v-if="selected" class="pane-sub">{{ relPath(selected) }}</span>
            </div>
            <div v-if="diffLoading" class="hint">计算中…</div>
            <div v-else-if="diff?.isBinary" class="hint">二进制文件，无法并排显示差异。</div>
            <div v-else-if="diff" class="diffbox">
              <DiffView
                :title="`${relPath(selected!)}${diff.isUnversioned ? '（未版本化，无基线）' : ''}`"
                :old-text="diff.oldText"
                :new-text="diff.newText"
                :chunks="diffChunks"
                :path="selected?.path"
                @close="closeDiff"
              />
            </div>
            <div v-else class="hint">点击左侧文件行查看本地修改差异。</div>
          </div>
        </div>
      </div>

      <!-- 属性弹层 -->
      <div v-if="propsPanel" class="proppanel">
        <div class="proppanel-head">
          <b>属性：{{ propsTarget }}</b>
          <button class="link" @click="propsPanel = false">关闭</button>
        </div>
        <div class="proplist">
          <div v-if="propsList.length === 0" class="propempty">（无属性）</div>
          <div v-for="p in propsList" :key="p.name" class="proprow">
            <div class="proprow-head">
              <b>{{ p.name }}</b>
              <button
                class="link danger"
                :disabled="busy"
                @click="delProp(p.name)"
              >
                删除属性
              </button>
            </div>
            <pre>{{ p.value || "（空值）" }}</pre>
          </div>
        </div>
        <div class="propedit">
          <input v-model="newPropName" class="msg" placeholder="属性名（如 svn:ignore）" />
          <textarea
            v-model="newPropValue"
            rows="4"
            placeholder="属性值（多行，如忽略规则每行一个）"
          ></textarea>
          <button :disabled="busy || !newPropName.trim()" @click="saveProp">
            添加/更新属性
          </button>
        </div>
      </div>

      <!-- Blame 弹层 -->
      <div v-if="blamePanel" class="blamepanel">
        <div class="blamepanel-head">
          <b>逐行归属：{{ blameTitle }}</b>
          <button class="link" @click="blamePanel = false">关闭</button>
        </div>
        <div class="blametable">
          <table>
            <thead>
              <tr><th>行</th><th>版本</th><th>作者</th><th>内容</th></tr>
            </thead>
            <tbody>
              <tr v-for="b in blameLines" :key="b.lineNo">
                <td class="ln">{{ b.lineNo }}</td>
                <td class="rev">r{{ b.revision }}</td>
                <td class="au">{{ b.author }}</td>
                <td class="tx">{{ b.text }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <!-- 三方合并弹层（批次 9） -->
      <MergeDialog
        v-if="mergePath"
        :path="mergePath"
        @close="
          mergePath = null;
          void refresh();
        "
      />

      <!-- 补丁弹层 -->
      <div v-if="patchPanel" class="patchpanel">
        <div class="patchpanel-head">
          <b>{{ patchMode === "create" ? "创建补丁（svn diff）" : "应用补丁（svn patch）" }}</b>
          <div class="spacer"></div>
          <button
            v-if="patchMode === 'create'"
            class="small"
            :disabled="busy"
            @click="copyPatch"
          >
            复制
          </button>
          <button
            v-if="patchMode === 'apply'"
            class="small"
            :disabled="busy || !patchText.trim()"
            @click="doApplyPatch"
          >
            应用
          </button>
          <button class="link" @click="patchPanel = false">关闭</button>
        </div>
        <textarea
          v-model="patchText"
          class="patchbox"
          :readonly="patchMode === 'create'"
          spellcheck="false"
          placeholder="在此粘贴补丁内容（svn patch 格式）…"
        ></textarea>
      </div>
    </template>

    <div v-else class="empty">
      输入 SVN 工作副本目录路径并点击“打开”。（新目录请先在“远程仓库”页 checkout。）
    </div>

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
.locktag {
  cursor: default;
  font-size: 12px;
  margin-left: 4px;
}
.proppanel {
  position: fixed;
  inset: 48px 12% 48px 12%;
  background: #fff;
  border: 1px solid #d0d7de;
  border-radius: 8px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  z-index: 30;
  padding: 12px;
  gap: 10px;
  overflow: auto;
}
.proppanel-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 14px;
}
.proplist {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.proprow {
  border: 1px solid #e5e8eb;
  border-radius: 6px;
  padding: 8px;
}
.proprow-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.proprow pre {
  background: #f6f8fa;
  border-radius: 4px;
  padding: 6px;
  white-space: pre-wrap;
  font-size: 12px;
  margin: 6px 0 0;
}
.propedit {
  display: flex;
  flex-direction: column;
  gap: 6px;
  border-top: 1px solid #e5e8eb;
  padding-top: 10px;
}
.propedit textarea {
  font-family: inherit;
  font-size: 13px;
}
.propempty {
  color: #57606a;
  font-size: 13px;
}
.blamepanel {
  position: fixed;
  inset: 48px 12% 48px 12%;
  background: #fff;
  border: 1px solid #d0d7de;
  border-radius: 8px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  z-index: 30;
  padding: 12px;
  gap: 10px;
  overflow: auto;
}
.blamepanel-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 14px;
}
.blametable {
  overflow: auto;
  flex: 1;
  min-height: 0;
}
.blametable table {
  border-collapse: collapse;
  width: 100%;
  font-size: 12px;
}
.blametable th,
.blametable td {
  border: 1px solid #e5e8eb;
  padding: 3px 8px;
  text-align: left;
  white-space: pre;
}
.blametable td.ln {
  color: #57606a;
  text-align: right;
  user-select: none;
}
.blametable td.rev {
  color: #1a7f37;
  font-weight: 600;
}
.blametable td.au {
  color: #8250df;
}
.outofdate {
  color: #b35900;
  font-size: 12px;
  cursor: default;
}
.patchpanel {
  position: fixed;
  inset: 48px 12% 48px 12%;
  background: #fff;
  border: 1px solid #d0d7de;
  border-radius: 8px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  z-index: 30;
  padding: 12px;
  gap: 10px;
}
.patchpanel-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
}
.patchpanel-head .spacer {
  flex: 1;
}
.patchbox {
  flex: 1;
  min-height: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  white-space: pre;
  resize: none;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  padding: 8px;
  background: #f6f8fa;
}
.wc {
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
  gap: 14px;
  font-size: 12px;
  color: #57606a;
  flex-wrap: wrap;
  align-items: center;
  padding: 6px 8px;
  background: #f6f8fa;
  border-radius: 6px;
}
.small {
  font-size: 12px;
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 4px;
  padding: 1px 8px;
  cursor: pointer;
}
.columns {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(380px, 1fr) minmax(400px, 1.2fr);
  gap: 10px;
}
.left,
.right {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
}
.list {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: block;
}
.list thead {
  position: sticky;
  top: 0;
  background: #f6f8fa;
}
.list th,
.list td {
  text-align: left;
  padding: 4px 8px;
  border-bottom: 1px solid #eaeef2;
  white-space: nowrap;
}
.list tbody tr:hover {
  background: #f0f6ff;
}
.list tr.sel {
  background: #ddf4ff;
}
.list tr.conflicted {
  background: #fff8c5;
}
.list tr.unver {
  color: #8b949e;
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
.actions {
  display: flex;
  gap: 8px;
  padding-top: 8px;
  flex-wrap: wrap;
  align-items: center;
}
.conflictwarn {
  color: #c0392b;
  font-size: 13px;
  font-weight: 600;
}
.conflictguide {
  margin-top: 10px;
  border: 1px solid #d1242f;
  background: #fff5f5;
  border-radius: 8px;
  padding: 10px 12px;
}
.cg-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 13px;
}
.cg-list {
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.cg-list li {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}
.cg-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: ui-monospace, Menlo, monospace;
}
.cg-hint {
  margin: 8px 0 0;
  font-size: 12px;
  color: #8b949e;
}
.actions .msg {
  flex: 1;
  min-width: 200px;
  padding: 6px 10px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
}
.actions button {
  padding: 6px 12px;
  border-radius: 6px;
  border: 1px solid #d0d7de;
  background: #fff;
  cursor: pointer;
  font-size: 13px;
}
.actions button:disabled {
  opacity: 0.5;
  cursor: default;
}
.result {
  font-size: 12px;
  color: #1a7f37;
  padding-top: 6px;
}
.result .out {
  background: #f6f8fa;
  border: 1px solid #d0d7de;
  border-radius: 4px;
  padding: 6px;
  max-height: 140px;
  overflow: auto;
  font-size: 11px;
}
.right .pane {
  display: flex;
  flex-direction: column;
  border: 1px solid #d8dee4;
  border-radius: 6px;
  min-height: 0;
  background: #fff;
  flex: 1;
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
.diffbox {
  flex: 1;
  min-height: 0;
  display: flex;
}
.diffbox > * {
  flex: 1;
  min-width: 0;
}
.hint {
  padding: 12px;
  font-size: 12px;
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
</style>
