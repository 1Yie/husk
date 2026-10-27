// npm run test:terminal [-- --webkit]
// CHAT_TERMINAL_CPU=4 simulates a slower CPU in Chromium.
// CHAT_TERMINAL_EXECUTABLE optionally selects an existing browser executable.
// Requires: npx playwright install chromium (or webkit).
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium, webkit } from "playwright";
import { checkTerminalResize } from "./resize-check.mjs";
import { checkTerminalContextMenu } from "./context-check.mjs";
import { checkTerminalTabs } from "./tabs-check.mjs";
import { checkTerminalSplits } from "./split-check.mjs";
import { checkTerminalSettings } from "./settings-check.mjs";
import { checkAppearance } from "./appearance-check.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const engine = process.argv.includes("--webkit") ? "webkit" : "chromium";
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-terminal-reveal", logLevel: "error",
  plugins: [react(), {
    name: "terminal-reveal-fixture",
    configureServer(vite) {
      vite.middlewares.use(async (req, res, next) => {
        if (!req.url?.startsWith("/terminal-reveal.html")) return next();
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml(req.url, `<html><head><link rel="icon" href="data:,"></head><body><div id="root"></div><script type="module" src="/scripts/terminal-reveal/fixture.tsx"></script></body></html>`));
      });
    },
  }],
  resolve: { alias: [
    { find: /^@tauri-apps\/api\/(core|event|window)$/, replacement: path.join(here, "tauri.ts") },
    { find: "@", replacement: path.join(root, "src") },
  ] },
  server: { host: "127.0.0.1", port: 0 },
});

