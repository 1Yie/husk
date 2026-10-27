import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DirectoryListing, FilePreview, WorkspaceEntry } from "../types";
import { codeHighlightPlugin } from "@/lib/syntax-highlight";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
// Keep this command contract local; do not extend global native mocks.
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { WorkspaceFiles } from "../index";

const entry = (name: string, kind: WorkspaceEntry["kind"] = "file", path = name): WorkspaceEntry =>
  ({ name, kind, path, size: 20 });
const listing = (entries: WorkspaceEntry[], path = ""): DirectoryListing =>
  ({ path, entries, truncated: false });
const text = (name: string, content = "hello world"): FilePreview =>
  ({ path: name, name, size: content.length, kind: "text", text: content });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => { invoke.mockReset(); });

describe("workspace files", () => {
  it("shows directory skeleton rows while the listing is pending", async () => {
    const pending = deferred<DirectoryListing>();
    invoke.mockReturnValueOnce(pending.promise);
    render(<WorkspaceFiles root="/fixture" />);
    const skeleton = screen.getByRole("status", { name: "正在读取目录" });
    expect(skeleton).toHaveAttribute("aria-busy", "true");
    expect(skeleton.querySelectorAll(".workspace-files-loading-row")).toHaveLength(6);
    await act(async () => pending.resolve(listing([entry("ready.js")])));
    expect(await screen.findByRole("button", { name: "ready.js" })).toBeVisible();
    expect(screen.queryByRole("status", { name: "正在读取目录" })).toBeNull();
  });

  it("replaces the list with a code-shaped skeleton while reading the file", async () => {
    const pending = deferred<FilePreview>();
    invoke.mockResolvedValueOnce(listing([entry("pending.ts")])).mockReturnValueOnce(pending.promise);
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "pending.ts" }));
    const skeleton = screen.getByRole("status", { name: "正在读取文件" });
    expect(skeleton.querySelectorAll(".workspace-files-code-loading-row")).toHaveLength(10);
    expect(screen.queryByLabelText("工作区目录")).toBeNull();
    expect(screen.getByRole("button", { name: "返回文件列表" })).toBeVisible();
    await act(async () => pending.resolve(text("pending.ts", "const loaded = true;")));
    expect(await screen.findByLabelText("文件源码")).toHaveTextContent("loaded");
    expect(screen.queryByRole("status", { name: "正在读取文件" })).toBeNull();
  });

  it("falls back to readable plain text instead of an endless highlighter skeleton", async () => {
    vi.spyOn(codeHighlightPlugin, "highlight").mockImplementation(() => null);
    invoke.mockResolvedValueOnce(listing([entry("plain.ts")]))
      .mockResolvedValueOnce(text("plain.ts", "plain fallback content"));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "plain.ts" }));
    expect(await screen.findByRole("status", { name: "正在加载代码" })).toBeVisible();
    expect(await screen.findByLabelText("文件源码")).toHaveTextContent("plain fallback content");
    expect(screen.queryByRole("status", { name: "正在加载代码" })).toBeNull();
  });

  it("uses the matching JS icon in both the file list and preview header", async () => {
    invoke.mockImplementation(command => Promise.resolve(command === "workspace_list_files"
      ? listing([entry("main.js")]) : text("main.js", "const value = 1;")));
    render(<WorkspaceFiles root="/fixture" />);
    const file = await screen.findByRole("button", { name: "main.js" });
    expect(file.querySelector('[data-file-icon="javascript"]')).not.toBeNull();
    fireEvent.click(file);
    await screen.findByLabelText("文件源码");
    expect(screen.getByLabelText("文件预览").querySelector('[data-file-icon="javascript"]')).not.toBeNull();
    expect(screen.queryByLabelText("工作区目录")).toBeNull();
  });

  it("replaces the list with preview and restores expansion, scroll and focus on return", async () => {
    invoke.mockImplementation((command, args) => Promise.resolve(command === "workspace_list_files"
      ? args.path === "" ? listing([entry("src", "directory")])
        : listing([entry("main.ts", "file", "src/main.ts")], "src")
      : text("main.ts", "export const value = 1;")));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "src" }));
    const file = await screen.findByRole("button", { name: "main.ts" });
    screen.getByLabelText("工作区目录").scrollTop = 72;
    fireEvent.click(file);
    await screen.findByLabelText("文件源码");
    expect(screen.queryByLabelText("工作区目录")).toBeNull();
    expect(screen.getByLabelText("文件预览")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    const restored = screen.getByRole("button", { name: "main.ts" });
    expect(screen.getByLabelText("工作区目录").scrollTop).toBe(72);
    expect(restored).toHaveFocus();
    expect(restored).toHaveAttribute("aria-current", "true");
    expect(screen.queryByLabelText("文件预览")).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => command === "workspace_list_files")).toHaveLength(2);
  });

  it("uses semantic syntax classes instead of unreadable dark-only inline colors", async () => {
    invoke.mockImplementation(command => Promise.resolve(command === "workspace_list_files"
      ? listing([entry("main.ts")]) : text("main.ts", "export const value = 'hello';")));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "main.ts" }));
    const source = await screen.findByLabelText("文件源码");
    expect(source.querySelector(".token.keyword")).not.toBeNull();
    expect(source.querySelector('span[style*="color"]')).toBeNull();
    expect(source.querySelector('span[style*="--shiki-dark"]')).toBeNull();
    const status = screen.getByLabelText("文件状态");
    expect(status).toHaveTextContent("typescript");
    expect(status).toHaveTextContent("只读");
    expect(source.compareDocumentPosition(status) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
  });

  it("renders Markdown safely and can switch back to its source", async () => {
    const markdown = "# Workspace notes\n\n**Important**\n\n<script>alert('never')</script>\n\n![tracking](https://example.invalid/tracker.png)";
    invoke.mockImplementation(command => Promise.resolve(command === "workspace_list_files"
      ? listing([entry("README.md")]) : text("README.md", markdown)));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "README.md" }));
    expect(await screen.findByRole("heading", { name: "Workspace notes" })).toBeVisible();
    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector('img[src^="https://example.invalid"]')).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "源码" }));
    expect((await screen.findByLabelText("文件源码")).textContent).toContain("# Workspace notes");
  });

  it("offers opening a workspace without invoking filesystem IPC", () => {
    const open = vi.fn();
    render(<WorkspaceFiles root="" onOpenWorkspace={open} />);
    fireEvent.click(screen.getByRole("button", { name: "打开工作区" }));
    expect(open).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("loads directories lazily, caches collapsed entries and refreshes", async () => {
    invoke.mockImplementation((command, args) => {
      expect(command).toBe("workspace_list_files");
      return Promise.resolve(args.path === ""
        ? listing([entry("src", "directory")])
        : listing([entry("main.rs", "file", "src/main.rs")], "src"));
    });
    render(<WorkspaceFiles root="/fixture" />);
    const src = await screen.findByRole("button", { name: "src" });
    expect(invoke).toHaveBeenCalledTimes(1);
    fireEvent.click(src);
    await screen.findByRole("button", { name: "main.rs" });
    expect(invoke).toHaveBeenLastCalledWith("workspace_list_files", { root: "/fixture", path: "src" });
    fireEvent.click(src);
    expect(screen.queryByRole("button", { name: "main.rs" })).toBeNull();
    fireEvent.click(src);
    expect(screen.getByRole("button", { name: "main.rs" })).toBeVisible();
    expect(invoke).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole("button", { name: "刷新文件" }));
    await screen.findByRole("button", { name: "src" });
    expect(invoke).toHaveBeenCalledTimes(3);
    expect(screen.queryByRole("button", { name: "main.rs" })).toBeNull();
  });

  it("previews real text as inert source with line numbers and an expanded dialog", async () => {
    const source = '<script>alert("never execute")</script>\n你好';
    invoke.mockImplementation(command => Promise.resolve(command === "workspace_list_files"
      ? listing([entry("page.html")]) : text("page.html", source)));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "page.html" }));
    const code = await screen.findByLabelText("文件源码");
    expect(code.textContent).toContain('alert("never execute")');
    expect(code.querySelector("script")).toBeNull();
    expect(code.textContent).toContain("你好");
    expect(code.querySelectorAll(".workspace-files-line-number")).toHaveLength(2);
    expect(invoke).toHaveBeenLastCalledWith("workspace_preview_file", { root: "/fixture", path: "page.html" });
    const expand = screen.getByRole("button", { name: "展开预览" });
    fireEvent.click(expand);
    expect(within(await screen.findByRole("dialog")).getByLabelText("文件源码")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("handles directory errors, retry, empty listings and truncation", async () => {
    invoke.mockRejectedValueOnce(new Error("permission denied"))
      .mockResolvedValueOnce(listing([]));
    const { unmount } = render(<WorkspaceFiles root="/fixture" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("permission denied");
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByText("此目录为空")).toBeVisible();
    unmount();
    invoke.mockResolvedValue({ ...listing([entry("last")]), truncated: true });
    render(<WorkspaceFiles root="/fixture" />);
    expect(await screen.findByText(/仅显示前 2,000 项/)).toBeVisible();
  });

  it("shows binary, size-limit and file errors without decoding", async () => {
    invoke.mockResolvedValueOnce(listing([entry("large"), entry("binary"), entry("broken")]))
      .mockResolvedValueOnce({ path: "large", name: "large", size: 10_000_000, kind: "too_large" })
      .mockResolvedValueOnce({ path: "binary", name: "binary", size: 22, kind: "binary" })
      .mockRejectedValueOnce(new Error("file path escapes the workspace"));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "large" }));
    expect(await screen.findByText(/文件过大，无法预览/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    fireEvent.click(screen.getByRole("button", { name: "binary" }));
    expect(await screen.findByText(/二进制或不支持的文件/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    fireEvent.click(screen.getByRole("button", { name: "broken" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("file path escapes");
    expect(screen.queryByLabelText("文件源码")).toBeNull();
  });

  it("renders only allowlisted raster images, not SVG documents", async () => {
    invoke.mockResolvedValueOnce(listing([entry("image.png"), entry("unsafe.svg")]))
      .mockResolvedValueOnce({ path: "image.png", name: "image.png", size: 8, kind: "image", mime: "image/png", data: "aGVsbG8=" })
      .mockResolvedValueOnce({ path: "unsafe.svg", name: "unsafe.svg", size: 8, kind: "image", mime: "image/svg+xml", data: "aGVsbG8=" });
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "image.png" }));
    expect(await screen.findByRole("img", { name: "image.png" })).toHaveAttribute("src", "data:image/png;base64,aGVsbG8=");
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    fireEvent.click(screen.getByRole("button", { name: "unsafe.svg" }));
    await screen.findByText(/二进制或不支持的文件/);
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("ignores stale directory and file requests across a rapid workspace switch", async () => {
    const oldListing = deferred<DirectoryListing>();
    const oldFile = deferred<FilePreview>();
    invoke.mockImplementation((command, args) => {
      if (args.root === "/old") return command === "workspace_list_files"
        ? Promise.resolve(listing([entry("old.txt"), entry("nested", "directory")]))
        : oldFile.promise;
      if (args.path === "nested") return oldListing.promise;
      return Promise.resolve(listing([entry("new.txt")]));
    });
    const view = render(<WorkspaceFiles root="/old" />);
    fireEvent.click(await screen.findByRole("button", { name: "old.txt" }));
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    // Leave a root listing outstanding too.
    invoke.mockImplementationOnce(() => oldListing.promise);
    fireEvent.click(screen.getByRole("button", { name: "nested" }));
    view.rerender(<WorkspaceFiles root="/new" />);
    await screen.findByRole("button", { name: "new.txt" });
    await act(async () => {
      oldFile.resolve(text("old.txt", "STALE CONTENT"));
      oldListing.resolve(listing([entry("STALE ENTRY")]));
    });
    expect(screen.queryByText(/STALE/)).toBeNull();
    expect(screen.queryByRole("button", { name: "old.txt" })).toBeNull();
    expect(screen.getByLabelText("工作区目录")).toBeVisible();
    expect(screen.queryByLabelText("文件预览")).toBeNull();
  });

  it("ignores an older preview when files are clicked quickly", async () => {
    const first = deferred<FilePreview>();
    invoke.mockResolvedValueOnce(listing([entry("first.txt"), entry("second.txt")]))
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(text("second.txt", "CURRENT CONTENT"));
    render(<WorkspaceFiles root="/fixture" />);
    fireEvent.click(await screen.findByRole("button", { name: "first.txt" }));
    fireEvent.click(screen.getByRole("button", { name: "返回文件列表" }));
    fireEvent.click(screen.getByRole("button", { name: "second.txt" }));
    expect(await screen.findByText("CURRENT CONTENT")).toBeVisible();
    await act(async () => first.resolve(text("first.txt", "STALE CONTENT")));
    expect(screen.queryByText("STALE CONTENT")).toBeNull();
  });
});
