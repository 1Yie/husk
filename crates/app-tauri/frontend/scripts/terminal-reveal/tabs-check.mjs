import assert from "node:assert/strict";

export async function checkTerminalTabs(page) {
  await page.locator("#session-navigation").getByRole("button", { name: "打开终端", exact: true }).click();
  await page.waitForSelector(".xterm-rows");
  await page.waitForTimeout(1200);
  await page.evaluate(async () => {
    const url = performance.getEntriesByType("resource").find((entry) => entry.name.includes("/terminal-session.ts"))?.name;
    const { TerminalSession } = await import(url);
    const fit = TerminalSession.prototype.fitNow;
    TerminalSession.prototype.fitNow = function () {
      const start = performance.now();
      try { return fit.call(this); } finally {
        window.__terminalMetrics.fits?.push({ ms: performance.now() - start });
      }
    };
  });
  const runs = [];
  const phase = async (name, click) => {
    await page.evaluate(() => {
      const old = document.querySelector('.terminal-panel [role="tab"][aria-selected="true"]')?.id;
      const data = window.__terminalMetrics = {
        start: 0, ipc: [], fits: [], frames: [], active: true, firstPaint: null, old,
      };
      let previous = 0;
      document.addEventListener("click", () => { previous = data.start = performance.now(); }, { capture: true, once: true });
      const sample = () => {
        if (data.start) {
          const now = performance.now();
          const selected = document.querySelector('.terminal-panel [role="tab"][aria-selected="true"]');
          if (selected?.id !== old && data.firstPaint === null) data.firstPaint = now - data.start;
          data.frames.push(now - previous);
          previous = now;
        }
        if (data.active) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    await click();
    await page.waitForTimeout(1000);
    runs.push(await page.evaluate((name) => {
      const data = window.__terminalMetrics;
      data.active = false;
      const gaps = data.frames.slice().sort((a, b) => a - b);
      return { name, firstPaintMs: data.firstPaint, maxFrameMs: gaps.at(-1),
        p95FrameMs: gaps[Math.floor(gaps.length * .95)],
        totalFitMs: data.fits.reduce((total, fit) => total + fit.ms, 0),
        opens: data.ipc.filter((call) => call.command === "terminal_open"),
        resizes: data.ipc.filter((call) => call.command === "terminal_resize").length };
    }, name));
  };
  const panel = page.locator(".terminal-panel");
  for (let i = 0; i < 2; i++) await phase(`new-${i}`, () => panel.getByRole("button", { name: "新建终端", exact: true }).click());
  for (let i = 0; i < 4; i++) await phase(`switch-${i}`, () => panel.getByRole("tab").nth(i % 2 === 0 ? 0 : 2).click());
  console.log(JSON.stringify({ tabs: runs }, null, 2));
  for (const run of runs.slice(0, 2)) {
    assert.equal(run.opens.length, 1, "new tab owns exactly one new PTY");
    assert.ok(run.firstPaintMs < run.opens[0].at, "new active pane must paint before expensive renderer initialization");
  }
  for (const run of runs.slice(2)) {
    assert.equal(run.opens.length, 0, "tab switching must retain existing PTYs");
    assert.equal(run.resizes, 0, "equal-sized tab switching must not reflow scrollback or resize PTYs");
  }
  return runs;
}
