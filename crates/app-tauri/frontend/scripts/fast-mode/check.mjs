// FAST_MODE_EXECUTABLE=/usr/bin/google-chrome node scripts/fast-mode/check.mjs
// Real browser/CSS, isolated ModelPicker fixture (no native IPC or paid API calls).
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
process.chdir(root);
const output = process.env.FAST_MODE_ARTIFACTS || "/tmp/husk-fast-mode";
await fs.mkdir(output, { recursive: true });
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-fast-mode",
  plugins: [react(), {
    name: "fast-mode-fixture",
    configureServer(vite) {
      vite.middlewares.use("/fast-mode", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml("/fast-mode",
          '<html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module" src="/scripts/fast-mode/fixture.tsx"></script></body></html>'));
      });
      vite.middlewares.use("/fast-settings", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml("/fast-settings",
          '<html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module" src="/scripts/fast-mode/settings-fixture.tsx"></script></body></html>'));
      });
    },
  }],
  resolve: { alias: {
    "@": path.join(root, "src"),
    "@tauri-apps/api/core": path.join(root, "scripts/fast-mode/tauri.ts"),
    "@tauri-apps/api/event": path.join(root, "scripts/fast-mode/tauri.ts"),
    "@tauri-apps/api/window": path.join(root, "scripts/fast-mode/tauri.ts"),
  } },
  server: { host: "127.0.0.1", port: 0, watch: null },
});

