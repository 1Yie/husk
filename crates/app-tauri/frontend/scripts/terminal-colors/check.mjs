// TERMINAL_BROWSER=/usr/bin/google-chrome node scripts/terminal-colors/check.mjs
// Actual xterm parsing/rendering, simulated PTY IPC and startup color queries.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
process.chdir(root);
const output = "/tmp/husk-terminal-colors";
const theme = process.argv.includes("--light") ? "light" : "dark";
await fs.mkdir(output, { recursive: true });
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-terminal-colors",
  plugins: [react(), {
    name: "terminal-colors",
    configureServer(vite) {
      vite.middlewares.use("/terminal-colors", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml("/terminal-colors",
          '<html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div class="terminal-panel__view" style="inset:0"><div id="terminal" class="terminal-panel__screen"></div></div><script type="module" src="/scripts/terminal-colors/fixture.tsx"></script></body></html>'));
      });
    },
  }],
  resolve: { alias: {
    "@": path.join(root, "src"),
    "@tauri-apps/api/core": path.join(root, "scripts/terminal-colors/tauri.ts"),
    "@tauri-apps/api/event": path.join(root, "scripts/terminal-colors/tauri.ts"),
  } },
  server: { host: "127.0.0.1", port: 0, watch: null },
});

let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    executablePath: process.env.TERMINAL_BROWSER, headless: true,
    args: ["--no-sandbox", "--no-proxy-server", "--disable-background-networking"],
  });
  const page = await browser.newPage({ viewport: { width: 720, height: 360 } });
  const errors = [];
  let changed = false;
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("requestfailed", (request) => { if (request.failure()?.errorText.includes("ERR_NETWORK_CHANGED")) changed = true; });
  await page.goto(`${server.resolvedUrls.local[0]}terminal-colors?theme=${theme}`, { waitUntil: "networkidle" });
  if (changed) {
    errors.length = 0;
    await page.reload({ waitUntil: "networkidle" });
  }
  await page.waitForFunction(() => window.__terminalColors?.shell.opened);
  await page.waitForTimeout(100);
  const handshake = await page.evaluate(() => window.__terminalColors.shell);
  assert.equal(handshake.queriedBeforeOpen, true, "fixture must query color before terminal_open resolves");
  assert.ok(handshake.writes.join("").includes("]11;rgb:"), "initial shell background-color query must receive xterm's reply");
  assert.ok(handshake.writes.join("").includes(theme === "light"
    ? "]11;rgb:f6f6/f8f8/fafa" : "]11;rgb:1010/1818/2020"), "color query reports the actual light/dark renderer background");
  await page.waitForFunction(() => document.querySelector(".xterm-rows")?.textContent.includes("TRUECOLOR"), undefined, { timeout: 5000 }).catch(async (error) => {
    console.error(await page.evaluate(() => ({
      shell: window.__terminalColors.shell,
      state: window.__terminalColors.state,
      rows: document.querySelector(".xterm-rows")?.textContent,
      host: { width: document.getElementById("terminal").clientWidth, height: document.getElementById("terminal").clientHeight },
      errors: document.body.innerText,
    })));
    await page.screenshot({ path: path.join(output, "render-failure.png") });
    throw error;
  });
  const color = (label, property = "color") => page.evaluate((args) => {
    const node = [...document.querySelectorAll(".xterm-rows span")].find((item) => item.textContent === args.label);
    if (!node) throw new Error(`Missing terminal label: ${args.label}`);
    return getComputedStyle(node)[args.property];
  }, { label, property });
  assert.equal(await color("ANSI16"), "rgb(159, 58, 109)", "shell OSC palette overrides remain intact");
  assert.equal(await color("ANSI256"), "rgb(175, 95, 255)", "256-color output is not remapped");
  assert.equal(await color("TRUECOLOR"), "rgb(37, 149, 233)", "24-bit shell colors render verbatim");
  assert.equal(await color("BACKGROUND", "backgroundColor"), "rgb(20, 60, 100)");
  await page.evaluate(() => window.__terminalColors.setShellTheme());
  await page.waitForFunction(() => [...document.querySelectorAll(".xterm-rows span")]
    .some((node) => node.textContent === "SHELLDEFAULT" && getComputedStyle(node).color === "rgb(213, 230, 247)"));
  assert.equal(await color("SHELLDEFAULT"), "rgb(213, 230, 247)");
  await page.waitForFunction(() => {
    const node = document.querySelector(".xterm-scrollable-element");
    return node && getComputedStyle(node).backgroundColor === "rgb(36, 54, 74)";
  }, undefined, { timeout: 5000 }).catch(async (error) => {
    console.error(await page.evaluate(() => ({
      viewport: document.querySelector(".xterm-scrollable-element")?.outerHTML.slice(0, 1000),
      terminal: document.querySelector(".xterm")?.getAttribute("style"),
      screen: document.querySelector(".xterm-screen")?.getAttribute("style"),
      shell: window.__terminalColors.shell,
    })));
    await page.screenshot({ path: path.join(output, "background-failure.png") });
    throw error;
  });
  const background = await page.locator(".xterm-scrollable-element").evaluate((node) => getComputedStyle(node).backgroundColor);
  assert.equal(background, "rgb(36, 54, 74)", "shell OSC default background must paint xterm's actual scrollable surface");
  const before = await page.evaluate(() => ({
    writes: window.__terminalColors.shell.writes.length,
    resizes: window.__terminalColors.shell.resizes,
  }));
  await page.evaluate(() => {
    window.__terminalColors.hide();
    window.__terminalColors.probeHidden();
  });
  await page.waitForFunction((count) => window.__terminalColors.shell.writes.length > count, before.writes);
  assert.equal(await page.evaluate(() => window.__terminalColors.shell.resizes), before.resizes, "color probes must not refit the PTY");
  await page.evaluate(() => window.__terminalColors.show());
  await page.waitForFunction(() => [...document.querySelectorAll(".xterm-rows span")]
    .some((node) => node.textContent === "TRUECOLOR" && getComputedStyle(node).color === "rgb(37, 149, 233)"));
  assert.equal(await color("TRUECOLOR"), "rgb(37, 149, 233)");
  await page.evaluate(() => window.__terminalColors.changeFont());
  await page.waitForFunction(() => {
    const rows = document.querySelector(".xterm-rows");
    return rows && getComputedStyle(rows).fontSize === "16px";
  });
  await page.waitForFunction(() => [...document.querySelectorAll(".xterm-rows span")]
    .some((node) => node.textContent === "SHELLDEFAULT" && getComputedStyle(node).color === "rgb(213, 230, 247)"));
  assert.equal(await color("ANSI16"), "rgb(159, 58, 109)", "changing fonts must not reset the shell palette");
  assert.equal(await color("TRUECOLOR"), "rgb(37, 149, 233)");
  assert.equal(await page.locator(".xterm-scrollable-element").evaluate((node) => getComputedStyle(node).backgroundColor), "rgb(36, 54, 74)");
  assert.equal(await page.evaluate(() => window.__terminalColors.shell.opens), 1, "colors do not restart the shell");
  assert.equal(await page.evaluate(() => window.__terminalColors.shell.closes), 0);
  assert.deepEqual(errors, []);
  await page.screenshot({ path: path.join(output, `shell-colors-${theme}.png`) });
  console.log(`Shell color checks passed (${theme}): startup/hidden OSC queries, shell palette/default colors, ANSI256, exact RGB, font changes preserve palette, no restart/color-query refit. Screenshot: ${output}/shell-colors-${theme}.png`);
} finally {
  await browser?.close();
  await server.close();
}
