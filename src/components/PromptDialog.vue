<script setup lang="ts">
// 通用输入弹窗（替代 window.prompt：WKWebView 不桥接 prompt，弹窗无法显示）
import { onMounted, ref } from "vue";

const props = defineProps<{
  title: string;
  message?: string;
  initial?: string;
  placeholder?: string;
}>();
const emit = defineEmits<{ ok: [value: string]; cancel: [] }>();

const value = ref(props.initial ?? "");
const input = ref<HTMLInputElement | null>(null);

onMounted(() => {
  input.value?.focus();
  input.value?.select();
});

function ok(): void {
  emit("ok", value.value);
}
function cancel(): void {
  emit("cancel");
}
</script>

<template>
  <div class="pd-mask" @click.self="cancel">
    <div class="pd-dialog">
      <h3 class="pd-title">{{ title }}</h3>
      <p v-if="message" class="pd-msg">{{ message }}</p>
      <input
        ref="input"
        v-model="value"
        class="pd-input"
        :placeholder="placeholder ?? ''"
        spellcheck="false"
        @keyup.enter="ok"
        @keyup.esc="cancel"
      />
      <div class="pd-btns">
        <button class="pd-primary" @click="ok">确定</button>
        <button @click="cancel">取消</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pd-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 70;
}
.pd-dialog {
  background: #fff;
  color: #1f2328;
  border-radius: 8px;
  padding: 16px 20px;
  width: 420px;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.3);
}
.pd-title {
  margin: 0 0 8px;
  font-size: 15px;
}
.pd-msg {
  margin: 0 0 10px;
  font-size: 12px;
  color: #57606a;
  white-space: pre-wrap;
  word-break: break-all;
}
.pd-input {
  width: 100%;
  box-sizing: border-box;
  padding: 7px 10px;
  border: 1px solid #d0d7de;
  border-radius: 6px;
  font-size: 13px;
}
.pd-btns {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 12px;
}
.pd-btns button {
  border: 1px solid #d0d7de;
  background: #fff;
  border-radius: 6px;
  padding: 5px 14px;
  cursor: pointer;
  font-size: 13px;
}
.pd-btns .pd-primary {
  background: #1f883d;
  border-color: #1f883d;
  color: #fff;
}
</style>
