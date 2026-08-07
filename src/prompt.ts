// 通用输入弹窗状态管理（替代 window.prompt：WKWebView 不桥接 prompt）
import { reactive } from "vue";

export interface PromptState {
  visible: boolean;
  title: string;
  message: string;
  initial: string;
  placeholder: string;
  resolve: ((v: string | null) => void) | null;
}

export function usePrompt() {
  const promptState = reactive<PromptState>({
    visible: false,
    title: "",
    message: "",
    initial: "",
    placeholder: "",
    resolve: null,
  });

  /** 弹出输入框，返回输入值；取消返回 null */
  function uiPrompt(
    title: string,
    initial = "",
    message = "",
    placeholder = "",
  ): Promise<string | null> {
    promptState.title = title;
    promptState.message = message;
    promptState.initial = initial;
    promptState.placeholder = placeholder;
    promptState.visible = true;
    return new Promise<string | null>((res) => {
      promptState.resolve = res;
    });
  }

  function onPromptOk(v: string): void {
    promptState.visible = false;
    promptState.resolve?.(v);
    promptState.resolve = null;
  }

  function onPromptCancel(): void {
    promptState.visible = false;
    promptState.resolve?.(null);
    promptState.resolve = null;
  }

  return { promptState, uiPrompt, onPromptOk, onPromptCancel };
}
