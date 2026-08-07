<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { api } from "../api";
import type { TaskInfo, TaskState } from "../types";

const tasks = ref<TaskInfo[]>([]);
const expanded = ref<number | null>(null);
const nowMs = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;

async function refresh(): Promise<void> {
  nowMs.value = Date.now();
  try {
    tasks.value = await api.taskList();
  } catch {
    // 任务列表不可用（如未初始化）时静默
  }
}

function fmtElapsed(startedAt: number): string {
  const s = Math.floor(Math.max(0, nowMs.value - startedAt) / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

onMounted(() => {
  void refresh();
  timer = setInterval(() => void refresh(), 1000);
});

onUnmounted(() => {
  if (timer) clearInterval(timer);
});

async function cancel(id: number): Promise<void> {
  const ok = await api.taskCancel(id);
  if (!ok) void refresh();
}

/** 重试失败/取消的任务 */
async function retry(id: number): Promise<void> {
  try {
    const newId = await api.taskRetry(id);
    void refresh();
    window.alert(`已重新加入后台任务 #${newId}，见底部任务栏`);
  } catch (e) {
    window.alert(String(e));
  }
}

function stateText(s: TaskState): string {
  switch (s) {
    case "running":
      return "进行中";
    case "done":
      return "完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
  }
}

function stateIcon(s: TaskState): string {
  switch (s) {
    case "running":
      return "⟳";
    case "done":
      return "✓";
    case "failed":
      return "✗";
    case "cancelled":
      return "⊘";
  }
}
</script>

<template>
  <footer class="taskbar">
    <span class="tbar-title">任务</span>
    <span v-if="tasks.length === 0" class="tbar-empty">无后台任务</span>
    <div
      v-for="t in tasks.slice(0, 5)"
      :key="t.id"
      class="titem"
      :class="t.state"
      :title="t.desc"
    >
      <span class="ticon" :class="t.state">{{ stateIcon(t.state) }}</span>
      <span class="tdesc">{{ t.desc }}</span>
      <span class="tres">
        {{
          t.state === "done"
            ? (t.result ?? "完成")
            : t.state === "failed"
              ? "失败"
              : t.state === "cancelled"
                ? "已取消"
                : `进行中… ${fmtElapsed(t.startedAt)}`
        }}
      </span>
      <button v-if="t.state === 'running'" class="link" @click="cancel(t.id)">
        取消
      </button>
      <button
        v-if="t.state === 'failed' || t.state === 'cancelled'"
        class="link"
        @click="retry(t.id)"
      >
        重试
      </button>
      <button
        v-if="t.state === 'failed'"
        class="link"
        @click="expanded = expanded === t.id ? null : t.id"
      >
        {{ expanded === t.id ? "收起" : "详情" }}
      </button>
    </div>
    <pre v-if="expanded !== null" class="terr">{{
      tasks.find((t) => t.id === expanded)?.output ?? ""
    }}</pre>
  </footer>
</template>

<style scoped>
.taskbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 12px;
  background: #1c2128;
  color: #c9d1d9;
  font-size: 12px;
  border-top: 1px solid #30363d;
  min-height: 28px;
  position: relative;
}
.tbar-title {
  color: #8b949e;
  white-space: nowrap;
}
.tbar-empty {
  color: #6e7681;
}
.titem {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 8px;
  border-radius: 5px;
  background: #21262d;
  max-width: 340px;
  white-space: nowrap;
}
.titem.running {
  background: #1f3a5f;
}
.titem.failed {
  background: #3d1f1f;
}
.titem.cancelled {
  opacity: 0.6;
}
.ticon.running {
  color: #58a6ff;
  animation: spin 1.2s linear infinite;
  display: inline-block;
}
.ticon.done {
  color: #3fb950;
}
.ticon.failed {
  color: #f85149;
}
.ticon.cancelled {
  color: #8b949e;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.tdesc {
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 200px;
}
.tres {
  color: #8b949e;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 120px;
}
button.link {
  background: none;
  border: none;
  color: #58a6ff;
  cursor: pointer;
  font-size: 12px;
  padding: 0 2px;
}
.terr {
  position: absolute;
  right: 12px;
  bottom: 30px;
  max-width: 70%;
  max-height: 200px;
  overflow: auto;
  background: #161b22;
  border: 1px solid #30363d;
  border-radius: 6px;
  padding: 8px 10px;
  color: #f85149;
  font-size: 11px;
  white-space: pre-wrap;
  z-index: 40;
}
</style>