function luminance(color) {
  const rgb = color.match(/[\d.]+/g).slice(0, 3).map(Number).map((n) => {
    const v = n / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
}

async function navigate(page, url, clearErrors = () => {}) {
  let changed = false;
  const onFailure = (request) => {
    if (request.failure()?.errorText.includes("ERR_NETWORK_CHANGED")) changed = true;
  };
  page.on("requestfailed", onFailure);
  try {
    await page.goto(url, { waitUntil: "networkidle" });
    if (changed) {
      console.warn("Retrying local fixture after ERR_NETWORK_CHANGED");
      changed = false;
      clearErrors();
      await page.reload({ waitUntil: "networkidle" });
    }
    assert.equal(changed, false, "fixture transport must recover from a local network change");
  } finally {
    page.off("requestfailed", onFailure);
  }
}

let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    executablePath: process.env.FAST_MODE_EXECUTABLE,
    headless: true, args: ["--no-sandbox", "--no-proxy-server", "--disable-background-networking"],
  });
  if (process.argv.includes("--motion")) {
    const page = await browser.newPage({ viewport: { width: 720, height: 500 }, reducedMotion: "no-preference" });
    page.on("pageerror", (error) => console.error(error.message));
    page.on("requestfailed", (request) => console.error(request.url(), request.failure()));
    page.on("console", (message) => { if (message.type() === "error") console.error(message.text()); });
    await navigate(page, `${server.resolvedUrls.local[0]}fast-mode`);
    await page.getByRole("button", { name: /^当前模型:/ }).click();
    await page.getByRole("menuitem", { name: /^思考/ }).click();
    const fast = page.getByRole("switch", { name: "Fast 模式" });
    const fill = page.locator("[data-fast-fill]");
    const offset = () => fill.evaluate((node) => new DOMMatrix(getComputedStyle(node).transform).m41);
    const originalGradient = await fill.locator("..").evaluate((node) => getComputedStyle(node).backgroundImage);
    const tier = await page.getByRole("slider").getAttribute("aria-valuenow");
    await fast.click();
    const start = await offset();
    await page.waitForTimeout(180);
    const middle = await offset();
    assert.ok(start > middle && middle > 0, `purple front advances right to left: ${start} -> ${middle}`);
    assert.ok(await page.locator("[data-fast-front]").isVisible(), "front sparkles follow the transition edge");
    assert.equal(await page.locator("[data-fast-points]").count(), 0, "background points wait for the actual reveal end");
    assert.equal(await page.locator("[data-fast-ambient]").count(), 0, "meteors cannot start during the reveal");
    assert.equal(await fill.locator("..").evaluate((node) => getComputedStyle(node).backgroundImage), originalGradient, "the original blue gradient is unchanged");
    await page.getByRole("menu").screenshot({ path: path.join(output, "transition-mid.png") });
    await page.waitForFunction(() => {
      const node = document.querySelector("[data-fast-fill]");
      return node && new DOMMatrix(getComputedStyle(node).transform).m41 < 0.5
        && !document.querySelector("[data-fast-front]") && document.querySelector("[data-fast-points]");
    });
    assert.equal(await page.getByRole("slider").getAttribute("aria-valuenow"), tier);
    const point = page.locator("[data-fast-points] > span").first();
    const pointBefore = await point.boundingBox();
    await page.getByRole("slider").press("Home");
    await page.waitForTimeout(250);
    const pointAfter = await point.boundingBox();
    assert.equal(pointAfter.width, pointBefore.width, "points keep a constant pixel size");
    assert.equal(pointAfter.x, pointBefore.x, "points stay fixed to the track, not the changing fill width");
    assert.equal(await point.evaluate((node) => getComputedStyle(node).transform), "none", "background points do not scale");
    await page.locator("[data-fast-ambient]").first().waitFor({ state: "attached" });
    await page.getByRole("menu").screenshot({ path: path.join(output, "transition-complete.png") });
    await fast.click();
    await page.emulateMedia({ reducedMotion: "reduce" });
    await fast.click();
    assert.equal(await page.locator("[data-fast-front]").count(), 0, "reduced motion skips front particles");
    assert.equal(await page.locator("[data-fast-points]").count(), 0, "reduced motion skips background effects");
    assert.equal(await page.locator("[data-fast-ambient]").count(), 0, "reduced motion skips meteors");
    assert.equal(await fill.evaluate((node) => getComputedStyle(node).transitionDuration), "0s");
    assert.ok(await offset() < 0.5, "reduced motion changes the palette immediately");
    console.log(`Fast motion passed: right-to-left purple overlay, unchanged blue gradient, temporary sparkles, unchanged tier, reduced-motion fallback. Screenshots: ${output}`);
  } else if (process.argv.includes("--settings")) {
    for (const theme of ["light", "dark"]) {
      for (const width of [720, 360]) {
        const page = await browser.newPage({ viewport: { width, height: 860 }, reducedMotion: "reduce" });
        const errors = [];
        page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
        await navigate(page, `${server.resolvedUrls.local[0]}fast-settings?theme=${theme}${width === 360 ? "&font=1" : ""}`, () => { errors.length = 0; });
        await page.getByRole("button", { name: /GPT 6.1 Sol/ }).click();
        const dialog = page.getByRole("dialog");
        const fast = dialog.locator('section[aria-label="Fast 模式配置"]');
        await fast.screenshot({ path: path.join(output, `settings-${theme}-${width}.png`) });
        const geometry = await fast.evaluate((node) => ({
          height: node.getBoundingClientRect().height,
          scrollWidth: node.scrollWidth,
          width: node.clientWidth,
          font: getComputedStyle(node.querySelector('button[aria-label="Fast 模式"]')).fontSize,
        }));
        assert.ok(geometry.height < 200, `Fast should be a compact form, not a nested settings card: ${JSON.stringify(geometry)}`);
        assert.ok(geometry.scrollWidth <= geometry.width + 1, "Fast form has no horizontal overflow");
        assert.equal(geometry.font, "12px", "selector uses the form's 12px UI typography");
        assert.equal(await dialog.getByRole("textbox", { name: "Fast 参数 JSON" }).isVisible(), false, "optional protocol fields start collapsed");
        const select = dialog.getByRole("button", { name: "Fast 模式" });
        const otherSelect = dialog.getByText("推理/思考支持", { exact: true }).locator("..").getByRole("button");
        const fastBox = await select.boundingBox();
        const otherBox = await otherSelect.boundingBox();
        assert.equal(fastBox.width, otherBox.width, "Fast dropdown matches the other column's width");
        assert.equal(fastBox.x, otherBox.x, "Fast dropdown aligns with the same right column");
        assert.equal(await dialog.getByRole("switch").count(), 0, "settings use a single dropdown, not a switch");
        const disclosureStyle = (node) => {
          const css = getComputedStyle(node);
          return { font: css.fontSize, weight: css.fontWeight, spacing: css.letterSpacing, line: css.lineHeight };
        };
        const modelDisclosure = await dialog.getByRole("button", { name: "高级参数" }).evaluate(disclosureStyle);
        await page.screenshot({ path: path.join(output, `settings-${theme}-${width}-form.png`) });
        await select.click();
        await page.getByRole("menuitemradio", { name: "model", exact: true }).click();
        await dialog.getByRole("textbox", { name: "快速版模型 ID" }).fill("vendor-highspeed");
        await dialog.getByRole("button", { name: "高级参数" }).click();
        assert.ok(await dialog.getByRole("textbox", { name: "Fast 参数 JSON" }).isVisible());
        await dialog.getByRole("textbox", { name: "Fast 参数 JSON" }).fill('{"latency":"low"}');
        await page.screenshot({ path: path.join(output, `settings-${theme}-${width}-advanced.png`) });
        await page.keyboard.press("Escape");
        await dialog.waitFor({ state: "hidden" });
        await page.getByRole("button", { name: "编辑提供商", exact: true }).click();
        const providerDisclosure = await page.getByRole("button", { name: "兼容性与请求头" }).evaluate(disclosureStyle);
        assert.deepEqual(modelDisclosure, providerDisclosure, "advanced fields reuse the provider editor typography");
        assert.deepEqual(errors, []);
        await page.close();
      }
    }
    console.log(`Fast settings layout passed: compact 12px form, collapsed advanced fields, light/dark, 360/720px, model selection. Screenshots: ${output}`);
  } else {
  for (const theme of ["light", "dark"]) {
    for (const width of [720, 360]) {
      const page = await browser.newPage({ viewport: { width, height: 500 }, reducedMotion: "reduce" });
      const errors = [];
      page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
      page.on("console", (message) => { if (message.type() === "error") console.error(message.text()); });
      await navigate(page, `${server.resolvedUrls.local[0]}fast-mode?theme=${theme}${width === 360 ? "&long=1&font=1" : ""}`, () => { errors.length = 0; });
      const trigger = page.getByRole("button", { name: /^当前模型:/ });
      await trigger.click({ timeout: 5000 });
      await page.waitForTimeout(250);
      const rootHeader = await page.getByText("模型与思考", { exact: true }).boundingBox();
      assert.equal(await page.getByRole("switch", { name: "Fast 模式" }).count(), 0, "Fast is not on the outer menu");
      await page.getByRole("menuitem", { name: /^模型 / }).click();
      const modelHeader = await page.getByRole("button", { name: "返回", exact: true }).locator("..").boundingBox();
      await page.getByRole("button", { name: "返回", exact: true }).click();
      try {
        await page.getByRole("menuitem", { name: /^思考/ }).click({ timeout: 5000 });
      } catch (error) {
        await page.screenshot({ path: path.join(output, `${theme}-${width}-failure.png`) });
        console.error({ theme, width, errors, body: await page.locator("body").innerText() });
        throw error;
      }
      const menu = page.getByRole("menu");
      const thinkingHeader = page.getByRole("button", { name: "返回", exact: true }).locator("..");
      const headerBox = await thinkingHeader.boundingBox();
      assert.equal(headerBox.height, rootHeader.height, `Thinking title must match root title height; root=${rootHeader.height}, thinking=${headerBox.height}`);
      assert.equal(headerBox.height, modelHeader.height, "Model and thinking title rows have identical height");
      assert.equal(headerBox.width, rootHeader.width, "Nested title has no additional horizontal inset");
      const fast = page.getByRole("switch", { name: "Fast 模式" });
      assert.equal(await fast.getAttribute("title"), null, "no native title tooltip");
      assert.ok(await fast.locator('svg[aria-hidden="true"]').isVisible(), "Fast has a visible lightning icon");
      assert.equal(await fast.locator("button").count(), 0, "Fast is an icon toggle, not a nested switch");
      await fast.hover();
      const slider = page.getByRole("slider", { name: "思考等级" });
      const tier = await slider.getAttribute("aria-valuenow");
      const offColor = await fast.locator("svg").evaluate((node) => getComputedStyle(node).color);
      await menu.screenshot({ path: path.join(output, `${theme}-${width}-off.png`) });
      await page.getByRole("tooltip").waitFor({ state: "visible" });
      assert.equal(await page.getByRole("tooltip").innerText(), "更快速的吐字模式，可能会产生额外费用");
      await fast.click();
      assert.equal(await fast.getAttribute("aria-checked"), "true");
      assert.equal(await slider.getAttribute("aria-valuenow"), tier, "Fast must not change thinking");
      const onColor = await fast.locator("svg").evaluate((node) => getComputedStyle(node).color);
      assert.notEqual(onColor, offColor, "the lightning turns purple when Fast is enabled");
      await fast.focus();
      const box = await menu.boundingBox();
      const toggleBox = await fast.boundingBox();
      assert.ok(box.x >= 0 && box.x + box.width <= width + 1, "menu fits the viewport");
      assert.ok(toggleBox.x >= box.x && toggleBox.x + toggleBox.width <= box.x + box.width, "Fast is not clipped");
      assert.ok(toggleBox.y < box.y + 55, "Fast stays in the top-right header");
      const colors = await fast.evaluate((node) => ({
        text: getComputedStyle(node).color,
        background: getComputedStyle(node).backgroundColor,
        menu: getComputedStyle(node.closest('[role="menu"]')).backgroundColor,
      }));
      const background = colors.background === "rgba(0, 0, 0, 0)" ? colors.menu : colors.background;
      const values = [luminance(colors.text), luminance(background)].sort((a, b) => b - a);
      assert.ok((values[0] + 0.05) / (values[1] + 0.05) >= 4.5, "Fast text contrast is at least 4.5:1");
      await menu.screenshot({ path: path.join(output, `${theme}-${width}.png`) });
      await page.getByRole("button", { name: "返回", exact: true }).click();
      assert.equal(await page.getByRole("img", { name: "Fast 已开启" }).count(), 2, "both the composer tier and thinking summary show a lightning indicator");
      assert.equal(await page.getByRole("switch", { name: "Fast 模式" }).count(), 0, "the root menu only has a status indicator");
      await menu.screenshot({ path: path.join(output, `${theme}-${width}-summary.png`) });
      await page.getByRole("menuitem", { name: /^思考/ }).click();
      await fast.focus();
      await page.keyboard.press("Enter");
      assert.equal(await fast.getAttribute("aria-checked"), "false", "keyboard toggles Fast");
      assert.ok(await menu.isVisible(), "toggling keeps the menu open");
      await page.keyboard.press("Escape");
      await menu.waitFor({ state: "hidden" });
      await trigger.click();
      await page.getByRole("textbox", { name: "其他输入" }).click();
      await menu.waitFor({ state: "hidden" });
      assert.equal(await page.getByRole("textbox").evaluate((node) => node === document.activeElement), true);
      assert.deepEqual(errors, []);
      await page.close();
    }
  }
  const page = await browser.newPage();
  await navigate(page, `${server.resolvedUrls.local[0]}fast-mode?noReasoning=1`);
  await page.getByRole("button", { name: /^当前模型:/ }).click();
  assert.equal(await page.getByRole("switch", { name: "Fast 模式" }).count(), 0);
  await page.getByRole("menuitem", { name: /^思考/ }).click();
  await page.getByRole("switch", { name: "Fast 模式" }).click();
  assert.equal(await page.getByRole("switch").getAttribute("aria-checked"), "true");
  console.log(`Fast menu browser checks passed: light/dark, 360/720px, long names, user font, keyboard, outside focus, no-reasoning model. Screenshots: ${output}`);
  }
} finally {
  await browser?.close();
  await server.close();
}
