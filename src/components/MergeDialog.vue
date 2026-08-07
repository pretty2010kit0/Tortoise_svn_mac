<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api } from "../api";
import {
  normalizeError,
  type ConflictBlock,
  type ConflictChoice,
  type UiError,
} from "../types";

const props = defineProps<{ path: string }>();
const emit = defineEmits<{ close: [] }>();

const loading = ref(false);
const err = ref<UiError | null>(null);
const blocks = ref<ConflictBlock[]>([]);
const current = ref(0);
/** 每块选择；null = 尚未处理 */
const choices = ref<(ConflictChoice | null)[]>([]);
const saving = ref(false);
const note = ref("");

watch(
  () => props.path,
  (p) => {
    if (p) void load();
  },
  { immediate: true },
);

async function load(): Promise<void> {
  loading.value = true;
  err.value = null;
  note.value = "";
  try {
    const info = await api.wcConflictParse(props.path);
    blocks.value = info.blocks;
    choices.value = info.blocks.map(() => null);
    current.value = 0;
    if (!info.hasMarkers) {
      err.value = {
        summary: "该冲突没有文本冲突标记",
        detail:
          "可能是属性冲突或二进制冲突，请关闭后使用「解决冲突」策略（mine-full / theirs-full / working / base）。",
        hint: "",
        category: "io",
      };
    }
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    loading.value = false;
  }
}

const block = computed(() => blocks.value[current.value] ?? null);
const doneCount = computed(() => choices.value.filter((c) => c !== null).length);
const allDone = computed(
  () => choices.value.length > 0 && choices.value.every((c) => c !== null),
);
const isUnresolved = computed(() => choices.value[current.value] == null);

function pick(c: ConflictChoice): void {
  choices.value[current.value] = c;
}
function prev(): void {
  if (current.value > 0) current.value -= 1;
}
function next(): void {
  if (current.value < blocks.value.length - 1) current.value += 1;
}

/** 当前块合并预览（未处理时默认展示本地行） */
const blockPreview = computed(() => {
  const b = block.value;
  const c = choices.value[current.value];
  if (!b) return "";
  switch (c) {
    case "mine":
      return b.mine.join("\n");
    case "theirs":
      return b.theirs.join("\n");
    case "both":
      return [...b.mine, ...b.theirs].join("\n");
    case "none":
      return "";
    default:
      return b.mine.join("\n");
  }
});

async function save(): Promise<void> {
  if (!allDone.value || saving.value) return;
  saving.value = true;
  err.value = null;
  try {
    await api.wcConflictResolve(props.path, choices.value as ConflictChoice[]);
    note.value = "已解决冲突";
    emit("close");
  } catch (e) {
    err.value = normalizeError(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="mergedialog">
    <div class="md-head">
      <b>三方合并：{{ props.path.split("/").pop() }}</b>
      <span class="md-path">{{ props.path }}</span>
      <button class="link" :disabled="loading || saving" @click="emit('close')">取消</button>
    </div>

    <div v-if="loading" class="md-loading">解析冲突…</div>
    <div v-else-if="err" class="errbox">
      <span class="errcat">{{ err.category }}</span>
      <b>{{ err.summary }}</b>
      <span>{{ err.detail }}</span>
    </div>
    <template v-else>
      <div class="md-nav">
        <button class="link" :disabled="current === 0" @click="prev">上一处</button>
        <span>
          冲突块 {{ current + 1 }} / {{ blocks.length }}（已处理 {{ doneCount }} /
          {{ blocks.length }}）
        </span>
        <button class="link" :disabled="current >= blocks.length - 1" @click="next">
          下一处
        </button>
      </div>

      <div class="md-cols">
        <div class="md-col">
          <div class="md-col-head">本地（mine）</div>
          <pre class="md-code">{{ block.mine.join("\n") }}</pre>
        </div>
        <div class="md-col">
          <div class="md-col-head">基准（base）</div>
          <pre class="md-code">{{ block.base.join("\n") }}</pre>
        </div>
        <div class="md-col">
          <div class="md-col-head">远端（theirs）</div>
          <pre class="md-code">{{ block.theirs.join("\n") }}</pre>
        </div>
      </div>

      <div class="md-actions">
        <button class="md-btn" :class="{ active: choices[current] === 'mine' }" @click="pick('mine')">
          取本地
        </button>
        <button class="md-btn" :class="{ active: choices[current] === 'theirs' }" @click="pick('theirs')">
          取远端
        </button>
        <button class="md-btn" :class="{ active: choices[current] === 'both' }" @click="pick('both')">
          都留
        </button>
        <button class="md-btn" :class="{ active: choices[current] === 'none' }" @click="pick('none')">
          都删
        </button>
        <span v-if="isUnresolved" class="md-warn">待处理</span>
        <span v-else class="md-ok">已选择</span>
      </div>

      <div class="md-preview">
        <div class="md-col-head">当前块合并预览</div>
        <pre class="md-code preview">{{ blockPreview }}</pre>
        <div v-if="blockPreview === ''" class="md-empty">（该块选择为空）</div>
      </div>

      <div class="md-foot">
        <span v-if="note" class="md-note">{{ note }}</span>
        <button class="md-save" :disabled="!allDone || saving" @click="save">
          {{ saving ? "保存中…" : `保存并解决（${doneCount}/${blocks.length}）` }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.mergedialog {
  position: fixed;
  inset: 48px 10% 48px 10%;
  background: #fff;
  border: 1px solid #d0d7de;
  border-radius: 8px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  z-index: 40;
  padding: 12px;
  gap: 10px;
}
.md-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
}
.md-path {
  flex: 1;
  color: #888;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.md-loading {
  color: #666;
  padding: 20px;
}
.md-nav {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 13px;
}
.md-cols {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 8px;
  flex: 1;
  min-height: 0;
}
.md-col {
  display: flex;
  flex-direction: column;
  border: 1px solid #e5e8eb;
  border-radius: 6px;
  min-height: 0;
}
.md-col-head {
  font-size: 12px;
  font-weight: 600;
  padding: 4px 8px;
  background: #f6f8fa;
  border-bottom: 1px solid #e5e8eb;
  border-radius: 6px 6px 0 0;
}
.md-code {
  flex: 1;
  overflow: auto;
  margin: 0;
  padding: 8px;
  font-size: 12px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: ui-monospace, "SF Mono", Menlo, monospace;
}
.md-code.preview {
  border-top: 1px dashed #d0d7de;
  min-height: 60px;
}
.md-empty {
  color: #aaa;
  font-size: 12px;
  padding: 4px 8px;
}
.md-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.md-btn {
  padding: 5px 14px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  background: #fff;
  font-size: 13px;
  cursor: pointer;
}
.md-btn.active {
  background: #2e5fa3;
  color: #fff;
  border-color: #2e5fa3;
}
.md-warn {
  color: #c0392b;
  font-size: 12px;
  font-weight: 600;
}
.md-ok {
  color: #1a7f37;
  font-size: 12px;
  font-weight: 600;
}
.md-preview {
  border: 1px solid #e5e8eb;
  border-radius: 6px;
  display: flex;
  flex-direction: column;
  max-height: 180px;
}
.md-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}
.md-note {
  color: #1a7f37;
  font-size: 13px;
}
.md-save {
  padding: 6px 18px;
  background: #1a7f37;
  color: #fff;
  border: none;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
}
.md-save:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
