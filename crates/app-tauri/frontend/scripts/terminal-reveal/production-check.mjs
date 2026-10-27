// Native WebKit against the real ChatPage + xterm, bundled in production mode.
// Only the IPC boundary is simulated. Build artifacts live outside app/dist.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { mkdtemp, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import react from "@vitejs/plugin-react";
import { build } from "vite";
import assert from "node:assert/strict";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const repeat = Number(process.argv.find((arg) => arg.startsWith("--repeat="))?.split("=")[1] ?? 1);
assert.ok(Number.isInteger(repeat) && repeat >= 1 && repeat <= 10, "--repeat must be an integer from 1 to 10");
const reducedMotion = process.argv.includes("--reduced-motion");
const outDir = await mkdtemp(path.join(tmpdir(), "terminal-webkit-"));
try {
  await build({
    configFile: false, root, logLevel: "warn", plugins: [react()],
    resolve: { alias: [
      { find: /^@tauri-apps\/api\/(core|event|window)$/, replacement: path.join(here, "tauri.ts") },
      { find: "@", replacement: path.join(root, "src") },
    ] },
    build: { outDir, emptyOutDir: true, rollupOptions: { input: path.join(here, "index.html") } },
  });
  const url = "husk-terminal-test://localhost/scripts/terminal-reveal/index.html?resize";
  for (let run = 1; run <= repeat; run++) {
    console.log(JSON.stringify({ mode: "production", run, total: repeat, reducedMotion }));
    const child = spawn("python3", [
      path.join(here, "native-check.py"), url, "--asset-root", outDir,
      ...(reducedMotion ? ["--reduced-motion"] : []),
    ], {
      stdio: "inherit",
      env: { ...process.env, GDK_BACKEND: process.env.WAYLAND_DISPLAY ? "wayland" : "x11",
        __NV_DISABLE_EXPLICIT_SYNC: "1", WEBKIT_SKIA_ENABLE_CPU_RENDERING: "1" },
    });
    process.exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code ?? 1));
    });
    // Repetition is a stability check, never a retry-until-green policy.
    if (process.exitCode !== 0) break;
  }
} finally {
  await rm(outDir, { recursive: true, force: true });
}
