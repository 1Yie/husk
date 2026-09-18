import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", target: "es2021", minify: "esbuild" },
  envPrefix: ["VITE_", "TAURI_"],
});
