import assert from "node:assert/strict";
import path from "node:path";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";

export async function checkWorkspaceLinks({ browser, base, root }) {
  const parent = path.resolve(root, "../../../.husk/scratch/workspace-file-links");
  await mkdir(parent, { recursive: true });
  const output = await mkdtemp(path.join(parent, "browser-"));
  console.log(`Workspace link evidence: ${output}`);
  const page = await browser.newPage({ viewport: { width: 1280, height: 840 } });
  const errors = [];
  const checks = [];
  page.on("pageerror", error => errors.push(error.message));
  const located = async (line, column) => {
    const target = page.locator(`.workspace-files-code-scroll [data-line="${line}"][aria-current="location"]`);
    await target.waitFor({ state: "visible" });
    const geometry = await target.evaluate(element => {
      const pane = element.closest(".workspace-files-code-scroll").getBoundingClientRect();
      const rect = element.getBoundingClientRect();
      return { visible: rect.top >= pane.top && rect.bottom <= pane.bottom, background: getComputedStyle(element).backgroundColor };
    });
    assert.equal(geometry.visible, true, "target line is inside its own scroller");
    assert.notEqual(geometry.background, "rgba(0, 0, 0, 0)", "selected line has a painted highlight");
    if (column) assert.equal(await target.locator("[data-source-column]").count(), 1);
    checks.push({ line, column, geometry });
  };
  try {
    await page.goto(`${base}terminal-reveal.html?workspace-links`);
    await page.getByRole("link", { name: "file source", exact: true }).waitFor();
    assert.equal(await page.getByRole("link", { name: "unsafe", exact: true }).count(), 0);
    assert.equal(await page.getByRole("link", { name: "outside", exact: true }).count(), 0);
    assert.equal(await page.getByRole("link", { name: "web", exact: true }).getAttribute("href"), "https://example.com/");
    for (const theme of ["light", "dark"]) {
      await page.evaluate(theme => window.workspacePanelsFixture.theme(theme), theme);
      for (const office of [false, true]) {
        await page.evaluate(office => {
          window.workspaceLinksFixture.office(office);
          window.workspaceLinksFixture.close();
        }, office);
        await page.getByRole("link", { name: "file source", exact: true }).click();
        await located(80, 3);
        await page.getByRole("tab", { name: "文件", exact: true }).waitFor();
        assert.equal(await page.getByRole("tab", { name: /^改动/ }).count(), 0);
        await page.getByRole("link", { name: "relative source", exact: true }).focus();
        await page.keyboard.press("Enter");
        await located(4, 2);
        await page.getByRole("link", { name: "file source", exact: true }).click();
        await located(80, 3);
        await page.getByRole("link", { name: "file source", exact: true }).click();
        await located(80, 3);
        await page.setViewportSize({ width: 880, height: 840 });
        await page.getByRole("link", { name: "Markdown source", exact: true }).click();
        await located(3, 2);
        assert.equal(await page.getByRole("tab", { name: "源码", exact: true }).getAttribute("aria-selected"), "true");
        await page.screenshot({ path: path.join(output, `${theme}-${office ? "office" : "programming"}-narrow.png`) });
        await page.setViewportSize({ width: 1280, height: 840 });
      }
    }
    await page.getByRole("link", { name: "missing file", exact: true }).click();
    await page.getByRole("alert").filter({ hasText: "Fixture file not found" }).waitFor();
    await page.getByRole("link", { name: "truncated source", exact: true }).click();
    await page.getByRole("status", { name: "文件定位" }).filter({ hasText: "预览范围" }).waitFor();
    assert.equal(await page.locator('[aria-current="location"]').count(), 0);
    await page.getByRole("button", { name: "展开预览", exact: true }).waitFor();
    await page.getByRole("link", { name: "invalid line", exact: true }).click();
    await page.getByRole("status", { name: "文件定位" }).filter({ hasText: "超出文件范围" }).waitFor();
    await page.getByRole("link", { name: "large file", exact: true }).click();
    await page.getByText("文件过大，无法预览。", { exact: false }).waitFor();
    await page.getByRole("link", { name: "slow file", exact: true }).click();
    await page.getByRole("link", { name: "relative source", exact: true }).click();
    await located(4, 2);
    await page.waitForTimeout(500);
    await located(4, 2);
    await page.evaluate(() => window.workspaceLinksFixture.switch("/repo", 2));
    await page.waitForTimeout(100);
    assert.equal(await page.locator('[aria-current="location"]').count(), 0, "session switches discard location");
    await page.getByRole("link", { name: "relative source", exact: true }).click();
    await located(4, 2);
    await page.evaluate(() => window.workspaceLinksFixture.switch("/other", 1));
    await page.waitForTimeout(100);
    assert.equal(await page.locator('[aria-current="location"]').count(), 0, "root switches discard location");
    const calls = await page.evaluate(() => window.workspaceFileCalls);
    assert.ok(calls.every(call => ["/repo", "/other"].includes(call.root) && !call.path.startsWith("/") && !call.path.split("/").includes("..")));
    assert.equal(errors.length, 0, errors.join("\n"));
    await writeFile(path.join(output, "result.json"), JSON.stringify({ passed: true, checks, calls, errors }, null, 2));
  } catch (error) {
    await page.screenshot({ path: path.join(output, "first-failure.png") });
    await writeFile(path.join(output, "failure.json"), JSON.stringify({ error: String(error), checks, errors }, null, 2));
    throw error;
  } finally {
    await page.close();
  }
}
