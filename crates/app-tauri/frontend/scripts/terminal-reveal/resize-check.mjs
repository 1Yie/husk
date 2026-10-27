import assert from "node:assert/strict";

export async function checkTerminalResize(page) {
  await page.locator("#session-navigation").getByRole("button", { name: "打开终端", exact: true }).click();
  await page.waitForSelector(".xterm-rows");
  await page.waitForTimeout(1600);
  // Instrument the real project renderer, not mocked dimensions. No tracing
  // code is installed in production.
  await page.evaluate(async () => {
    const url = performance.getEntriesByType("resource").find((entry) => entry.name.includes("/terminal-session.ts"))?.name;
    const { TerminalSession } = await import(url);
    const original = TerminalSession.prototype.fitNow;
    TerminalSession.prototype.fitNow = function () {
      const start = performance.now();
      try { return original.call(this); } finally {
        window.__terminalMetrics.fits?.push({
          at: performance.now() - window.__terminalMetrics.start,
          ms: performance.now() - start, cols: this.terminal.cols, rows: this.terminal.rows,
        });
      }
    };
  });
  const begin = () => page.evaluate(() => {
    const metrics = window.__terminalMetrics = {
      start: performance.now(), ipc: [], fits: [], frames: [], active: true, released: null,
      initialStreamHeight: document.querySelector(".stream-scroll").clientHeight,
    };
    let previous = metrics.start;
    const sample = () => {
      const now = performance.now();
      metrics.frames.push({
        at: now - metrics.start, gap: now - previous,
        streamHeight: document.querySelector(".stream-scroll").clientHeight,
        followGap: document.querySelector("[data-terminal-composer]").getBoundingClientRect().bottom
          - document.querySelector(".terminal-panel__surface").getBoundingClientRect().top,
        geometryGap: Math.abs(document.querySelector(".terminal-panel__surface").getBoundingClientRect().height
          - document.querySelector(".terminal-panel").getBoundingClientRect().height),
      });
      previous = now;
      if (metrics.active) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  const finish = async (vertical = false) => {
    await page.waitForTimeout(800);
    const metrics = await page.evaluate(() => {
      window.__terminalMetrics.active = false;
      return window.__terminalMetrics;
    });
    const requests = metrics.ipc.filter((entry) => entry.command === "terminal_resize");
    const late = requests.filter((entry) => entry.at > metrics.released + 400);
    const gaps = metrics.frames.filter((entry) => entry.at < metrics.released)
      .map((entry) => entry.gap).sort((a, b) => a - b);
    const renderer = metrics.fits.at(-1);
    const last = requests.at(-1);
    const during = metrics.frames.filter((entry) => entry.at < metrics.released);
    let previousHeight = metrics.initialStreamHeight;
    let dragLayouts = 0;
    for (const entry of during) {
      if (Math.abs(entry.streamHeight - previousHeight) > 0.5) dragLayouts++;
      previousHeight = entry.streamHeight;
    }
    const summary = {
      durationMs: metrics.released, fits: metrics.fits.length,
      totalFitMs: metrics.fits.reduce((total, entry) => total + entry.ms, 0),
      p95FrameMs: gaps[Math.floor(gaps.length * 0.95)], maxFrameMs: Math.max(...gaps),
      requests: requests.length, lateRequests: late.length,
      resizeTailMs: last?.at - metrics.released, renderer, lastRequest: last,
      dragLayouts,
      fitsWhileDragging: metrics.fits.filter((entry) => entry.at < metrics.released).length,
      maxFollowGapPx: Math.max(0, ...during.map((entry) => Math.abs(entry.followGap))),
      maxGeometryGapPx: Math.max(0, ...during.map((entry) => entry.geometryGap)),
    };
    console.log(JSON.stringify(summary));
    assert.equal(late.length, 0, "release must not leave obsolete PTY sizes queued for seconds");
    assert.ok(last && renderer, "gesture must exercise real xterm resize");
    assert.equal(last.rows, renderer.rows, "backend must settle at the renderer's final rows");
    assert.equal(last.cols, renderer.cols, "backend must settle at the renderer's final columns");
    if (vertical) {
      assert.ok(summary.maxGeometryGapPx < 1, "dragged panel bounds must match the visible surface and menu hit area");
      assert.equal(summary.fitsWhileDragging, 0, "hold xterm/PTY geometry stable until pointer release");
      assert.ok(summary.maxFollowGapPx < 2, "composer must follow the drag boundary");
    }
    return summary;
  };

  const handle = page.getByRole("separator", { name: "调整终端高度" });
  const box = await handle.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + 3;
  await page.mouse.move(x, y);
  await begin();
  await page.evaluate(() => document.addEventListener("pointerdown", (event) => {
    window.__terminalMetrics.pointer = event.pointerId;
  }, { once: true, capture: true }));
  await page.mouse.down();
  // Native press/capture; fixed-rate PointerEvents thereafter. Awaiting each
  // Playwright mouse.move lets the slow UI itself throttle the driver, hiding
  // the backlog that a real high-rate pointer creates.
  await page.evaluate(async ({ x, y }) => {
    const handle = document.querySelector('[role="separator"][aria-label="调整终端高度"]');
    const pointerId = window.__terminalMetrics.pointer;
    for (const [from, to] of [[y, y - 240], [y - 240, y + 120], [y + 120, y - 120]]) {
      for (let n = 1; n <= 60; n++) {
        handle.dispatchEvent(new PointerEvent("pointermove", {
          bubbles: true, pointerId, buttons: 1, clientX: x, clientY: from + (to - from) * n / 60,
        }));
        await new Promise((resolve) => setTimeout(resolve, 8));
      }
    }
    window.__terminalMetrics.released = performance.now() - window.__terminalMetrics.start;
    handle.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId, button: 0, clientX: x, clientY: y - 120 }));
  }, { x, y });
  await page.mouse.up();
  const height = await finish(true);

  await begin();
  await page.evaluate(async () => {
    const main = document.querySelector("#root > div");
    for (const [from, to] of [[1280, 680], [680, 1080], [1080, 960]]) {
      for (let n = 1; n <= 40; n++) {
        main.style.width = `${Math.round(from + (to - from) * n / 40)}px`;
        await new Promise((resolve) => setTimeout(resolve, 8));
      }
    }
    window.__terminalMetrics.released = performance.now() - window.__terminalMetrics.start;
  });
  const width = await finish();
  assert.ok(width.fits <= 3, "continuous width input must not reflow the entire scrollback every frame");
  return { height, width };
}
