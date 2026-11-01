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
  if (process.argv.includes("--panels")) {
    const output = process.env.CHAT_TERMINAL_OUTPUT || path.resolve(root, "../../../.husk/scratch/independent-workspace-panels", mac ? "." : "non-mac");
    await mkdir(output, { recursive: true });
    const layout = () => page.evaluate(() => {
      const chat = document.querySelector(".chat-workspace-card");
      const dock = document.getElementById("workspace-tools-sidebar");
      const panel = document.querySelector(".terminal-panel");
      const surface = document.querySelector(".terminal-panel__surface");
      const main = document.querySelector(".main-col");
      const rect = el => el.getBoundingClientRect().toJSON();
      const alpha = el => {
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext("2d");
        ctx.fillStyle = getComputedStyle(el).backgroundColor;
        ctx.fillRect(0, 0, 1, 1);
        return ctx.getImageData(0, 0, 1, 1).data[3];
      };
      const prefs = window.workspacePanelsFixture.snapshot();
      return {
        chat: rect(chat), dock: rect(dock), panel: rect(panel), surface: rect(surface), main: rect(main),
        wrapperWidth: dock.parentElement.getBoundingClientRect().width,
        radii: [chat, dock, surface].map(el => getComputedStyle(el).borderTopLeftRadius),
        alphas: [chat, dock, surface].map(alpha),
        width: prefs.dock.width, height: prefs.terminal.panelHeight,
        frameAlpha: alpha(document.querySelector(".main-workspace-frame") || main),
        title: rect(main.querySelector(':scope > [data-tauri-drag-region="deep"]')),
      };
    });
    const near = (a, b, message) => assert.ok(Math.abs(a - b) < .1, `${message}: ${a} vs ${b}`);
    const grip = async (handle, state) => {
      const g = await handle.evaluate(el => {
        const r = el.getBoundingClientRect(), css = getComputedStyle(el, "::after");
        const transform = new DOMMatrix(css.transform);
        const percent = (value, size) => value.endsWith("%") ? Number.parseFloat(value) * size / 100 : Number.parseFloat(value);
        const left = el.classList.contains("session-navigation-resize");
        const a = document.querySelector(left ? "#session-navigation" : ".chat-workspace-card").getBoundingClientRect();
        const b = document.querySelector(left ? ".main-col" : "#workspace-tools-sidebar").getBoundingClientRect();
        const reference = document.createElement("span");
        reference.style.color = "var(--husk-fg-muted)";
        el.append(reference);
        const color = getComputedStyle(reference).color;
        reference.remove();
        return { x: r.left + percent(css.left, r.width) + transform.m41 + Number.parseFloat(css.width) / 2,
          y: r.top + percent(css.top, r.height) + transform.m42 + Number.parseFloat(css.height) / 2,
          centerX: r.left + r.width / 2, centerY: r.top + r.height / 2, gutter: (a.right + b.left) / 2,
          width: css.width, height: css.height, color: css.backgroundColor, neutral: color, opacity: Number(css.opacity) };
      });
      assert.ok(Math.abs(g.x - g.centerX) <= .5 && Math.abs(g.y - g.centerY) <= .5, JSON.stringify(g));
      if (mac) assert.ok(Math.abs(g.x - g.gutter) <= .5, JSON.stringify(g));
      assert.equal(g.width, "3px");
      assert.equal(g.height, "26px");
      assert.equal(g.color, g.neutral, "grips use the existing neutral, not accent");
      if (state.includes("idle")) assert.equal(g.opacity, 0, `${state}: grip is hidden`);
      else assert.ok(g.opacity >= .79 && g.opacity <= .81, `${state}: grip has visible feedback`);
      await page.screenshot({ path: path.join(output, `grip-${await handle.getAttribute("aria-label")}-${state}.png`) });
      return g;
    };
    const check = async () => {
      const g = await layout();
      assert.deepEqual(g.radii, mac ? ["14px", "14px", "14px"] : ["0px", "0px", "0px"]);
      if (mac) assert.deepEqual(g.alphas, [255, 255, 255]);
      near(g.dock.left - g.chat.right, mac ? 8 : 0, "dock gutter");
      near(g.surface.top - g.chat.bottom, mac ? 8 : 0, "terminal gutter");
      near(g.dock.width, g.width, "stored dock width is the visible card");
      near(g.surface.height, g.height, "stored terminal height is the visible card");
      near(g.wrapperWidth, g.width + (mac ? 8 : 0), "dock reveal budget");
      near(g.panel.height, g.height + (mac ? 8 : 0), "terminal reveal budget");
      near(g.title.width, g.main.width, "title toolbar spans the whole workspace");
      return g;
    };
    await page.locator("#session-navigation").getByRole("button", { name: "打开终端", exact: true }).click();
    await page.waitForSelector(".xterm-rows");
    await page.waitForTimeout(900);
    await page.evaluate(() => {
      window.__panelHosts = [document.querySelector(".stream-scroll"), document.querySelector(".xterm")];
      window.__panelOpens = window.__terminalMetrics.ipc.filter(c => c.command === "terminal_open").length;
      if (navigator.platform.startsWith("Mac")) document.documentElement.dataset.sidebarGlass = "native";
    });
    const leftHandle = page.getByRole("separator", { name: "调整会话侧栏宽度" });
    const leftWidth = () => page.locator("#session-navigation").evaluate(el => el.getBoundingClientRect().width);
    const sidebarHeaders = [];
    const checkSidebar = async () => {
      const rects = await page.evaluate(() => ["#session-navigation", "#session-navigation > div", ".session-navigation-panel"].map(selector =>
        document.querySelector(selector).getBoundingClientRect().width));
      assert.ok(rects.every(width => Math.abs(width - rects[0]) < .1), JSON.stringify(rects));
      const header = page.locator(".session-navigation-panel > div").first();
      assert.equal(await header.getAttribute("data-tauri-drag-region"), "deep");
      assert.equal(await header.locator(".session-navigation-brand").count(), mac ? 0 : 2);
      assert.equal(await header.locator("img").count(), mac ? 0 : 1);
      assert.equal(await header.getByText("Husk", { exact: true }).count(), mac ? 0 : 1);
      if (!mac) {
        await header.locator("img").waitFor({ state: "visible" });
        await header.getByText("Husk", { exact: true }).waitFor({ state: "visible" });
        assert.equal(await header.locator("img").evaluate(el => el.complete && el.naturalWidth > 0), true);
      }
      const slot = header.locator(".w-\\[55px\\]");
      assert.equal(await slot.count(), mac ? 1 : 0);
      if (mac) near((await slot.boundingBox()).width, 55, "native traffic-light slot");
      for (const tab of ["编程", "工作"]) {
        assert.equal(await page.locator("#session-navigation").getByRole("tab", { name: `切换到${tab}模式`, exact: true }).evaluate(el => {
          const r = el.getBoundingClientRect(), p = el.closest("aside").getBoundingClientRect();
          const slot = el.closest("aside").querySelector(".w-\\[55px\\]")?.getBoundingClientRect();
          return r.left >= (slot?.right ?? p.left) && r.right <= p.right && r.width >= 48 && r.height >= 22
            && el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
        }), true, "sidebar fixed mode actions remain reachable");
      }
      sidebarHeaders.push({ width: rects[0], brandNodes: await header.locator(".session-navigation-brand").count(),
        nativeSlot: mac ? (await slot.boundingBox()).width : 0, hitTestableModes: true });
    };
    const checkSidebarModes = async (state) => {
      const sidebar = page.locator("#session-navigation");
      const build = sidebar.getByRole("tab", { name: "切换到编程模式", exact: true });
      const office = sidebar.getByRole("tab", { name: "切换到工作模式", exact: true });
      await build.hover();
      await page.getByRole("tooltip", { name: /编程 — 在项目中读写代码、运行命令/ }).waitFor();
      await page.screenshot({ path: path.join(output, `sidebar-${state}-tooltip.png`) });
      await page.mouse.move(700, 18);
      await page.getByRole("tooltip", { name: /编程 — 在项目中读写代码、运行命令/ }).waitFor({ state: "hidden" });
      await office.hover();
      await page.getByRole("tooltip", { name: /工作 — 创建与编辑 Word \/ Excel \/ PPT/ }).waitFor();
      await office.click();
      assert.equal(await office.getAttribute("aria-selected"), "true", "pointer invokes the real mode callback");
      await build.focus();
      await page.keyboard.press("Enter");
      assert.equal(await build.getAttribute("aria-selected"), "true", "keyboard invokes the real mode callback");
      assert.equal(await build.evaluate(el => document.activeElement === el), true);
      await page.screenshot({ path: path.join(output, `sidebar-${state}.png`) });
      await page.mouse.move(700, 18);
    };
    await checkSidebar();
    await checkSidebarModes("default-240");
    await page.mouse.move(700, 18);
    await grip(leftHandle, "idle");
    await leftHandle.hover();
    await page.waitForTimeout(180);
    await grip(leftHandle, "hover");
    await page.mouse.move(700, 18);
    await page.waitForTimeout(180);
    await grip(leftHandle, "hover-leave-idle");
    await leftHandle.focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowLeft");
    await page.waitForTimeout(180);
    await grip(leftHandle, "focus");
    await page.evaluate(() => {
      window.__sidebarFitCalls = [];
      const original = window.__terminalTestSession.prototype.fitNow;
      window.__terminalTestSession.prototype.fitNow = function (...args) {
        window.__sidebarFitCalls.push({ held: document.querySelector(".session-navigation-resize").hasAttribute("data-resizing") });
        return original.apply(this, args);
      };
      window.__sidebarResizeCount = window.__terminalMetrics.ipc.filter(c => c.command === "terminal_resize").length;
    });
    await page.locator("[data-terminal-composer] textarea").first().click();
    await page.evaluate(() => { window.__sidebarInput = document.activeElement; });
    const leftBox = await leftHandle.boundingBox();
    await page.mouse.move(leftBox.x + 3, leftBox.y + 100);
    await page.mouse.down();
    await page.mouse.move(leftBox.x + 103, leftBox.y + 100, { steps: 12 });
    await page.waitForTimeout(180);
    await grip(leftHandle, "drag");
    assert.equal(await page.evaluate(() => window.__sidebarFitCalls.filter(c => c.held).length), 0, "sidebar drag freezes xterm fitting until release");
    assert.equal(await page.evaluate(() => window.__terminalMetrics.ipc.filter(c => c.command === "terminal_resize").length - window.__sidebarResizeCount), 0);
    await page.mouse.up();
    await page.waitForTimeout(900);
    assert.equal(await page.evaluate(() => document.activeElement === window.__sidebarInput), true, "pointer resizing preserves the input focus");
    await page.mouse.move(700, 18);
    await page.waitForTimeout(180);
    await grip(leftHandle, "released-idle");
    near(await leftWidth(), 340, "left pointer commits its real width");
    await checkSidebar();
    assert.equal(await page.evaluate(() => window.__terminalMetrics.ipc.filter(c => c.command === "terminal_resize").length - window.__sidebarResizeCount), 1, "sidebar release converges the mock PTY once");
    for (const cancel of ["Escape", "blur", "pointercancel", "lostpointercapture"]) {
      const b = await leftHandle.boundingBox();
      await page.mouse.move(b.x + 3, b.y + 100);
      await page.mouse.down();
      await page.mouse.move(b.x + 43, b.y + 100);
      if (cancel === "Escape") await page.keyboard.press("Escape");
      else await page.evaluate(type => {
        const target = type === "lostpointercapture" ? document.querySelector(".session-navigation-resize") : window;
        target.dispatchEvent(type === "blur" ? new Event(type) : new PointerEvent(type));
      }, cancel);
      await page.mouse.up();
      near(await leftWidth(), 340, `${cancel} restores committed width`);
      assert.equal(await leftHandle.getAttribute("data-resizing"), null);
    }
    await page.getByRole("button", { name: "收起会话侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    near(await leftWidth(), 0, "sidebar close removes all layout width");
    await page.getByRole("separator", { name: "调整会话侧栏宽度", includeHidden: true }).waitFor({ state: "hidden" });
    await page.getByRole("button", { name: "展开会话侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    near(await leftWidth(), 340, "sidebar reopening retains the window's committed width");
    await leftHandle.focus();
    await page.keyboard.press("ArrowLeft");
    near(await leftWidth(), 316, "left keyboard shrinks");
    await page.keyboard.press("ArrowRight");
    near(await leftWidth(), 340, "left keyboard expands");
    await page.keyboard.press("End");
    near(await leftWidth(), 480, "left maximum keeps room for the main workspace");
    await checkSidebar();
    await checkSidebarModes("wide-480");
    await page.setViewportSize({ width: 880, height: 840 });
    await page.waitForTimeout(500);
    near(await leftWidth(), 400, "window resize reclamps the sidebar");
    await page.setViewportSize({ width: 1280, height: 840 });
    await page.waitForTimeout(500);
    await leftHandle.focus();
    await page.keyboard.press("Home");
    near(await leftWidth(), 200, "left minimum preserves the full header");
    await checkSidebar();
    await checkSidebarModes("minimum-200");
    await page.getByRole("button", { name: "收起会话侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    await page.getByRole("button", { name: "展开会话侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    near(await leftWidth(), 200, "minimum width survives collapse and reopen");
    await checkSidebar();
    await checkSidebarModes("reopened-200");
    assert.equal(await leftHandle.getAttribute("aria-valuemin"), "200");
    const minimum = await leftHandle.boundingBox();
    await page.mouse.move(minimum.x + 3, minimum.y + 100);
    await page.mouse.down();
    await page.mouse.move(minimum.x + 43, minimum.y + 100, { steps: 8 });
    await page.mouse.up();
    near(await leftWidth(), 240, "pointer restores the default width from the minimum");
    await checkSidebar();
    const defaultHandle = await leftHandle.boundingBox();
    await page.mouse.move(defaultHandle.x + 3, defaultHandle.y + 100);
    await page.mouse.down();
    await page.mouse.move(defaultHandle.x - 57, defaultHandle.y + 100, { steps: 8 });
    await page.mouse.up();
    near(await leftWidth(), 200, "pointer can shrink below the default and clamps at the minimum");
    await checkSidebar();
    const results = [];
    for (const theme of ["light", "dark", "custom"]) {
      await page.evaluate(value => window.workspacePanelsFixture.theme(value), theme);
      await page.waitForTimeout(300);
      const g = await check();
      await checkSidebar();
      await checkSidebarModes(`${theme}-200`);
      if (mac) assert.ok(Math.abs(g.frameAlpha / 255 - .68) < .01, "gaps expose the 68% panel frame");
      if (mac) {
        for (const [name, state] of [["收起会话侧栏", "collapsed"], ["展开会话侧栏", "expanded"]]) {
          await page.evaluate(() => {
            window.__topFrames = [];
            const start = performance.now();
            const sample = () => {
              const main = document.querySelector(".main-col");
              const title = main.querySelector(':scope > [data-tauri-drag-region="deep"]').getBoundingClientRect();
              const shell = document.querySelector(".main-layout").getBoundingClientRect();
              const toggle = title && main.querySelector('[aria-controls="session-navigation"]').getBoundingClientRect();
              window.__topFrames.push({ top: title.top - shell.top, height: title.height,
                center: title.top - shell.top + title.height / 2, toggleLeft: toggle.left - shell.left });
              if (performance.now() - start < 450) requestAnimationFrame(sample);
            };
            sample();
          });
          await page.getByRole("button", { name, exact: true }).click();
          await page.waitForTimeout(500);
          const frames = await page.evaluate(() => window.__topFrames);
          assert.ok(frames.length > 3);
          for (const frame of frames) {
            near(frame.top, 0, "title top during sidebar animation");
            near(frame.height, 36, "title height during sidebar animation");
            near(frame.center, 18, "title center matches the measured traffic lights");
            assert.ok(frame.toggleLeft >= 72, `sidebar toggle clears traffic lights: ${JSON.stringify(frame)}`);
          }
          await page.screenshot({ path: path.join(output, `top-${theme}-${state}.png`) });
        }
      }
      const attachment = page.getByRole("button", { name: "添加附件", exact: true });
      await attachment.click();
      const menu = page.getByRole("menu");
      await menu.waitFor();
      await page.getByRole("menuitem", { name: "文本文件", exact: true }).hover();
      assert.ok(await page.getByRole("menuitem", { name: "文本文件", exact: true }).evaluate(el => {
        const r = el.getBoundingClientRect();
        return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
      }), "composer menu remains hit-testable outside its card");
      await page.screenshot({ path: path.join(output, `panels-${theme}-menu.png`) });
      await page.keyboard.press("Escape");
      await menu.waitFor({ state: "hidden" });
      const restoredTriggerFocus = await attachment.evaluate(el => document.activeElement === el);
      await attachment.focus();
      assert.equal(await attachment.evaluate(el => document.activeElement === el), true, "composer trigger remains keyboard focusable");
      await attachment.click();
      await menu.waitFor();
      await page.locator("[data-terminal-composer] textarea").first().click();
      await menu.waitFor({ state: "hidden" });
      assert.equal(await page.locator("[data-terminal-composer] textarea").first().evaluate(el => document.activeElement === el), true, "closing the modeless menu preserves the user's chosen input focus");
      results.push({ theme, geometry: g, restoredTriggerFocus });
      await leftHandle.hover();
      await page.waitForTimeout(180);
      await grip(leftHandle, `${theme}-hover`);
      const rightGrip = page.getByRole("separator", { name: "调整侧栏宽度" });
      await rightGrip.hover();
      await page.waitForTimeout(180);
      await grip(rightGrip, `${theme}-hover`);
    }
    await page.evaluate(() => window.workspacePanelsFixture.theme("light"));
    const dockHandle = page.getByRole("separator", { name: "调整侧栏宽度" });
    const terminalHandle = page.getByRole("separator", { name: "调整终端高度" });
    for (const handle of [dockHandle, terminalHandle]) {
      assert.equal(await handle.evaluate(el => {
        const r = el.getBoundingClientRect();
        return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === el;
      }), true, "resize handle is hit-testable in the gap");
    }
    await page.locator("[data-terminal-composer] textarea").first().focus();
    await page.mouse.move(700, 18);
    await page.waitForTimeout(180);
    await grip(dockHandle, "idle");
    await dockHandle.focus();
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    assert.equal(await dockHandle.evaluate(el => document.activeElement === el && el.matches(":focus-visible")), true);
    await page.mouse.move(700, 18);
    await page.waitForTimeout(180);
    await grip(dockHandle, "focus");
    await dockHandle.hover();
    await page.waitForTimeout(180);
    await grip(dockHandle, "hover");
    await page.keyboard.press("ArrowLeft");
    await page.waitForTimeout(350);
    near((await check()).width, 324, "dock keyboard width");
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(350);
    await page.locator("[data-terminal-composer] textarea").first().click();
    const box = await dockHandle.boundingBox();
    await page.mouse.move(box.x + 3, box.y + 80);
    await page.mouse.down();
    const persisted = await page.evaluate(() => localStorage.getItem("husk.dock.v1"));
    await page.mouse.move(box.x - 45, box.y + 80, { steps: 8 });
    await page.waitForTimeout(180);
    await grip(dockHandle, "drag");
    assert.equal(await page.evaluate(() => localStorage.getItem("husk.dock.v1")), persisted, "dock preview does not persist per move");
    await page.mouse.up();
    await page.waitForTimeout(400);
    await page.mouse.move(700, 18);
    await page.waitForTimeout(180);
    await grip(dockHandle, "released-idle");
    near((await check()).width, 348, "dock drag adds to the card width, not the gutter");
    await page.mouse.move(box.x - 45, box.y + 80);
    await page.mouse.down();
    await page.mouse.move(box.x - 65, box.y + 80);
    await page.evaluate(() => window.dispatchEvent(new PointerEvent("pointercancel")));
    await page.mouse.up();
    assert.equal(await dockHandle.getAttribute("data-resizing"), null);
    await terminalHandle.focus();
    await page.keyboard.press("ArrowUp");
    await page.waitForTimeout(400);
    near((await check()).height, 300, "terminal keyboard stores card height");
    const t = await terminalHandle.boundingBox();
    await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2);
    await page.mouse.down();
    await page.mouse.move(t.x + t.width / 2, t.y - 30);
    await page.evaluate(() => document.querySelector(".terminal-panel__separator").dispatchEvent(new PointerEvent("pointercancel", { bubbles: true })));
    await page.mouse.up();
    await page.waitForTimeout(500);
    assert.equal(await page.locator(".terminal-panel").getAttribute("data-resizing"), null);
    await check();
    const textarea = page.locator("[data-terminal-composer] textarea").first();
    await textarea.fill("光标与独立工作面");
    await textarea.press("ArrowLeft");
    assert.equal(await textarea.evaluate(el => el.selectionStart), await textarea.evaluate(el => el.value.length - 1));
    if (mac) {
    await terminalHandle.focus();
    await page.keyboard.press("End");
    await page.waitForTimeout(400);
    await textarea.fill("/");
    const commands = page.getByRole("listbox");
    await commands.waitFor();
    console.log(JSON.stringify(await page.evaluate(() => Object.fromEntries(
      [".chat-workspace-card", "[data-terminal-composer]", "[role=listbox]", "[role=listbox] > div:first-child", "[role=option]"].map(selector =>
        [selector, document.querySelector(selector).getBoundingClientRect().toJSON()])))));
    assert.equal(await commands.getByRole("option").first().evaluate(el => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    }), true, "inline command popup must stay hit-testable with the terminal at its maximum height");
    await page.screenshot({ path: path.join(output, "panels-inline-command-menu.png") });
    await textarea.press("ArrowDown");
    await textarea.press("Escape");
    await commands.waitFor({ state: "hidden" });
    }
    await textarea.fill("光标与独立工作面");
    await terminalHandle.focus();
    await page.keyboard.press("Home");
    await page.waitForTimeout(400);
    near((await check()).height, 120, "terminal keyboard minimum keeps the card height");
    const stream = page.locator(".stream-scroll");
    await stream.evaluate(el => { el.scrollTop = Math.max(0, el.scrollTop - 180); });
    const scrollBefore = await stream.evaluate(el => el.scrollTop);
    await page.mouse.wheel(0, -180);
    await page.waitForTimeout(100);
    assert.ok(await stream.evaluate(el => Number.isFinite(el.scrollTop) && el.scrollHeight > el.clientHeight));
    await page.getByRole("button", { name: "收起工具侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    let g = await layout();
    near(g.wrapperWidth, 0, "closed dock removes its whole gutter");
    near(g.chat.right, g.main.right, "chat fills the closed dock slot");
    await page.getByRole("button", { name: "展开工具侧栏", exact: true }).click();
    await page.waitForTimeout(400);
    for (const width of [960, 1100, 1280]) {
      await page.setViewportSize({ width, height: 840 });
      await page.waitForTimeout(600);
      await check();
      assert.ok(await attachmentVisible(page), "composer action remains reachable at supported widths");
      await page.screenshot({ path: path.join(output, `panels-width-${width}.png`) });
    }
    await page.evaluate(() => window.workspacePanelsFixture.populateDock());
    await page.waitForTimeout(600);
    console.log(JSON.stringify(await page.locator("#workspace-tools-sidebar").evaluate(el => ({
      text: el.innerText, width: el.getBoundingClientRect().width,
      tabs: [...el.querySelectorAll("[role=tab]")].map(tab => ({ label: tab.getAttribute("aria-label"), width: tab.getBoundingClientRect().width })),
    }))));
    const more = page.getByRole("button", { name: "更多工作区标签", exact: true });
    for (const theme of ["light", "dark"]) {
      await page.evaluate(value => window.workspacePanelsFixture.theme(value), theme);
      await more.click();
      const menu = page.getByRole("menu");
      await menu.waitFor();
      await menu.getByRole("menuitem").first().hover();
      assert.equal(await menu.getByRole("menuitem").first().evaluate(el => {
        const r = el.getBoundingClientRect();
        return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
      }), true, "dock overflow menu escapes the rounded card clip");
      await page.keyboard.press("ArrowDown");
      await page.screenshot({ path: path.join(output, `panels-dock-menu-${theme}.png`) });
      await page.keyboard.press("Escape");
      await menu.waitFor({ state: "hidden" });
      await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "更多工作区标签");
    }
    await check();
    assert.deepEqual(await page.evaluate(() => ({
      hosts: [document.querySelector(".stream-scroll"), document.querySelector(".xterm")].every((el, i) => el === window.__panelHosts[i]),
      opens: window.__terminalMetrics.ipc.filter(c => c.command === "terminal_open").length - window.__panelOpens,
    })), { hosts: true, opens: 0 }, "dock/theme/resize retains real renderer hosts and mock PTY");
    await page.getByRole("button", { name: "隐藏终端面板", exact: true }).click();
    await page.waitForTimeout(400);
    g = await layout();
    near(g.panel.height, 0, "closed terminal removes its whole gutter");
    near(g.chat.bottom, g.main.bottom, "chat fills the closed terminal slot");
    for (const state of ["welcome", "empty"]) {
      await page.goto(`${server.resolvedUrls.local[0]}terminal-reveal.html?${state}`);
      await page.locator(".chat-workspace-card").waitFor();
      assert.equal(await page.locator(".chat-workspace-card").evaluate(el => getComputedStyle(el).borderTopLeftRadius), mac ? "14px" : "0px");
      await page.screenshot({ path: path.join(output, `panels-${state}.png`) });
    }
    assert.equal(errors.length, 0);
    const report = { engine, mac, results, sidebarHeaders, retainedHosts: true, extraMockPtys: 0, scrollBefore, errors };
    await writeFile(path.join(output, "panels.json"), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } else if (appearance) {
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
