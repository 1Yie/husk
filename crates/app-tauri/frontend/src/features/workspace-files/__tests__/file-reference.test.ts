import { describe, expect, it } from "vitest";
import { parseWorkspaceFileReference as parse, parseMarkdownResourceReference as parseMarkdown, workspaceFileHref } from "../file-reference";

describe("workspace file reference boundary", () => {
  it.each([
    ["src/a.ts:12:3", "/repo", "src/a.ts", 12, 3],
    ["./src/a.ts#L12", "/repo", "src/a.ts", 12, undefined],
    ["/repo/a%20b.ts#L2C4", "/repo", "a b.ts", 2, 4],
    ["file:///repo/a%20b.ts:2", "/repo", "a b.ts", 2, undefined],
    ["C:\\repo\\src\\a.ts:12:3", "C:\\repo", "src/a.ts", 12, 3],
    ["file:///C:/repo/src/a.ts#L12C3", "C:\\repo", "src/a.ts", 12, 3],
    ["src/a.ts:1", "/", "src/a.ts", 1, undefined],
    ["C:/src/a.ts:1", "C:/", "src/a.ts", 1, undefined],
  ])("resolves %s against %s", (href, root, path, line, column) => {
    const target = { path, ...(line === undefined ? {} : { line }), ...(column === undefined ? {} : { column }) };
    expect(parse(href as string, root as string)).toEqual(target);
    expect(parse(workspaceFileHref(target as { path: string; line?: number; column?: number }, root as string), root as string)).toEqual(target);
  });

  it.each([
    "https://example.com/a.ts", "#L12", "javascript:alert(1)", "data:text/plain,hi",
    "file:///other/a.ts", "file://server/repo/a.ts", "/repository/a.ts",
    "../a.ts", "./src/../a.ts", "%2e%2e/a.ts", "src/%2e%2e/a.ts",
    "%252e%252e/a.ts", "a%00.ts", "a%2fts/../b.ts", "src/a.ts:0",
    "src/a.ts#L0", "src/a.ts#L1C0",
  ])("rejects an external or invalid target: %s", (href) => {
    expect(parse(href, "/repo")).toBeNull();
  });
});

describe("Markdown document-relative references", () => {
  it.each([
    ["guide.md#L2C3", "/repo", "docs/start.md", { path: "docs/guide.md", line: 2, column: 3 }],
    ["../README.md", "/repo", "docs/start.md", { path: "README.md" }],
    ["../assets/picture.png", "/repo", "docs/start.md", { path: "assets/picture.png" }],
    ["%2e%2e/assets/a%20b.png", "/repo", "docs/start.md", { path: "assets/a b.png" }],
    ["a b.md", "/repo", "docs/start.md", { path: "docs/a b.md" }],
    ["/repo/docs/a%20b.md:2", "/repo", "docs/start.md", { path: "docs/a b.md", line: 2 }],
    ["file:///repo/assets/picture.png", "/repo", "docs/start.md", { path: "assets/picture.png" }],
    ["../guide.md", "C:\\repo", "docs/start.md", { path: "guide.md" }],
    ["file:///C:/repo/docs/guide.md", "C:\\repo", "docs/start.md", { path: "docs/guide.md" }],
    ["#heading", "/repo", "docs/start.md", { path: "docs/start.md", fragment: "heading" }],
    ["guide.md#hello%20world", "/repo", "docs/start.md", { path: "docs/guide.md", fragment: "hello world" }],
    ["#L2C3", "/repo", "docs/start.md", { path: "docs/start.md", line: 2, column: 3 }],
  ])("resolves %s with a trusted document base", (href, root, documentPath, target) => {
    expect(parseMarkdown(href as string, root as string, documentPath as string)).toEqual(target);
  });

  it.each([
    "../../escape.png", "../../repo/secret.md", "/repo/docs/../../escape.png",
    "/outside/../repo/secret.md", "file:///outside/secret.md", "file://server/repo/secret.md",
    "//repo/secret.md", "javascript:alert(1)", "data:image/svg+xml,anything", "blob:untrusted",
    "%252e%252e/secret.md", "bad%00.png", "guide.md?download=1", "picture.png?raw=1",
    "../guide.md#bad%00anchor", "guide.md:0", ":1", "#L0",
  ])("rejects unsafe or unsupported local resources: %s", href => {
    expect(parseMarkdown(href, "/repo", "docs/start.md")).toBeNull();
  });

  it("keeps the chat parser strict when preview accepts a parent reference", () => {
    expect(parse("../README.md", "/repo")).toBeNull();
    expect(parse("guide.md#heading", "/repo")).toBeNull();
    expect(parseMarkdown("../README.md", "/repo", "docs/start.md")).toEqual({ path: "README.md" });
  });
});
