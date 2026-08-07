<script setup lang="ts">
// 检出对话框（批次 17）：目标目录 + 稀疏深度 + revision
import { ref } from "vue";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";

const props = defineProps<{ url: string }>();
const emit = defineEmits<{ ok: [dest: string, depth: string, rev: string | null]; cancel: [] }>();

const dir = ref("");
const depth = ref("infinity");
const rev = ref("");
const busy = ref(false);

async function pickDir(): Promise<void> {
  const d = await dialogOpen({ directory: true, title: "选择检出目标文件夹" });
  if (d) dir.value = d;
}

function ok(): void {
  const dest = `${dir.value.replace(/\/$/, "")}/${props.url.split("/").filter(Boolean).pop() || "wc"}`;
  emit("ok", dest, depth.value, rev.value.trim() || null);
}
</script>

<template>
  <div class="co-mask" @click.self="emit('cancel')">
    <div class="co-dialog">
      <h3>检出到本地</h3>
      <p class="co-url" :title="url">{{ url }}</p>
      <div class="co-row">
        <input :value="dir" placeholder="目标文件夹（点击右侧选择）" readonly class="co-input" />
        <button :disabled="busy" @click="pickDir">选择…</button>
      </div>
      <div class="co-row">
        <span class="co-label">深度</span>
        <select v-model="depth" class="co-input">
          <option value="infinity">完整（infinity）</option>
          <option value="files">仅文件（files）</option>
          <option value="immediates">直接子项（immediates）</option>
          <option value="empty">仅目录（empty）</option>
        </select>
      </div>
      <div class="co-row">
        <span class="co-label">版本</span>
        <input v-model="rev" placeholder="HEAD（留空取最新）" class="co-input" spellcheck="false" />
      </div>
      <div class="co-btns">
        <button class="co-primary" :disabled="!dir || busy" @click="ok">检出</button>
        <button @click="emit('cancel')">取消</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.co-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 65;
}
.co-dialog {
  background: #fff;
  color: #1f2328;
  border-radius: 8px;
  padding: 16px 20px;
  width: 460px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.3);
}
.co-dialog h3 {
  margin: 0 0 8px;
  font-size: 15px;
}
.co-url {
  font-size: 12px;
  color: #57606a;
  word-break: break-all;
  margin: 0 0 12px;
}
.co-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.co-label {
  width: 40px;
  font-size: 13px;
  color: #57606a;
}
.co-input {
  flex: 1;
  padding: 6px 8px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
  box-sizing: border-box;
}
.co-row button {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 5px 12px;
  cursor: pointer;
  font-size: 13px;
  white-space: nowrap;
}
.co-btns {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 12px;
}
.co-btns button {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 5px 14px;
  cursor: pointer;
  font-size: 13px;
}
.co-btns .co-primary {
  background: #1f883d;
  border-color: #1f883d;
  color: #fff;
}
</style>
