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
  configFile: false, root, cacheDir: "node_modules/.vite-workspace-check", logLevel: "error",
  plugins: [react(), {
    name: "workspace-fixture",
    configureServer(vite) {
      vite.middlewares.use(async (req, res, next) => {
        if (req.url !== "/workspace.html") return next();
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml(req.url, '<html><head><link rel="icon" href="data:,"></head><body><div id="root"></div><script type="module" src="/scripts/terminal-reveal/workspace-fixture.tsx"></script></body></html>'));
      });
    },
  }],
  resolve: { alias: { "@": path.join(root, "src") } },
  server: { host: "127.0.0.1", port: 0 },
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    headless: true,
    ...(process.env.CHAT_TERMINAL_EXECUTABLE ? { executablePath: process.env.CHAT_TERMINAL_EXECUTABLE } : {}),
  });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
  page.on("console", (message) => { if (message.type() === "error") console.error(message.text()); });
  await page.goto(`${server.resolvedUrls.local[0]}workspace.html`);
  await page.getByRole("heading", { name: "工作区工具" }).waitFor();
  await page.evaluate(() => document.fonts.ready);
  for (const mode of ["light", "dark"]) {
    await page.evaluate((mode) => document.documentElement.classList.toggle("dark", mode === "dark"), mode);
    for (const width of [260, 280, 320, 400, 640]) {
      await page.setViewportSize({ width, height: 630 });
      await page.waitForTimeout(100);
      const geometry = await page.locator("[data-dock-tabs-lane]").evaluate((lane) => {
        const rect = lane.getBoundingClientRect();
        const buttons = [...lane.querySelectorAll("button")].map((button) => button.getBoundingClientRect());
        return {
          within: buttons.every((button) => button.left >= rect.left && button.right <= rect.right + 1),
          separated: buttons.every((button, i) => i === 0 || button.left >= buttons[i - 1].right),
          documentOverflow: document.documentElement.scrollWidth > innerWidth,
        };
      });
      assert.deepEqual(geometry, { within: true, separated: true, documentOverflow: false });
      const more = page.getByRole("button", { name: "更多工作区标签" });
      if (width === 280) {
        await more.click();
        const item = page.getByRole("menuitem", { name: /^子代理/ });
        await item.waitFor();
        await item.click();
        const selected = page.getByRole("tab", { name: /^子代理/ });
        await selected.waitFor();
        assert.equal(await selected.getAttribute("aria-selected"), "true");
        await page.waitForFunction(() => document.activeElement?.getAttribute("data-dock-tab") === "subagents");
        await more.focus();
        await page.keyboard.press("Enter");
        await page.getByRole("menu").waitFor();
        await page.keyboard.press("Escape");
        await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "更多工作区标签");
        await page.getByRole("button", { name: "工作区工具" }).click();
      }
      if (width === 640) assert.equal(await more.count(), 0, "wide panels restore all tabs");
      if (width === 320) await page.screenshot({ path: `/tmp/workspace-welcome-${mode}.png` });
    }
  }
  assert.deepEqual(errors, []);
  console.log("Workspace: 10 theme/width checks, overflow selection, keyboard dismissal and focus passed.");
} finally {
  await browser?.close();
  await server.close();
}
