import assert from "node:assert/strict";

export async function checkTerminalContextMenu(page) {
  const navigation = page.locator("#session-navigation");
  const open = async () => {
    await navigation.getByRole("button", { name: "打开终端", exact: true }).click();
    await page.waitForSelector(".xterm-rows");
    await page.waitForTimeout(400);
  };
  const menu = page.getByRole("menu");
  const waitForMenu = () => page.waitForFunction(() => {
    const node = document.querySelector('[role="menu"]');
    return node && getComputedStyle(node).opacity === "1";
  });
  const itemStyle = () => menu.getByRole("menuitem").first().evaluate((node) => {
    const style = getComputedStyle(node);
    return { fontSize: style.fontSize, lineHeight: style.lineHeight,
      paddingTop: style.paddingTop, paddingBottom: style.paddingBottom };
  });
  const showMenu = async () => {
    await page.locator(".terminal-panel__screen").first().click({ button: "right", position: { x: 30, y: 30 } });
    await page.getByRole("menuitem", { name: "全选", exact: true }).waitFor();
    await waitForMenu();
  };
  const checkMenuStyle = async (theme, expectedColor) => {
    const items = await menu.getByRole("menuitem").evaluateAll((nodes) => nodes.map((node) => {
      const icon = node.querySelector("svg");
      const box = node.getBoundingClientRect();
      const iconBox = icon?.getBoundingClientRect();
      return {
        hasIcon: !!icon, decorative: icon?.getAttribute("aria-hidden"),
        width: iconBox?.width, height: iconBox?.height,
        centerGap: iconBox ? Math.abs(iconBox.y + iconBox.height / 2 - box.y - box.height / 2) : null,
      };
    }));
    assert.equal(items.length, 6);
    for (const item of items) {
      assert.ok(item.hasIcon && item.decorative === "true", `${theme}: each action has a decorative icon`);
      assert.equal(item.width, 14, `${theme}: icons share the compact 14px size`);
      assert.equal(item.height, 14);
      assert.ok(item.centerGap < 1, `${theme}: icons are vertically centered`);
    }
    const close = menu.getByRole("menuitem", { name: "关闭终端", exact: true });
    const color = () => close.evaluate((node) => ({
      text: getComputedStyle(node).color, icon: getComputedStyle(node.querySelector("svg")).color,
    }));
    const normal = await color();
    assert.deepEqual(normal, { text: expectedColor, icon: expectedColor }, `${theme}: close text and icon are red`);
    await close.hover();
    await close.locator('[aria-hidden="true"]').waitFor();
    const highlighted = await color();
    assert.ok(await close.evaluate((node) => node.hasAttribute("data-highlighted")), `${theme}: real hover highlights close`);
    assert.deepEqual(highlighted, normal, `${theme}: hovering must not reset the destructive color`);
    await menu.screenshot({ path: `/tmp/terminal-context-menu-${theme}.png` });
    return { icons: items.length, normal, highlighted };
  };

  await open();
  await page.locator(".stream-scroll").click({ button: "right", position: { x: 80, y: 50 } });
  await waitForMenu();
  const referenceItemStyle = await itemStyle();
  const referenceMinWidth = await menu.evaluate((node) => getComputedStyle(node).minWidth);
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "detached", timeout: 2000 });
  const before = await page.evaluate(() => ({
    pointerEvents: document.body.style.pointerEvents,
    width: document.querySelector(".main-col").getBoundingClientRect().width,
  }));
  await showMenu();
  const terminalItemStyle = await itemStyle();
  assert.deepEqual(terminalItemStyle, referenceItemStyle, "terminal menu must use the same item density as the chat context menu");
  assert.equal(await menu.evaluate((node) => getComputedStyle(node).minWidth), referenceMinWidth, "terminal and chat menus must share their minimum width");
  const lightMenuStyle = await checkMenuStyle("light", "rgb(220, 38, 38)");
  await page.screenshot({ path: "/tmp/terminal-context-current.png" });
  const during = await page.evaluate(() => ({
    pointerEvents: document.body.style.pointerEvents,
    width: document.querySelector(".main-col").getBoundingClientRect().width,
    panel: document.querySelector(".terminal-panel").getBoundingClientRect().toJSON(),
    surface: document.querySelector(".terminal-panel__surface").getBoundingClientRect().toJSON(),
    menu: document.querySelector('[role="menu"]').getBoundingClientRect().toJSON(),
  }));
  assert.equal(during.pointerEvents, before.pointerEvents, "terminal context menu must not disable pointer input on the app body");
  assert.equal(during.width, before.width, "opening a terminal context menu must not resize the app");
  assert.ok(Math.abs(during.panel.height - during.surface.height) < 1, "menu trigger surface must match the real panel");

  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "detached" });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await showMenu();
  const darkMenuStyle = await checkMenuStyle("dark", "rgb(248, 113, 113)");
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "detached" });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await showMenu();

  // Real mouse input, not locator.click's retries: one click must both dismiss
  // the menu and activate the intended panel toggle.
  const hide = await navigation.getByRole("button", { name: "隐藏终端", exact: true }).boundingBox();
  await page.mouse.click(hide.x + hide.width / 2, hide.y + hide.height / 2);
  await page.waitForFunction(() => document.querySelector(".terminal-panel").dataset.open === "false", null, { timeout: 2000 });
  await menu.waitFor({ state: "detached", timeout: 2000 });
  await page.waitForTimeout(350);

  await open();
  await showMenu();
  await page.keyboard.press("Control+Shift+J");
  await page.waitForFunction(() => document.querySelector(".terminal-panel").dataset.open === "false", null, { timeout: 2000 });
  await menu.waitFor({ state: "detached", timeout: 2000 });
  await page.waitForTimeout(350);
  return { before, during, referenceItemStyle, terminalItemStyle, lightMenuStyle, darkMenuStyle,
    closesWithOneClick: true, closesWithShortcut: true };
}
