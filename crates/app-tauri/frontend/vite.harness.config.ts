import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
export default defineConfig({
  plugins: [react()], clearScreen: false,
  resolve: { alias: [
    { find: /^@tauri-apps\/api\/window$/, replacement: path.resolve(__dirname, "harness-stubs/tauri-window.ts") },
    { find: /^@tauri-apps\/api\/core$/, replacement: path.resolve(__dirname, "harness-stubs/tauri-core.ts") },
    { find: "@", replacement: path.resolve(__dirname, "./src") },
  ] },
  server: { port: 1437, strictPort: true },
});
