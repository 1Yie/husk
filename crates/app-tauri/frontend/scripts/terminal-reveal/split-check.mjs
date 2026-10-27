import assert from "node:assert/strict";

export async function checkTerminalSplits(page) {
  const panel = page.locator(".terminal-panel");
  await page.locator("#session-navigation").getByRole("button", { name: "打开终端", exact: true }).click();
  await page.waitForSelector('[data-pane-key="1"] .xterm-rows');
  await page.waitForTimeout(1000);
  await page.evaluate(async () => {
    window.__splitHosts = new Map([[1, document.querySelector('[data-pane-key="1"] .terminal-panel__screen')]]);
    window.__splitFrame = 0;
    window.__terminalMetrics = { start: performance.now(), ipc: [], fits: [], active: true, chatCommits: [] };
    const url = performance.getEntriesByType("resource").find((entry) => entry.name.includes("/terminal-session.ts"))?.name;
    const { TerminalSession } = await import(url);
    const fit = TerminalSession.prototype.fitNow;
    TerminalSession.prototype.fitNow = function () {
      const at = performance.now() - window.__terminalMetrics.start;
      try { return fit.call(this); } finally {
        window.__terminalMetrics.fits.push({ at, ms: performance.now() - window.__terminalMetrics.start - at, frame: window.__splitFrame });
      }
    };
  });
  const ready = async (key) => {
    await page.waitForSelector(`[data-pane-key="${key}"] .xterm-rows`);
    await page.waitForTimeout(500);
  };
  const sameHosts = () => page.evaluate(() => [...window.__splitHosts].every(([key, host]) =>
    document.querySelector(`[data-pane-key="${key}"] .terminal-panel__screen`) === host));
  const rects = () => page.locator(".terminal-panel__group:not([hidden]) [data-pane-key]").evaluateAll((nodes) =>
    nodes.map((node) => ({ key: Number(node.dataset.paneKey), ...node.getBoundingClientRect().toJSON() })));
  const typeInto = async (key, text) => {
    const before = await page.evaluate(() => window.__terminalMetrics.ipc.filter((call) => call.command === "terminal_write").length);
    await page.locator(`[data-pane-key="${key}"] .xterm-helper-textarea`).focus();
    await page.keyboard.type(text);
    await page.waitForFunction((before) => window.__terminalMetrics.ipc.filter((call) => call.command === "terminal_write").length > before, before);
    return page.evaluate(() => window.__terminalMetrics.ipc.filter((call) => call.command === "terminal_write").at(-1).id);
  };

  await panel.getByRole("button", { name: "左右分屏", exact: true }).click();
  await ready(2);
  assert.equal(await sameHosts(), true, "splitting must retain the original xterm host");
  let boxes = await rects();
  assert.equal(boxes.length, 2);
  assert.ok(Math.abs(boxes[0].right - boxes[1].left) < 1);
  assert.ok(Math.abs(boxes[0].width - boxes[1].width) < 1);
  const rightId = await typeInto(2, "RIGHT");
  const leftId = await typeInto(1, "LEFT");
  assert.notEqual(leftId, rightId, "split panes must send input to different PTYs");
  await page.evaluate(() => window.__splitHosts.set(2, document.querySelector('[data-pane-key="2"] .terminal-panel__screen')));
  await panel.getByRole("button", { name: "上下分屏", exact: true }).click();
  await ready(3);
  const bottomId = await typeInto(3, "BOTTOM");
  assert.equal(new Set([leftId, rightId, bottomId]).size, 3);
  assert.equal(await sameHosts(), true, "nested splitting must not reparent existing hosts");
  boxes = await rects();
  const first = boxes.find((box) => box.key === 1), bottom = boxes.find((box) => box.key === 3);
  assert.ok(Math.abs(first.bottom - bottom.top) < 1 && Math.abs(first.left - bottom.left) < 1);

  const separator = panel.getByRole("separator", { name: "调整终端分屏宽度", exact: true });
  const box = await separator.boundingBox();
  const x = box.x + box.width / 2, y = box.y + box.height / 4;
  await page.mouse.move(x, y);
  await page.evaluate(() => {
    document.addEventListener("pointerdown", (event) => { window.__splitPointer = event.pointerId; }, { once: true, capture: true });
    window.__splitDrag = { start: performance.now() - window.__terminalMetrics.start, frames: [], active: true };
    let previous = performance.now();
    const sample = () => {
      const now = performance.now();
      window.__splitFrame++;
      window.__splitDrag.frames.push({ at: now - window.__terminalMetrics.start, gap: now - previous });
      previous = now;
      if (window.__splitDrag.active) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  await page.mouse.down();
  await page.evaluate(async ({ x, y }) => {
    const handle = document.querySelector('[role="separator"][aria-label="调整终端分屏宽度"]');
    for (let n = 1; n <= 60; n++) {
      handle.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: window.__splitPointer,
        buttons: 1, clientX: x + 90 * n / 60, clientY: y }));
      await new Promise((resolve) => setTimeout(resolve, 8));
    }
    window.__splitDrag.released = performance.now() - window.__terminalMetrics.start;
    handle.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: window.__splitPointer,
      button: 0, clientX: x + 90, clientY: y }));
  }, { x, y });
  await page.mouse.up();
  await page.waitForTimeout(1200);
  const drag = await page.evaluate(() => {
    const data = window.__splitDrag, fits = window.__terminalMetrics.fits;
    data.active = false;
    const held = data.frames.filter((frame) => frame.at <= data.released).map((frame) => frame.gap).sort((a, b) => a - b);
    const trailing = fits.filter((fit) => fit.at >= data.released && fit.at < data.released + 1100);
    return {
      fitsWhileHeld: fits.filter((fit) => fit.at >= data.start && fit.at < data.released).length,
      p95FrameMs: held[Math.floor(held.length * .95)],
      fitsAfterRelease: trailing.length,
      distinctFitFrames: new Set(trailing.map((fit) => fit.frame)).size,
    };
  });
  assert.equal(drag.fitsWhileHeld, 0, "split dragging must not repeatedly reflow scrollback");
  assert.equal(drag.fitsAfterRelease, drag.distinctFitFrames, "separate pane fits must not bunch into one frame");
  assert.equal(await sameHosts(), true);
  await page.screenshot({ path: "/tmp/terminal-split-layout.png" });
  await panel.locator('[data-pane-key="3"]').getByRole("button", { name: "关闭窗格 repo", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[data-pane-key="3"]'));
  assert.equal(await sameHosts(), true, "closing one pane must preserve its siblings");

  await panel.getByRole("button", { name: "新建终端", exact: true }).click();
  await ready(4);
  const newId = await typeInto(4, "NEW");
  const target = panel.getByRole("tab").first();
  await target.click({ button: "right" });
  await page.getByRole("menuitem", { name: "重命名", exact: true }).click();
  await page.getByRole("textbox", { name: "终端标签名称", exact: true }).fill("shells");
  await page.getByRole("textbox", { name: "终端标签名称", exact: true }).press("Enter");
  await panel.getByRole("tab", { name: /^shells/ }).click({ button: "right" });
  const density = await page.getByRole("menuitem").evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).fontSize));
  assert.ok(density.every((size) => size === "12px"), "tab context menu uses the same compact typography");
  await page.getByRole("menuitem", { name: "关闭其他标签", exact: true }).click();
  await page.waitForFunction(() => document.querySelectorAll('.terminal-panel [role="tab"]').length === 1);
  assert.equal(await sameHosts(), true);
  await panel.getByRole("tab").click({ button: "right" });
  await page.getByRole("menuitem", { name: "关闭全部终端", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".terminal-panel").dataset.open === "false");
  const ipc = await page.evaluate(() => window.__terminalMetrics.ipc);
  const closed = ipc.filter((call) => call.command === "terminal_close").map((call) => call.id);
  for (const id of [leftId, rightId, bottomId, newId]) assert.equal(closed.filter((value) => value === id).length, 1);
  assert.equal(ipc.filter((call) => call.command === "terminal_open").length, 3, "splits and new tab create only the requested PTYs");
  return { nestedPanes: 3, retainedHosts: true, independentInput: true, drag, tabMenuDensity: density[0], uniqueClosedPtys: new Set(closed).size };
}
