import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 开发时前端走 vite 5173，API 走本地 Rust 服务 21600
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:21600",
        changeOrigin: true,
      },
      "/p": {
        target: "http://127.0.0.1:21600",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
