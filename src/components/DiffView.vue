<script setup lang="ts">
// Beyond Compare 风格并排 diff 视图（@codemirror/merge 双栏模式）
// 特性：A/B 双栏、行内字符级高亮、折叠/展开未变更块、双向同步滚动、差异统计、
//       语法高亮（@codemirror/language-data）、上一处/下一处跳转、外部对比（opendiff）
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Change, Chunk, MergeView } from "@codemirror/merge";
import { EditorView } from "@codemirror/view";
import { languages } from "@codemirror/language-data";
import type { Extension } from "@codemirror/state";
import { api } from "../api";
import { normalizeError } from "../types";
import type { DiffChunk } from "../types";

const props = defineProps<{
  title: string;
  oldText: string | null; // null = 无基线（未版本化新增文件，左侧为空）
  newText: string;
  chunks?: DiffChunk[] | null; // 大文件外部预计算变更块（Rust similar），缺省由 merge 内置算法
  path?: string; // 工作副本文件路径（存在时可外部对比）
}>();

const emit = defineEmits<{ close: [] }>();

const host = ref<HTMLDivElement | null>(null);
const collapsed = ref(true);
const added = ref(0);
const removed = ref(0);
const changed = ref(0);
const navLabel = ref("");
const externalBusy = ref(false);
let mv: MergeView | null = null;
let curChunk = -1;

function build(): void {
  if (!host.value) return;
  host.value.innerHTML = "";
  const extFor = langExt ? [langExt] : [];
  mv = new MergeView({
    parent: host.value,
    a: { doc: props.oldText ?? "", extensions: extFor },
    b: { doc: props.newText, extensions: extFor },
    ...(props.chunks && props.chunks.length > 0
      ? {
          chunks: props.chunks.map(
            (c) =>
              new Chunk(
                [new Change(0, c.toA - c.fromA, 0, c.toB - c.fromB)],
                c.fromA,
                c.toA,
                c.fromB,
                c.toB,
              ),
          ),
        }
      : {}),
    highlightChanges: true,
    gutter: true,
    collapseUnchanged: collapsed.value ? { margin: 4, minSize: 40 } : undefined,
  });
  // 差异统计：chunks 为变更行区间，按左右两侧行数归类
  let add = 0;
  let del = 0;
  for (const c of mv.chunks) {
    const aLines = c.toA - c.fromA;
    const bLines = c.toB - c.fromB;
    if (bLines === 0) del += aLines;
    else if (aLines === 0) add += bLines;
    else {
      del += aLines;
      add += bLines;
    }
  }
  added.value = add;
  removed.value = del;
  changed.value = mv.chunks.length;
  curChunk = -1;
  navLabel.value = "";
  void loadLanguage();
}

let langExt: Extension | null = null;

/** 语法高亮：按文件名/扩展名匹配语言，异步加载后重建应用 */
async function loadLanguage(): Promise<void> {
  const name = (props.title.split("/").pop() ?? props.title).toLowerCase();
  const dot = name.lastIndexOf(".");
  const ext = dot >= 0 ? name.slice(dot + 1) : "";
  const cand = languages.find((l) => {
    const exts = l.extensions ?? [];
    if (ext !== "" && exts.includes(ext)) return true;
    const fn = l.filename;
    return fn != null && fn.test(name);
  });
  if (!cand || langExt) return;
  try {
    const support: Extension = await cand.load();
    langExt = support;
    build(); // 重建以应用语法高亮
  } catch {
    // 语言加载失败不影响 diff
  }
}

/** 跳转到下一处/上一处变更块（右侧 b 编辑器） */
function gotoChunk(dir: 1 | -1): void {
  if (!mv || mv.chunks.length === 0) return;
  curChunk = (curChunk + dir + mv.chunks.length) % mv.chunks.length;
  const c = mv.chunks[curChunk];
  const target = mv.b;
  const lineNo = Math.min(target.state.doc.lines, Math.max(1, c.fromB + 1));
  const pos = target.state.doc.line(lineNo).from;
  target.dispatch({
    selection: { anchor: pos },
    effects: EditorView.scrollIntoView(pos, { y: "center" }),
  });
  navLabel.value = `${curChunk + 1} / ${mv.chunks.length}`;
}

/** 外部对比：导出 BASE/工作区并打开 FileMerge */
async function openExternal(): Promise<void> {
  if (!props.path || externalBusy.value) return;
  externalBusy.value = true;
  try {
    await api.wcDiffExternal(props.path);
  } catch (e) {
    window.alert(normalizeError(e).summary);
  } finally {
    externalBusy.value = false;
  }
}

onMounted(build);
watch(() => [props.oldText, props.newText, props.chunks], build);
watch(collapsed, () => {
  // 折叠切换无需重建：直接 reconfigure
  mv?.reconfigure({ collapseUnchanged: collapsed.value ? { margin: 4, minSize: 40 } : undefined });
});
onBeforeUnmount(() => mv?.destroy());
</script>

<template>
  <div class="dv-wrap">
    <div class="dv-bar">
      <b class="dv-title">{{ title }}</b>
      <span class="dv-stats">
        <span class="stat-add">+{{ added }}</span>
        <span class="stat-del">−{{ removed }}</span>
        <span class="stat-chunk">{{ changed }} 处变更</span>
      </span>
      <span class="dv-spacer" />
      <button class="dv-btn" :disabled="changed === 0" @click="gotoChunk(-1)">上一处</button>
      <button class="dv-btn" :disabled="changed === 0" @click="gotoChunk(1)">下一处</button>
      <span v-if="navLabel" class="dv-nav">{{ navLabel }}</span>
      <button v-if="path" class="dv-btn" :disabled="externalBusy" @click="openExternal" title="用 FileMerge 打开对比">
        {{ externalBusy ? "打开中…" : "外部对比" }}
      </button>
      <button class="dv-btn" @click="collapsed = !collapsed">
        {{ collapsed ? "展开未变更" : "折叠未变更" }}
      </button>
      <button class="dv-btn dv-close" @click="emit('close')">关闭</button>
    </div>
    <div ref="host" class="dv-host" @dblclick="openExternal" />
  </div>
</template>

<style scoped>
.dv-wrap {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: #fff;
}
.dv-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 12px;
  background: #f6f8fa;
  border-bottom: 1px solid #d0d7de;
  font-size: 13px;
}
.dv-title {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 45%;
}
.dv-stats {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.stat-add {
  color: #1a7f37;
  font-weight: 600;
}
.stat-del {
  color: #cf222e;
  font-weight: 600;
}
.stat-chunk {
  color: #57606a;
}
.dv-nav {
  font-size: 12px;
  color: #0969da;
  font-variant-numeric: tabular-nums;
}
.dv-spacer {
  flex: 1;
}
.dv-btn {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 3px 10px;
  font-size: 12px;
  cursor: pointer;
}
.dv-btn:hover {
  background: #eaeef2;
}
.dv-close {
  border-color: #cf222e;
  color: #cf222e;
}
.dv-host {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
.dv-host :deep(.cm-mergeView) {
  min-width: 100%;
  height: 100%;
}
</style>