let browser;
try {
  await server.listen();
  if (process.argv.includes("--native-webkit")) {
    const child = spawn("python3", [path.join(here, "native-check.py"), `${server.resolvedUrls.local[0]}terminal-reveal.html?resize`], {
      stdio: "inherit",
      env: { ...process.env, GDK_BACKEND: process.env.WAYLAND_DISPLAY ? "wayland" : "x11",
        __NV_DISABLE_EXPLICIT_SYNC: "1", WEBKIT_SKIA_ENABLE_CPU_RENDERING: "1" },
    });
    process.exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code ?? 1));
    });
  } else {
  browser = await (engine === "webkit" ? webkit : chromium).launch({
    headless: true,
    ...(process.env.CHAT_TERMINAL_EXECUTABLE ? { executablePath: process.env.CHAT_TERMINAL_EXECUTABLE } : {}),
  });
  const page = await browser.newPage({ viewport: { width: 1280, height: 840 } });
  const errors = [];
  page.on("console", (message) => {
    if (message.type() === "error") console.error("Browser console:", message.text());
  });
  page.on("pageerror", (error) => {
    errors.push(error.message);
    console.error("Browser error:", error.message);
  });
  await page.addInitScript(() => {
    window.__terminalMetrics = { start: 0, ipc: [], frames: [], chatCommits: [] };
    document.addEventListener("click", (event) => {
      if (window.__terminalMetrics.active) return;
      const button = event.target.closest('[aria-label="打开终端"], [aria-label="隐藏终端"], [aria-label="隐藏终端面板"], .terminal-panel__tab-close');
      if (!button) return;
      const streamHeight = () => document.querySelector(".stream-scroll")?.getBoundingClientRect().height ?? 0;
      const metrics = window.__terminalMetrics = {
        start: performance.now(), ipc: [], frames: [], chatCommits: [],
        opening: button.getAttribute("aria-label") === "打开终端",
        initialStreamHeight: streamHeight(),
        expandedHeight: document.querySelector(".terminal-panel")?.getBoundingClientRect().height ?? 0,
      };
      let previous = metrics.start;
      const sample = () => {
        const now = performance.now();
        const panel = document.querySelector(".terminal-panel");
        const surface = document.querySelector(".terminal-panel__surface")?.getBoundingClientRect();
        metrics.frames.push({
          at: now - metrics.start, gap: now - previous,
          // Measure the real visible terminal and its actual layout bounds.
          height: surface ? Math.max(0, Math.min(surface.height, panel.getBoundingClientRect().bottom - surface.top)) : 0,
          streamHeight: streamHeight(),
          followGap: surface
            ? document.querySelector("[data-terminal-composer]").getBoundingClientRect().bottom - Math.min(panel.getBoundingClientRect().bottom, surface.top)
            : 0,
          geometryGap: surface ? Math.abs(surface.height - panel.getBoundingClientRect().height) : 0,
        });
        previous = now;
        if (now - metrics.start < 700) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    }, true);
  });
  const resizing = process.argv.includes("--resize");
  const tabs = process.argv.includes("--tabs");
  const splits = process.argv.includes("--split");
  const settings = process.argv.includes("--settings");
  const appearance = process.argv.includes("--appearance");
  await page.goto(`${server.resolvedUrls.local[0]}terminal-reveal.html${appearance ? "?appearance" : settings ? "?settings" : resizing || tabs || splits ? "?resize" : ""}`);
  await page.waitForSelector(appearance ? "[data-appearance-fixture]" : settings ? "[data-terminal-settings-fixture] button[aria-pressed]" : ".terminal-panel", { state: "attached", timeout: 10000 }).catch(async (error) => {
    console.error(await page.evaluate(() => document.querySelector("vite-error-overlay")?.shadowRoot?.textContent ?? document.body.innerText));
    throw error;
  });
  await page.waitForTimeout(1500);
  if (engine === "chromium" && Number(process.env.CHAT_TERMINAL_CPU) > 1) {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: Number(process.env.CHAT_TERMINAL_CPU) });
  }
  if (appearance) {
    console.log(JSON.stringify({ engine, ...await checkAppearance(page), errors }, null, 2));
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else if (settings) {
    console.log(JSON.stringify({ engine, ...await checkTerminalSettings(page), errors }, null, 2));
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else if (splits) {
    console.log(JSON.stringify({ engine, ...await checkTerminalSplits(page), errors }, null, 2));
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else if (tabs) {
    await checkTerminalTabs(page);
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else if (process.argv.includes("--menu")) {
    console.log(JSON.stringify({ engine, ...await checkTerminalContextMenu(page), errors }, null, 2));
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else if (resizing) {
    console.log(JSON.stringify({ engine, ...await checkTerminalResize(page), errors }, null, 2));
    assert.equal(errors.length, 0, "no browser runtime errors");
  } else {
    // A cancelled cold reveal must never start a hidden shell.
    await page.locator("#session-navigation").getByRole("button", { name: "打开终端", exact: true }).click();
    await page.waitForTimeout(40);
    await page.getByRole("button", { name: "隐藏终端", exact: true }).click();
    await page.waitForTimeout(450);
    assert.equal(await page.locator(".xterm").count(), 0, "interrupted opening cancels initialization");

    const runs = [];
    const record = async (name, opening, buttonName = opening ? "打开终端" : "隐藏终端面板") => {
      await (opening ? page.locator("#session-navigation") : page).getByRole("button", { name: buttonName, exact: true }).click();
      await page.waitForTimeout(900);
      const run = await page.evaluate(() => ({
        ...window.__terminalMetrics,
        rows: document.querySelector(".xterm-rows")?.children.length,
        height: Number.parseFloat(document.querySelector(".terminal-panel").style.height) || window.__terminalMetrics.expandedHeight,
        shellRenders: Number(document.documentElement.dataset.fixtureRenders),
      }));
      const settledAt = run.frames.find((f) => opening ? f.height >= run.height - 0.5 : f.height < 0.5)?.at;
      let previous = run.initialStreamHeight;
      let layoutChanges = 0;
      for (const frame of run.frames) {
        if (Math.abs(previous - frame.streamHeight) > 0.5) layoutChanges++;
        previous = frame.streamHeight;
      }
      const motionFrames = run.frames.filter((f) => f.at <= settledAt);
      const intermediateFrames = motionFrames.filter((f) => f.height > 2 && f.height < run.height - 2);
      const summary = {
        name, settledPaintMs: settledAt, layoutChanges,
        intermediateFrames: intermediateFrames.length,
        maxGeometryGapPx: Math.max(0, ...run.frames.map((f) => f.geometryGap)),
        chatCommitsDuringMotion: run.chatCommits.filter((at) => at <= settledAt).length,
        maxFollowGapPx: Math.max(0, ...run.frames.filter((f) => f.height > 2 && f.height < run.height - 2).map((f) => Math.abs(f.followGap))),
        animationMaxFrameGapMs: Math.max(...motionFrames.map((f) => f.gap)),
        maxFrameGapMs: Math.max(...run.frames.map((f) => f.gap)),
        opens: run.ipc.filter((call) => call.command === "terminal_open"),
        resizes: run.ipc.filter((call) => call.command === "terminal_resize").length,
        rows: run.rows,
      };
      assert.ok(settledAt < 700, `${name}: slide must finish`);
      assert.ok(summary.maxGeometryGapPx < 1, `${name}: visible terminal and actual layout/hit-test surface must have the same bounds`);
      if (!name.startsWith("reduced-motion")) {
        assert.ok(intermediateFrames.length >= 3, `${name}: must animate through intermediate positions, not disappear instantly`);
        assert.ok(layoutChanges >= 3, `${name}: the real chat viewport must follow the split, not jump after an overlay animation`);
      }
      assert.ok(summary.maxFollowGapPx < 2, `${name}: composer must follow the terminal boundary throughout the slide`);
      assert.ok(summary.chatCommitsDuringMotion <= 2, `${name}: no per-frame chat reconciliation`);
      assert.equal(run.shellRenders, 1, `${name}: terminal state must not render the app shell`);
      runs.push(summary);
      return summary;
    };
    for (let i = 0; i < 2; i++) {
      await record(`open-${i}`, true);
      await record(`close-${i}`, false);
    }
    assert.equal(runs[0].opens.length, 1, "first open starts one PTY");
    assert.ok(runs[0].settledPaintMs < runs[0].opens[0].at, "paint the revealed panel before initialization");
    assert.equal(runs[0].opens[0].rows, runs[0].rows, "PTY must start at the final screen size");
    assert.equal(runs[2].opens.length, 0, "reopening must reuse the existing process");
    for (const run of runs) assert.equal(run.resizes, 0, "sliding must not resize xterm or its PTY");

    // Reverse both directions before either animation can commit geometry.
    for (const name of ["打开终端", "隐藏终端", "打开终端"]) {
      await page.locator("#session-navigation").getByRole("button", { name, exact: true }).click();
      await page.waitForTimeout(60);
    }
    await page.waitForTimeout(500);
    assert.equal(await page.locator(".terminal-panel").getAttribute("data-open"), "true");
    assert.equal(await page.locator(".terminal-panel").getAttribute("data-resizing"), null);
    assert.ok(await page.evaluate(() => Math.abs(
      document.querySelector("[data-terminal-composer]").getBoundingClientRect().bottom
      - document.querySelector(".terminal-panel__surface").getBoundingClientRect().top,
    ) < 0.5), "reversal must not strand chat controls at an obsolete offset");
    await page.getByRole("button", { name: "隐藏终端", exact: true }).click();
    await page.waitForTimeout(400);

    await page.emulateMedia({ reducedMotion: "reduce" });
    await record("reduced-motion-open", true);
    await record("reduced-motion-close", false);
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await record("before-last-tab-close", true);
    await record("last-tab-close", false, "关闭终端 repo");
    assert.equal(await page.locator(".terminal-panel__inner").count(), 0, "remove the empty terminal shell only after its closing animation");
    assert.equal(errors.length, 0, "no browser runtime errors");
    console.log(JSON.stringify({ engine, runs, errors }, null, 2));
  }
  }
} finally {
  await browser?.close();
  await server.close();
}
