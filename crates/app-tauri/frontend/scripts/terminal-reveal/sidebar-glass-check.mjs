import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mkdir } from "node:fs/promises";
import react from "@vitejs/plugin-react";
import { build, createServer } from "vite";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const output = path.resolve(root, "../../..", ".husk/scratch/sidebar-glass-integration");
process.chdir(root);
await mkdir(output, { recursive: true });
process.env.TMPDIR = path.join(output, "tmp");
await mkdir(process.env.TMPDIR, { recursive: true });
const config = {
  configFile: false, root, logLevel: "warn", plugins: [react()],
  cacheDir: path.join(output, "vite-cache"),
  resolve: { alias: { "@": path.join(root, "src") } },
};
if (process.argv.includes("--build")) {
  await build({ ...config, base: "./", build: {
    outDir: path.join(output, "dist"), emptyOutDir: true,
    rollupOptions: { input: path.join(here, "sidebar-glass.html") },
  } });
} else {
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0 } });
  let browser;
  try {
    await server.listen();
    browser = await chromium.launch({ headless: true,
      ...(process.env.CHAT_TERMINAL_EXECUTABLE ? { executablePath: process.env.CHAT_TERMINAL_EXECUTABLE } : {}) });
    const page = await browser.newPage({ viewport: { width: 1120, height: 760 } });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", { value: "MacIntel" });
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${server.resolvedUrls.local[0]}scripts/terminal-reveal/sidebar-glass.html`);
    await page.waitForSelector("html[data-sidebar-glass]");
    const geometry = () => page.evaluate(() => {
      const root = document.documentElement;
      const s = document.getElementById("session-navigation").getBoundingClientRect();
      const main = document.querySelector(".main-col");
      const frame = document.querySelector(".main-workspace-frame");
      const navigationPanel = getComputedStyle(document.querySelector(".session-navigation-panel")).backgroundColor;
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const context = canvas.getContext("2d");
      context.fillStyle = navigationPanel;
      context.fillRect(0, 0, 1, 1);
      return { width: s.width, mainX: main.getBoundingClientRect().x,
        shellRect: document.querySelector(".main-layout").getBoundingClientRect().toJSON(),
        mainRect: main.getBoundingClientRect().toJSON(), frameRect: frame?.getBoundingClientRect().toJSON(),
        frameBackground: frame && getComputedStyle(frame).backgroundColor,
        radius: getComputedStyle(main).borderTopLeftRadius,
        mainBorder: getComputedStyle(main).borderTopWidth,
        card: getComputedStyle(document.getElementById("opaque-workspace-card")).backgroundColor,
        cardRadius: getComputedStyle(document.getElementById("opaque-workspace-card")).borderTopLeftRadius,
        cardBorder: getComputedStyle(document.getElementById("opaque-workspace-card")).borderTopWidth,
        sidebarBorder: getComputedStyle(document.querySelector(".session-navigation-panel")).borderRightWidth,
        headerBorder: getComputedStyle(document.querySelector(".session-navigation-panel").firstElementChild).borderBottomWidth,
        navigationPanel, navigationPixel: Array.from(context.getImageData(0, 0, 1, 1).data),
        surfaces: [root, document.body, document.getElementById("root"), document.querySelector(".main-layout"),
          ...document.querySelectorAll(".session-navigation-surface")].map((e) => getComputedStyle(e).backgroundColor),
        main: getComputedStyle(main).backgroundColor,
        panel: getComputedStyle(document.getElementById("opaque-panel")).backgroundColor };
    });
    const checkFrame = (g) => {
      assert.equal(g.mainX, g.width + 8, JSON.stringify(g));
      for (const [index, delta] of [g.mainRect.left - g.frameRect.left, g.mainRect.top - g.frameRect.top,
        g.frameRect.right - g.mainRect.right, g.frameRect.bottom - g.mainRect.bottom].entries()) {
        assert.ok(Math.abs(delta - (index === 1 ? 0 : 8)) < .1, JSON.stringify(g));
      }
      assert.equal(g.radius, "0px");
      assert.equal(g.mainBorder, "0px");
      assert.equal(g.cardRadius, "14px");
      assert.equal(g.cardBorder, "1px");
      assert.notEqual(g.card, "rgba(0, 0, 0, 0)");
      assert.equal(g.frameBackground, g.navigationPanel);
      assert.equal(g.sidebarBorder, "0px");
      assert.equal(g.headerBorder, "0px");
    };
    const checkGroup = async (solid) => {
      const group = page.getByRole("button", { name: /^Fixture project/ });
      assert.equal(await group.evaluate((e) => e.parentElement.classList.contains("session-navigation-surface")), true);
      const background = await group.evaluate((e) => getComputedStyle(e.parentElement).backgroundColor);
      assert.equal(background, solid ? (await geometry()).panel : "rgba(0, 0, 0, 0)");
      await group.hover();
      assert.notEqual(await group.evaluate((e) => getComputedStyle(e).backgroundColor), "rgba(0, 0, 0, 0)");
      await group.focus();
      assert.equal(await group.evaluate((e) => e === document.activeElement), true);
      await page.keyboard.press("Enter");
      await page.getByRole("button", { name: /^Session 80/ }).waitFor({ state: "hidden" });
      await page.keyboard.press("Enter");
      await page.getByRole("button", { name: /^Session 80/ }).waitFor({ state: "attached" });
      const row = page.getByRole("button", { name: /^Session 02/ }).first();
      await row.click();
      await row.hover();
      await row.getByRole("button", { name: "会话操作" }).click();
      const menu = page.getByRole("menu");
      await menu.waitFor();
      assert.notEqual(await menu.evaluate((e) => getComputedStyle(e).backgroundColor), "rgba(0, 0, 0, 0)");
      await page.getByRole("menuitem", { name: "重命名" }).hover();
      await page.keyboard.press("ArrowDown");
      assert.ok(await menu.locator("[data-highlighted]").count() > 0);
      await page.keyboard.press("Escape");
      await menu.waitFor({ state: "hidden" });
    };
    for (const theme of ["light", "dark", "custom"]) {
      await page.evaluate((value) => window.glassFixture.theme(value), theme);
      await page.waitForTimeout(100);
      const g = await geometry();
      assert.equal(g.width, 240);
      checkFrame(g);
      assert.ok(g.surfaces.every((v) => v === "rgba(0, 0, 0, 0)"), JSON.stringify(g));
      assert.equal(g.main, "rgba(0, 0, 0, 0)");
      assert.notEqual(g.panel, "rgba(0, 0, 0, 0)");
      const tint = await page.evaluate(() => window.glassFixture.metrics.calls.at(-1).surface.background);
      const surface = await page.evaluate(() => window.glassFixture.metrics.calls.at(-1).surface);
      assert.equal(surface.width, g.shellRect.width);
      assert.equal(surface.height, g.shellRect.height);
      assert.equal(surface.x, g.shellRect.x);
      assert.equal(surface.y, g.shellRect.y);
      const panel = g.panel.match(/[\d.]+/g).slice(0, 3).map(Number);
      assert.ok(tint.every((v, i) => Math.abs(v * 255 - panel[i]) < 1), JSON.stringify({ tint, panel }));
      assert.ok(Math.abs(g.navigationPixel[3] / 255 - .85) < .01, JSON.stringify(g));
      assert.ok(panel.every((v, i) => Math.abs(g.navigationPixel[i] - v) <= 1), JSON.stringify(g));
      assert.equal(await page.locator(".session-navigation-panel").count(), 1);
      await checkGroup(false);
      await page.screenshot({ path: path.join(output, `visual-browser-${theme}.png`) });
      if (theme !== "custom") {
        await page.evaluate(() => window.glassFixture.setMode("unsupported"));
        await page.waitForSelector("html:not([data-sidebar-glass])");
        const solid = await geometry();
        assert.equal(solid.navigationPanel, solid.panel);
        assert.equal(solid.navigationPixel[3], 255);
        checkFrame(solid);
        await checkGroup(true);
        await page.screenshot({ path: path.join(output, `visual-browser-${theme}-solid.png`) });
        await page.evaluate(() => window.glassFixture.setMode("active"));
        await page.waitForSelector("html[data-sidebar-glass]");
      }
    }
    const beforeSidebarDrag = await page.evaluate(() => window.glassFixture.metrics.calls.length);
    const sidebarHandle = page.getByRole("separator", { name: "调整会话侧栏宽度" });
    const sidebarHandleBox = await sidebarHandle.boundingBox();
    await page.mouse.move(sidebarHandleBox.x + 3, sidebarHandleBox.y + 80);
    await page.mouse.down();
    await page.mouse.move(sidebarHandleBox.x + 63, sidebarHandleBox.y + 80, { steps: 10 });
    await page.mouse.up();
    await page.waitForTimeout(300);
    assert.equal((await geometry()).width, 300);
    await sidebarHandle.focus();
    await page.keyboard.press("Home");
    assert.equal((await geometry()).width, 200);
    const minimumHandleBox = await sidebarHandle.boundingBox();
    await page.mouse.move(minimumHandleBox.x + 3, minimumHandleBox.y + 80);
    await page.mouse.down();
    await page.mouse.move(minimumHandleBox.x + 43, minimumHandleBox.y + 80, { steps: 8 });
    await page.mouse.up();
    assert.equal((await geometry()).width, 240);
    await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => window.glassFixture.metrics.calls.length), beforeSidebarDrag,
      "sidebar width changes leave full-window native geometry untouched");
    const row = page.getByRole("button", { name: /^Session 02/ }).first();
    await row.click();
    await page.waitForFunction(() => window.glassFixture.selected() === 2);
    await row.hover();
    await row.getByRole("button", { name: "会话操作" }).click();
    await page.getByRole("menuitem", { name: "重命名" }).hover();
    await page.screenshot({ path: path.join(output, "browser-menu.png") });
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Escape");
    await page.getByRole("menu").waitFor({ state: "hidden" });
    const scroll = page.getByTestId("sidebar-project-scroll");
    await scroll.hover();
    await page.mouse.wheel(0, 400);
    await page.waitForFunction(() => document.querySelector('[data-testid="sidebar-project-scroll"]').scrollTop > 100);
    await scroll.focus();
    await page.keyboard.press("Home");
    await page.waitForTimeout(200);
    await page.locator("#main-input").fill("Keyboard focus stays in the main WebView");
    await page.keyboard.press("Control+Shift+J");
    await page.evaluate(() => window.glassFixture.setDelay(45));
    const before = await page.evaluate(() => window.glassFixture.metrics.calls.length);
    const gaps = await page.evaluate(async () => {
      const sample = window.glassFixture.sample();
      window.glassFixture.toggle();
      return sample;
    });
    await page.waitForFunction(() => document.getElementById("session-navigation").getBoundingClientRect().width === 0);
    checkFrame(await geometry());
    assert.equal(await page.evaluate(() => window.glassFixture.metrics.calls.length), before,
      "collapsing navigation preserves full-window glass without native geometry updates");
    await page.screenshot({ path: path.join(output, "frame-collapsed.png") });
    await page.setViewportSize({ width: 780, height: 560 });
    await page.locator("#toggle-sidebar").click();
    await page.waitForFunction(() => document.getElementById("session-navigation").getBoundingClientRect().width === 240);
    await page.waitForTimeout(150);
    checkFrame(await geometry());
    await page.screenshot({ path: path.join(output, "frame-narrow.png") });
    const after = await page.evaluate(() => window.glassFixture.metrics.calls.length);
    assert.equal(after - before, 1, "only the viewport resize changes full-window native geometry");
    await page.waitForTimeout(250);
    assert.equal(await page.evaluate(() => window.glassFixture.metrics.calls.length), after, "idle produces no IPC");
    console.log(JSON.stringify(await page.evaluate(() => window.glassFixture.metrics)));
    assert.equal(await page.evaluate(() => window.glassFixture.metrics.maxActive), 1);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.locator("#toggle-sidebar").click();
    await page.waitForFunction(() => document.getElementById("session-navigation").getBoundingClientRect().width === 0);
    assert.equal(await page.locator("#session-navigation").evaluate((e) => getComputedStyle(e).transitionProperty), "none");
    await page.locator("#toggle-sidebar").click();
    for (const mode of ["unsupported", "failure"]) {
      await page.evaluate((mode) => window.glassFixture.setMode(mode), mode);
      await page.waitForSelector("html:not([data-sidebar-glass])");
      const g = await geometry();
      assert.ok(g.surfaces.every((v) => v !== "rgba(0, 0, 0, 0)"), JSON.stringify(g));
    }
    await page.evaluate(() => window.glassFixture.setMode("active"));
    await page.waitForSelector("html[data-sidebar-glass]");
    await page.evaluate(() => window.glassFixture.mount(false));
    await page.waitForSelector("html:not([data-sidebar-glass])");
    await page.waitForTimeout(150);
    assert.equal(await page.evaluate(() => window.glassFixture.metrics.calls.at(-1).surface), null);
    assert.deepEqual(errors, []);
    const flat = await browser.newPage({ viewport: { width: 780, height: 560 } });
    await flat.addInitScript(() => {
      Object.defineProperty(navigator, "platform", { value: "Linux x86_64" });
      Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)" });
    });
    await flat.goto(`${server.resolvedUrls.local[0]}scripts/terminal-reveal/sidebar-glass.html`);
    await flat.waitForSelector(".main-col");
    for (const width of [240, 0]) {
      await flat.waitForFunction((width) => document.getElementById("session-navigation").getBoundingClientRect().width === width, width);
      const g = await flat.evaluate(() => {
        const main = document.querySelector(".main-col"), panel = document.querySelector(".session-navigation-panel");
        return { rect: main.getBoundingClientRect().toJSON(), frame: !!document.querySelector(".main-workspace-frame"),
          radius: getComputedStyle(main).borderTopLeftRadius, sidebarBorder: getComputedStyle(panel).borderRightWidth,
          headerBorder: getComputedStyle(panel.firstElementChild).borderBottomWidth,
          calls: window.glassFixture.metrics.calls.length };
      });
      assert.equal(g.frame, false);
      assert.equal(g.rect.left, width);
      assert.equal(g.rect.top, 0);
      assert.equal(g.rect.right, 780);
      assert.equal(g.rect.bottom, 560);
      assert.equal(g.radius, "0px");
      assert.equal(g.sidebarBorder, "1px");
      assert.equal(g.headerBorder, "1px");
      assert.equal(g.calls, 0);
      if (width) await flat.locator("#toggle-sidebar").click();
    }
    await flat.close();
    const sorted = [...gaps].sort((a, b) => a - b);
    console.log(JSON.stringify({ result: "PASS", ipc: after - before, maxInFlight: 1,
      frameP95Ms: sorted[Math.floor(sorted.length * .95)], idleIPC: 0,
      boundary: "mock native IPC; real React components, CSS, menus and input" }));
  } finally {
    await browser?.close();
    await server.close();
  }
}
