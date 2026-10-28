// WORKSPACE_BROWSER=/usr/bin/google-chrome node scripts/workspace-files/header-check.mjs
// Real CSS/geometry, mocked read-only filesystem IPC; never reads local files.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
process.chdir(root);
const output = "/tmp/husk-file-headers";
await fs.mkdir(output, { recursive: true });
const server = await createServer({
  configFile: false, root, cacheDir: "node_modules/.vite-file-header",
  plugins: [react(), {
    name: "file-header-fixture",
    configureServer(vite) {
      vite.middlewares.use("/file-header", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await vite.transformIndexHtml("/file-header",
          '<html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module" src="/scripts/workspace-files/header-fixture.tsx"></script></body></html>'));
      });
    },
  }],
  resolve: { alias: {
    "@": path.join(root, "src"),
    "@tauri-apps/api/core": path.join(root, "scripts/workspace-files/tauri.ts"),
    "@tauri-apps/api/event": path.join(root, "scripts/workspace-files/tauri.ts"),
    "@tauri-apps/api/window": path.join(root, "scripts/workspace-files/tauri.ts"),
  } },
  server: { host: "127.0.0.1", port: 0, watch: null },
});

const metrics = (header) => header.evaluate((node) => {
  const css = getComputedStyle(node);
  const title = getComputedStyle(node.querySelector(".workspace-files-name"));
  const actions = [...node.querySelectorAll("button")].map((button) => {
    const box = button.getBoundingClientRect();
    const icon = button.querySelector("svg").getBoundingClientRect();
    return { width: box.width, height: box.height, iconWidth: icon.width, iconHeight: icon.height };
  });
  return {
    height: node.getBoundingClientRect().height,
    padding: css.padding, background: css.backgroundColor, gap: css.gap,
    font: title.fontSize, line: title.lineHeight, family: title.fontFamily, actions,
  };
});

let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    executablePath: process.env.WORKSPACE_BROWSER, headless: true,
    args: ["--no-sandbox", "--no-proxy-server", "--disable-background-networking"],
  });
  for (const theme of ["light", "dark"]) {
    for (const width of [280, 420]) {
      const page = await browser.newPage({ viewport: { width, height: 540 } });
      const errors = [];
      let networkChanged = false;
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("requestfailed", (request) => { if (request.failure()?.errorText.includes("ERR_NETWORK_CHANGED")) networkChanged = true; });
      await page.goto(`${server.resolvedUrls.local[0]}file-header?theme=${theme}${width === 280 ? "&font=1" : ""}`, { waitUntil: "networkidle" });
      if (networkChanged) {
        console.warn("Retrying fixture after local ERR_NETWORK_CHANGED");
        errors.length = 0;
        await page.reload({ waitUntil: "networkidle" });
      }
      const header = page.locator(".workspace-files-header");
      await page.getByRole("button", { name: "src", exact: true }).click();
      const rootMetrics = await metrics(header);
      await header.screenshot({ path: path.join(output, `${theme}-${width}-root.png`) });
      await page.getByRole("button", { name: /a-very-long-file-name/ }).click();
      await page.getByRole("button", { name: "展开预览", exact: true }).waitFor();
      await header.screenshot({ path: path.join(output, `${theme}-${width}-preview.png`) });
      assert.deepEqual(await metrics(header), rootMetrics, "tree and nested preview use identical toolbar dimensions and typography");
      assert.ok(await header.evaluate((node) => node.scrollWidth <= node.clientWidth), "long filenames do not clip toolbar actions");
      await page.getByRole("button", { name: "返回文件列表", exact: true }).click();
      assert.deepEqual(await metrics(header), rootMetrics, "returning restores the same header");
      assert.deepEqual(errors, []);
      await page.close();
    }
  }
  console.log(`File header checks passed: shared tree/preview/back metrics, light/dark, 280/420px, custom font, long filename. Screenshots: ${output}`);
} finally {
  await browser?.close();
  await server.close();
}
