// Standalone: CHAT_TERMINAL_EXECUTABLE=/usr/bin/google-chrome node scripts/terminal-reveal/tab-strip-check.mjs
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
process.chdir(root);
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-terminal-tab-strip", plugins: [react(), {
    name: "terminal-tab-strip-fixture",
    configureServer(server) {
      server.middlewares.use("/tab-strip", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await server.transformIndexHtml("/tab-strip", '<div id="root"></div><script type="module" src="/scripts/terminal-reveal/tab-strip-fixture.tsx"></script>'));
      });
    },
  }],
  resolve: { alias: {
    "@": path.join(root, "src"),
    "@tauri-apps/api/core": path.join(root, "scripts/terminal-reveal/tauri.ts"),
    "@tauri-apps/api/event": path.join(root, "scripts/terminal-reveal/tauri.ts"),
    "@tauri-apps/api/window": path.join(root, "scripts/terminal-reveal/tauri.ts"),
  } },
  server: { host: "127.0.0.1", port: 0 },
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    executablePath: process.env.CHAT_TERMINAL_EXECUTABLE,
    headless: true, args: ["--no-sandbox"],
  });
  const page = await browser.newPage({ viewport: { width: 720, height: 600 }, hasTouch: true });
  const errors = [];
  page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
  await page.addInitScript(() => { window.__terminalMetrics = { start: 0, ipc: [] }; });
  await page.goto(`${server.resolvedUrls.local[0]}tab-strip`);
  await page.waitForSelector(".xterm-rows").catch(async (error) => {
    console.error(await page.evaluate(() => ({ url: location.href, body: document.body.innerText,
      panel: document.querySelector(".terminal-panel")?.outerHTML.slice(0, 3000), metrics: window.__terminalMetrics })));
    throw error;
  });
  const panel = page.locator(".terminal-panel");
  const tabs = panel.getByRole("tab");
  for (let i = 1; i < 8; i++) {
    await panel.getByRole("button", { name: "新建终端", exact: true }).click();
    await page.waitForFunction((n) => window.__terminalMetrics.ipc.filter((c) => c.command === "terminal_open").length === n, i + 1);
    await page.waitForTimeout(120);
  }
  const viewport = page.locator(".terminal-panel__tabs");
  const fullyVisible = async (tab) => tab.evaluate((button) => {
    const box = button.parentElement.getBoundingClientRect();
    const clip = button.closest('[role="tablist"]').getBoundingClientRect();
    const close = button.parentElement.querySelector(".terminal-panel__tab-close").getBoundingClientRect();
    return box.left >= clip.left - 1 && box.right <= clip.right + 1
      && close.left >= clip.left - 1 && close.right <= clip.right + 1;
  });
  assert.ok(await fullyVisible(tabs.last()), "new selected tab and its close button must be fully visible");
  assert.ok(await viewport.evaluate((node) => node.scrollWidth > node.clientWidth), "fixture must overflow");
  const ids = await tabs.evaluateAll((nodes) => nodes.map((node) => node.id));
  await page.evaluate(() => {
    window.__terminalHosts = [...document.querySelectorAll(".terminal-panel__screen")];
    window.__terminalRenderers = [...document.querySelectorAll(".xterm")];
  });
  const selected = () => panel.locator('[role="tab"][aria-selected="true"]');
  const readIds = () => tabs.evaluateAll((nodes) => nodes.map((node) => node.id));
  const assertRetained = async () => {
    const result = await page.evaluate(() => ({
      hosts: window.__terminalHosts.every((node) => node.isConnected),
      renderers: window.__terminalRenderers.every((node) => node.isConnected),
      opens: window.__terminalMetrics.ipc.filter((c) => c.command === "terminal_open").length,
      closes: window.__terminalMetrics.ipc.filter((c) => c.command === "terminal_close").length,
    }));
    assert.deepEqual(result, { hosts: true, renderers: true, opens: 8, closes: 0 }, "scroll/sort/select must retain renderers and PTYs");
  };
  const scrollLeft = () => viewport.evaluate((node) => node.scrollLeft);
  const waitScroll = async (predicate) => {
    for (let i = 0; i < 30; i++) {
      if (predicate(await scrollLeft())) return;
      await page.waitForTimeout(20);
    }
    assert.ok(predicate(await scrollLeft()), "strip must reach the expected scroll offset");
  };
  const endOffset = await scrollLeft();
  await viewport.hover();
  await page.mouse.wheel(0, -200);
  await waitScroll((x) => x < endOffset - 50);
  const wheelOffset = await scrollLeft();
  await page.mouse.wheel(-120, 0);
  await waitScroll((x) => x < wheelOffset - 50);
  const beforeButton = await scrollLeft();
  await panel.getByRole("button", { name: "向左滚动终端标签" }).click();
  assert.ok(await scrollLeft() < beforeButton);
  const beforeRight = await scrollLeft();
  await panel.getByRole("button", { name: "向右滚动终端标签" }).click();
  assert.ok(await scrollLeft() > beforeRight);
  const wheelModes = await viewport.evaluate((node) => {
    const from = node.scrollLeft;
    const line = new WheelEvent("wheel", { deltaY: -3, deltaMode: 1, bubbles: true, cancelable: true });
    node.dispatchEvent(line);
    const afterLine = node.scrollLeft;
    const zoom = new WheelEvent("wheel", { deltaY: 20, ctrlKey: true, bubbles: true, cancelable: true });
    node.dispatchEvent(zoom);
    const afterZoom = node.scrollLeft;
    const paged = new WheelEvent("wheel", { deltaY: 1, deltaMode: 2, bubbles: true, cancelable: true });
    node.dispatchEvent(paged);
    return { line: line.defaultPrevented && afterLine < from, zoom: !zoom.defaultPrevented && afterZoom === afterLine,
      page: paged.defaultPrevented && node.scrollLeft > afterZoom };
  });
  assert.deepEqual(wheelModes, { line: true, zoom: true, page: true });
  await selected().focus();
  await page.keyboard.press("Home");
  assert.equal(await selected().getAttribute("id"), ids[0]);
  assert.ok(await fullyVisible(tabs.first()));
  assert.equal(await scrollLeft(), 0);
  await page.keyboard.press("End");
  assert.equal(await selected().getAttribute("id"), ids[7]);
  assert.ok(await fullyVisible(tabs.last()));
  await page.keyboard.press("Home");
  // Drag across a sibling midpoint; release must not select/close another tab.
  const firstBox = await tabs.nth(0).boundingBox();
  const secondBox = await tabs.nth(1).boundingBox();
  await page.mouse.move(firstBox.x + 20, firstBox.y + firstBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(secondBox.x + secondBox.width - 8, secondBox.y + secondBox.height / 2, { steps: 8 });
  await page.mouse.up();
  assert.deepEqual(await readIds(), [ids[1], ids[0], ...ids.slice(2)]);
  assert.equal(await selected().getAttribute("id"), ids[0], "drag must preserve selected group");
  assert.equal(await page.getByRole("menu").count(), 0);
  await assertRetained();
  const background = await tabs.nth(0).boundingBox();
  const activeBox = await tabs.nth(1).boundingBox();
  await page.mouse.move(background.x + 20, background.y + background.height / 2);
  await page.mouse.down();
  await page.mouse.move(activeBox.x + activeBox.width - 8, activeBox.y + activeBox.height / 2, { steps: 8 });
  await page.mouse.up();
  assert.deepEqual(await readIds(), ids);
  assert.equal(await selected().getAttribute("id"), ids[0], "dragging a background tab must not select it");
  // Hold at the edge without more pointermove events: auto-scroll must keep going.
  const source = panel.locator(`[id="${ids[0]}"]`);
  const sourceBox = await source.boundingBox();
  const clip = await viewport.boundingBox();
  await page.mouse.move(sourceBox.x + 20, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(clip.x + clip.width - 2, sourceBox.y + sourceBox.height / 2, { steps: 6 });
  const edgeStart = await scrollLeft();
  await page.waitForTimeout(2000);
  assert.ok(await scrollLeft() > edgeStart + 100, "holding at the edge must scroll offscreen drop targets into reach");
  await page.mouse.up();
  assert.equal((await readIds()).at(-1), ids[0]);
  assert.equal(await selected().getAttribute("id"), ids[0]);
  await assertRetained();
  // Dropping outside the strip cancels, then ordinary click and right-click still work.
  const beforeCancel = await readIds();
  const box = await source.boundingBox();
  await page.mouse.move(box.x + 20, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x - 80, box.y - 100, { steps: 6 });
  await page.mouse.up();
  assert.deepEqual(await readIds(), beforeCancel);
  await source.click({ button: "right" });
  await page.getByRole("menuitem", { name: "向左移动标签", exact: true }).click();
  assert.equal((await readIds()).at(-2), ids[0], "single-pointer menu alternative reorders tabs");
  await source.focus();
  await page.keyboard.press("Alt+ArrowLeft");
  assert.equal((await readIds()).at(-3), ids[0]);
  assert.ok(await source.evaluate((node) => document.activeElement === node), "keyboard sorting must retain tab focus");
  await source.click({ button: "right" });
  await page.getByRole("menuitem", { name: "重命名", exact: true }).click();
  const input = panel.getByRole("textbox", { name: "终端标签名称" });
  await input.fill("A very long renamed terminal");
  const beforeRenameKeys = await readIds();
  await input.press("Alt+ArrowLeft");
  assert.deepEqual(await readIds(), beforeRenameKeys, "editing must not trigger tab sorting");
  await input.press("Enter");
  assert.equal(await source.innerText(), "A very long renamed terminal");
  await assertRetained();
  // Real touch input: a long press must not arm Radix's context menu during sorting.
  await source.focus();
  await page.keyboard.press("Home");
  const beforeTouch = await readIds();
  const touchStart = await tabs.nth(0).boundingBox();
  const touchEnd = await tabs.nth(1).boundingBox();
  const cdp = await page.context().newCDPSession(page);
  const touchY = touchStart.y + touchStart.height / 2;
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: touchStart.x + 20, y: touchY }] });
  await page.waitForTimeout(650);
  assert.equal(await page.getByRole("menu").count(), 0, "touch sorting must not open a long-press menu");
  await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: touchEnd.x + touchEnd.width - 8, y: touchY }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
  assert.deepEqual(await readIds(), [beforeTouch[1], beforeTouch[0], ...beforeTouch.slice(2)]);
  await assertRetained();
  await source.focus();
  await page.keyboard.press("Enter");
  // Real layout regression at narrow widths, including the individual close button.
  for (const width of [480, 320, 240, 720]) {
    await page.setViewportSize({ width, height: 600 });
    await page.waitForTimeout(80);
    assert.ok(await fullyVisible(source), `selected label and close button fit at ${width}px`);
    for (const name of ["新建终端", "隐藏终端面板", "终端设置"]) {
      const button = panel.getByRole("button", { name, exact: true });
      const hit = await button.evaluate((node) => {
        const box = node.getBoundingClientRect();
        return box.left >= 0 && box.right <= innerWidth
          && node.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
      });
      assert.ok(hit, `${name} remains fixed and reachable at ${width}px`);
    }
  }
  if (process.env.CHAT_TERMINAL_SCREENSHOT) await page.screenshot({ path: process.env.CHAT_TERMINAL_SCREENSHOT });
  await panel.getByRole("button", { name: "隐藏终端面板" }).click();
  await page.waitForTimeout(300);
  await page.getByRole("button", { name: "打开终端", exact: true }).click();
  await page.waitForTimeout(350);
  assert.ok(await fullyVisible(source), "reopening reveals selected tab");
  await assertRetained();
  // Only intentional close disposes a PTY, and adding again reveals the new tab.
  await source.locator("..").getByRole("button", { name: /关闭终端 / }).click();
  await page.waitForTimeout(120);
  assert.equal(await tabs.count(), 7);
  assert.equal(await page.evaluate(() => window.__terminalMetrics.ipc.filter((c) => c.command === "terminal_close").length), 1);
  await panel.getByRole("button", { name: "新建终端", exact: true }).click();
  assert.ok(await fullyVisible(selected()));
  console.log("PASS: 8 overflowing tabs; wheel/trackpad/line/page scrolling; navigation; auto-reveal; mouse/touch + edge drag sorting; cancellation; menu/keyboard alternatives; rename; 240–720px actions; retained xterm/PTYs; intentional close/new");
  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  await server.close();
}
