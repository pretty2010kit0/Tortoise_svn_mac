import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri 开发模式下 Vite 端口 1421（与 Git 工具的 1420 错开，避免同时开发冲突）
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2022",
    outDir: "dist",
  },
});
