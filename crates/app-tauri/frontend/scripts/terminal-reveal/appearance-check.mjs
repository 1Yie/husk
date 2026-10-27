import assert from "node:assert/strict";

// Resolve CSS colour-mix and ancestor opacity through the browser, rather than
// approximating contrast from class names or assuming a white background.
const inspectText = (locator) => locator.evaluate((node) => {
  const context = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  const rgba = (css) => {
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = css;
    context.fillRect(0, 0, 1, 1);
    return [...context.getImageData(0, 0, 1, 1).data].map((n) => n / 255);
  };
  const over = (top, bottom) => {
    const alpha = top[3] + bottom[3] * (1 - top[3]);
    return [0, 1, 2].map((i) => alpha ? (top[i] * top[3] + bottom[i] * bottom[3] * (1 - top[3])) / alpha : 0).concat(alpha);
  };
  const style = getComputedStyle(node);
  let ink = rgba(style.color), background = [0, 0, 0, 0];
  for (let item = node; item; item = item.parentElement) {
    const current = getComputedStyle(item), layer = rgba(current.backgroundColor);
    ink = over(ink, layer);
    background = over(background, layer);
    ink[3] *= Number(current.opacity);
    background[3] *= Number(current.opacity);
  }
  ink = over(ink, [1, 1, 1, 1]);
  background = over(background, [1, 1, 1, 1]);
  const luminance = (rgb) => rgb.slice(0, 3).map((c) => c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4)
    .reduce((sum, c, i) => sum + c * [.2126, .7152, .0722][i], 0);
  const foregroundLuma = luminance(ink), backgroundLuma = luminance(background);
  return {
    ratio: (Math.max(foregroundLuma, backgroundLuma) + .05) / (Math.min(foregroundLuma, backgroundLuma) + .05),
    color: style.color, fontFamily: style.fontFamily,
    fontSize: style.fontSize, fontWeight: style.fontWeight, letterSpacing: style.letterSpacing,
  };
});
const settle = (page) => page.evaluate(async () => {
  await document.fonts.ready;
  await Promise.all(document.getAnimations().filter((animation) => animation.effect?.getTiming().iterations !== Infinity)
    .map((animation) => animation.finished.catch(() => {})));
});

export async function checkAppearance(page) {
  const sidebar = page.locator("[data-appearance-sidebar]");
  await sidebar.getByRole("button", { name: /^已归档\s+\d+$/ }).click();
  const jobs = page.locator('[data-appearance-text="jobs"]');
  const subagents = page.locator('[data-appearance-text="subagents"]');
  const title = sidebar.getByText("最近完成的会话", { exact: true }).first();
  const targets = {
    jobs, activeSession: sidebar.getByText("正在查看的会话", { exact: true }).first(),
    inactiveSession: title, archivedSession: sidebar.getByText("已归档的会话", { exact: true }),
    projectHint: sidebar.getByText("资料项目", { exact: true }).first(),
    archiveToggle: sidebar.getByRole("button", { name: /^已归档\s+\d+$/ }),
    officeTab: sidebar.getByRole("tab", { name: "切换到工作模式" }),
  };
  const runs = [], typography = new Map();
  for (const font of ['"Inter", "MiSans", system-ui, sans-serif', "system-ui, sans-serif"]) {
    for (const mode of ["dark", "light"]) {
      await page.evaluate(({ mode, font }) => window.__appearanceFixture.setMode(mode, font), { mode, font });
      await page.mouse.move(1100, 20);
      await settle(page);
      const expectedFont = await page.locator("#root").evaluate((node) => getComputedStyle(node).fontFamily);
      const readings = Object.fromEntries(await Promise.all(Object.entries(targets).map(async ([key, locator]) => [key, await inspectText(locator)])));
      const failures = Object.entries(readings).filter(([, reading]) => reading.ratio < 4.5)
        .map(([key, reading]) => `${key}: ${reading.ratio.toFixed(2)} (${reading.color})`);
      assert.deepEqual(failures, [], `${mode}: small capsule/sidebar text needs at least 4.5:1 contrast`);
      for (const [key, reading] of Object.entries(readings))
        assert.equal(reading.fontFamily, expectedFont, `${mode}/${key}: retain the configured UI font`);
      const type = Object.fromEntries(Object.entries(readings).map(([key, reading]) => [key, {
        size: reading.fontSize, weight: reading.fontWeight, spacing: reading.letterSpacing,
      }]));
      if (typography.has(font)) assert.deepEqual(type, typography.get(font), "theme changes must not change font metrics");
      else typography.set(font, type);

      await page.getByRole("button", { name: "后台任务", exact: true }).hover();
      await settle(page);
      const hoveredJob = await inspectText(jobs);
      await title.hover();
      await settle(page);
      const hoveredSession = await inspectText(title);
      await page.getByRole("button", { name: "后台任务", exact: true }).focus();
      await page.keyboard.press("Tab");
      await settle(page);
      const focusedSubagent = await inspectText(subagents);
      for (const reading of [hoveredJob, hoveredSession, focusedSubagent]) assert.ok(reading.ratio >= 4.5, "hover and keyboard-expanded text stay readable");
      await page.keyboard.press("Escape");
      await page.locator("[data-appearance-fixture]").click({ position: { x: 1100, y: 20 } });
      await settle(page);
      if (font.startsWith('"Inter"')) await page.screenshot({ path: `/tmp/appearance-typography-${mode}.png` });
      runs.push({ mode, font, jobsContrast: readings.jobs.ratio,
        sidebarMinContrast: Math.min(...Object.entries(readings).filter(([key]) => key !== "jobs").map(([, reading]) => reading.ratio)),
        hoverContrast: Math.min(hoveredJob.ratio, hoveredSession.ratio), keyboardContrast: focusedSubagent.ratio });
    }
  }
  return { runs, preservedFontMetrics: true };
}
