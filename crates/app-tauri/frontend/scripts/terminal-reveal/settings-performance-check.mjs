import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
process.chdir(root);
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-terminal-settings-performance", logLevel: "error",
  plugins: [react(), {
    name: "settings-performance",
    configureServer(vite) {
      vite.middlewares.use("/settings-performance", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml("/settings-performance",
          '<link rel="icon" href="data:,"><div id="root"></div><script type="module" src="/scripts/terminal-reveal/settings-performance-fixture.tsx"></script>'));
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
  browser = await chromium.launch({ headless: true, executablePath: process.env.CHAT_TERMINAL_EXECUTABLE });
  const page = await browser.newPage({ viewport: { width: 1100, height: 1100 } });
  const errors = [];
  page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
  page.on("console", (message) => { if (message.type() === "error") console.error(message.text()); });
  await page.addInitScript(() => { window.__terminalMetrics = { start: 0, ipc: [] }; });
  await page.goto(`${server.resolvedUrls.local[0]}settings-performance`);
  await page.locator(".xterm-screen").waitFor();
  for (let i = 1; i < 8; i++) {
    await page.getByRole("button", { name: "新建终端", exact: true }).click();
    await page.waitForFunction((n) => window.__terminalMetrics.ipc.filter((c) => c.command === "terminal_open").length === n, i + 1);
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(300);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  const results = [];
  for (const name of ["Dracula", "Nord", "Solarized 浅色"]) {
    await page.getByRole("button", { name, exact: true }).scrollIntoViewIfNeeded();
    await page.evaluate(() => {
      window.__settingsPerf.applications.length = 0;
      window.__settingsPerf.fits = 0;
      window.__settingsFrames = [];
      window.__settingsMeasuring = true;
      let last = performance.now();
      const tick = () => {
        const now = performance.now();
        window.__settingsFrames.push(now - last);
        last = now;
        if (window.__settingsMeasuring) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await page.getByRole("button", { name, exact: true }).click();
    await page.waitForTimeout(450);
    results.push(await page.evaluate((name) => {
      window.__settingsMeasuring = false;
      return { name, ...window.__settingsPerf,
        totalApplyMs: window.__settingsPerf.applications.reduce((sum, n) => sum + n, 0),
        maxFrameMs: Math.max(...window.__settingsFrames) };
    }, name));
  }
  console.log(JSON.stringify(results, null, 2));
  assert.ok(results.every((run) => run.fits === 0), "palette-only changes must not remeasure terminal geometry");
  assert.ok(results.every((run) => run.applications.length === 1), "hidden terminals must not rebuild their palette on every setting change");
  await page.evaluate(() => {
    window.__settingsPerf.applications.length = 0;
    window.__settingsPerf.fits = 0;
  });
  await page.getByRole("switch", { name: "光标闪烁" }).click();
  await page.waitForTimeout(150);
  const cursor = await page.evaluate(() => ({ ...window.__settingsPerf }));
  assert.equal(cursor.fits, 0, "cursor updates do not affect geometry");
  assert.equal(cursor.applications.length, 1);
  await page.evaluate(() => { window.__settingsPerf.applications.length = 0; window.__settingsPerf.fits = 0; });
  const slider = page.locator('[aria-label="终端字号"] [role="slider"]');
  const initialSize = Number(await slider.getAttribute("aria-valuenow"));
  await slider.focus();
  await slider.press("ArrowRight");
  await page.waitForTimeout(200);
  assert.equal(Number(await slider.getAttribute("aria-valuenow")), initialSize + 1);
  const font = await page.evaluate(() => ({ ...window.__settingsPerf }));
  assert.equal(font.applications.length, 1, "font changes update the visible renderer only");
  assert.equal(font.fits, 1, "one font change results in one geometry fit");
  await page.evaluate(() => { window.__settingsPerf.applications.length = 0; });
  await page.getByRole("tab").first().click();
  await page.waitForTimeout(200);
  const caughtUp = await page.evaluate(() => ({
    calls: window.__settingsPerf.applications.length,
    fontSize: getComputedStyle(document.querySelector('.terminal-panel__view:not([hidden]) .xterm-rows')).fontSize,
    opens: window.__terminalMetrics.ipc.filter((call) => call.command === "terminal_open").length,
    closes: window.__terminalMetrics.ipc.filter((call) => call.command === "terminal_close").length,
  }));
  assert.equal(caughtUp.calls, 1, "a hidden tab catches up once when selected");
  assert.equal(caughtUp.fontSize, `${initialSize + 1}px`, "the newly revealed renderer uses the latest font");
  assert.equal(caughtUp.opens, 8, "settings do not restart terminals");
  assert.equal(caughtUp.closes, 0);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ cursor, font, caughtUp }, null, 2));
} finally {
  await browser?.close();
  await server.close();
}
