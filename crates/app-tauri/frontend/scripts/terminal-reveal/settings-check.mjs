import assert from "node:assert/strict";

export async function checkTerminalSettings(page) {
  const sizes = [
    { width: 320, fontSize: 16 }, { width: 360, fontSize: 16 },
    { width: 640, fontSize: 16 }, { width: 768, fontSize: 16 },
    { width: 840, fontSize: 16 }, { width: 1080, fontSize: 16 },
    { width: 1080, fontSize: 20 },
  ];
  const runs = [];
  for (const size of sizes) {
    await page.setViewportSize({ width: size.width, height: 900 });
    await page.evaluate(async (fontSize) => {
      document.documentElement.style.fontSize = `${fontSize}px`;
      await document.fonts.ready;
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, size.fontSize);
    const cards = await page.locator("[data-terminal-settings-fixture] button[aria-pressed]").evaluateAll((nodes) =>
      nodes.map((node) => {
        const footer = node.lastElementChild;
        const label = footer.firstElementChild.firstElementChild;
        const swatches = label.nextElementSibling;
        const radio = footer.lastElementChild;
        const box = node.getBoundingClientRect(), text = label.getBoundingClientRect();
        const dots = swatches.getBoundingClientRect(), mark = radio.getBoundingClientRect();
        return {
          label: label.textContent, x: box.x, width: box.width, height: box.height,
          labelHeight: text.height, lineHeight: Number.parseFloat(getComputedStyle(label).lineHeight),
          swatchCenterGap: Math.abs(text.y + text.height / 2 - dots.y - dots.height / 2),
          markerCenterGap: Math.abs(text.y + text.height / 2 - mark.y - mark.height / 2),
          dotsAfterLabel: dots.left >= text.right,
          dotsBeforeMarker: dots.right <= mark.left,
          withinCard: mark.right <= box.right,
        };
      }));
    assert.equal(cards.length, 7);
    for (const card of cards) {
      const context = `${size.width}px / ${size.fontSize}px font / ${card.label}`;
      assert.ok(card.labelHeight <= card.lineHeight + 1, `${context}: name stays on one line`);
      assert.ok(card.swatchCenterGap < 1, `${context}: colour dots stay on the name's row`);
      assert.ok(card.markerCenterGap < 1, `${context}: selection marker stays on the name's row`);
      assert.ok(card.dotsAfterLabel && card.dotsBeforeMarker && card.withinCard, `${context}: footer fits without overlap`);
    }
    const widths = cards.map((card) => card.width), heights = cards.map((card) => card.height);
    assert.ok(Math.max(...widths) - Math.min(...widths) < 1, "all theme columns have equal widths");
    assert.ok(Math.max(...heights) - Math.min(...heights) < 1, "all theme cards have equal heights");
    runs.push({ ...size, columns: new Set(cards.map((card) => Math.round(card.x))).size,
      cardWidth: widths[0], cardHeight: heights[0] });
  }
  await page.setViewportSize({ width: 840, height: 900 });
  await page.evaluate(() => {
    document.documentElement.style.fontSize = "16px";
    document.querySelector("[data-terminal-settings-fixture]").scrollTop = 0;
  });
  const solarized = page.getByRole("button", { name: /Solarized 浅色/ });
  await solarized.click();
  await page.waitForFunction(() => [...document.querySelectorAll("button[aria-pressed]")].some((node) =>
    node.textContent.includes("Solarized 浅色") && node.getAttribute("aria-pressed") === "true"));
  await solarized.locator("..").screenshot({ path: "/tmp/terminal-schemes-layout.png" });
  return { runs, selectedSolarized: true };
}
