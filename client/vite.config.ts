import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri 开发约定（坑位 D4）：固定端口 + 清空 build outDir
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 8618,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    outDir: 'dist',
    // emptyOutDir:false —— vite 不清空 dist、直接覆盖写入。
    // 某些环境下删除目录会被安全钩子拦截，导致 build 失败；旧 hash 产物残留无功能影响。
    emptyOutDir: false,
  },
});
