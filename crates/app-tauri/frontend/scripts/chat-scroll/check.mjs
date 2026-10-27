// npm run test:scroll                 — Chromium
// npm run test:scroll -- --webkit     — WebKit (Tauri/macOS/Linux)
// First install the chosen browser: npx playwright install chromium webkit.
// Optional: CHAT_SCROLL_EXECUTABLE points at an existing browser executable.
// jsdom cannot detect native scroll anchoring or real mixed-row layout.
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium, webkit } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const engine = process.argv.includes("--webkit") ? "webkit" : "chromium";
const server = await createServer({
  configFile: false,
  root,
  cacheDir: "node_modules/.vite-chat-scroll",
  logLevel: "error",
  plugins: [react(), {
    name: "chat-scroll-fixture",
    configureServer(vite) {
      vite.middlewares.use(async (req, res, next) => {
        if (!req.url?.startsWith("/chat-scroll.html")) return next();
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml(req.url, `<html><head></head><body><div id="root"></div><script type="module" src="/scripts/chat-scroll/fixture.tsx"></script></body></html>`));
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
  browser = await (engine === "webkit" ? webkit : chromium).launch({
    headless: true,
    ...(process.env.CHAT_SCROLL_EXECUTABLE ? { executablePath: process.env.CHAT_SCROLL_EXECUTABLE } : {}),
  });
  const page = await browser.newPage({ viewport: { width: 1100, height: 720 } });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto(`${server.resolvedUrls.local[0]}chat-scroll.html`);
  await page.waitForSelector("[data-step-mounted]");
  await page.waitForTimeout(1500);
  const result = await page.evaluate(async () => {
    const root = document.querySelector(".stream-scroll");
    const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
    const count = () => ({
      mounted: document.querySelectorAll("[data-step-mounted]").length,
      placeholders: document.querySelectorAll("[data-step-placeholder]").length,
    });
    const baseline = count();
    const drift = [];
    const frameTimes = [];
    let maxMounted = 0;
    const probe = () => {
      const top = root.getBoundingClientRect().top + 200;
      return [...document.querySelectorAll("[id*='-step-']")].find((el) => {
        const rect = el.getBoundingClientRect();
        return rect.top <= top && rect.bottom > top;
      });
    };
    const wheel = (delta) => root.dispatchEvent(new WheelEvent("wheel", { deltaY: delta, bubbles: true }));
    wheel(-10);
    root.scrollTop = root.scrollHeight * 0.65;
    await new Promise((resolve) => setTimeout(resolve, 1500));
    // For the SAME anchor node, a 4px scroll must move it by exactly 4px.
    // Any additional displacement is the twitch, even if scrollTop itself
    // legitimately changed to compensate an offscreen height correction.
    for (const delta of [-4, 4]) {
      for (let i = 0; i < 300; i++) {
        const target = probe();
        const before = target?.getBoundingClientRect().top;
        const started = performance.now();
        wheel(delta);
        root.scrollTop += delta;
        await frame();
        await frame();
        const after = target?.getBoundingClientRect().top;
        frameTimes.push(performance.now() - started);
        const extra = before != null && after != null ? after - before + delta : 0;
        if (Math.abs(extra) > 1) drift.push({ i, delta, extra, id: target.id });
        maxMounted = Math.max(maxMounted, count().mounted);
      }
    }
    return {
      baseline, maxMounted, final: count(), drift,
      overflowAnchor: getComputedStyle(root).overflowAnchor,
      framesOver70ms: frameTimes.filter((time) => time > 70).length,
    };
  });
  console.log(JSON.stringify({ engine, ...result, pageErrors }, null, 2));
  assert.equal(pageErrors.length, 0, "browser must have no runtime errors");
  assert.equal(result.overflowAnchor, "none", "manual and native anchors must not compete");
  assert.ok(result.baseline.placeholders > 250, "fixture must exceed a single long turn's viewport");
  assert.ok(result.maxMounted < 100, "scrolling must retain real virtualization");
  assert.equal(result.drift.length, 0, "mixed-row scrolling must not twitch");
} finally {
  await browser?.close();
  await server.close();
}
