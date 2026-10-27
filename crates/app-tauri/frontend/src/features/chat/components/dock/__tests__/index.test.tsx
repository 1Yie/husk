import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { renderWithProviders } from "@/test/render";
import { SessionDock } from "../index";
import { readFileSync } from "node:fs";
import { URL as NodeURL } from "node:url";
import { useLayoutStore } from "@/app/layout/layout-store";
import { mockCommand } from "@/test/tauri";
import {
  DOCK_DEFAULT_WIDTH,
  DOCK_MAX_WIDTH,
  DOCK_MIN_WIDTH,
  useDockStore,
} from "../state";
import type {
  Artifact,
  BgJob,
  FileChange,
} from "@/features/chat/hooks/stream-view";

const change = (path: string): FileChange => ({
  path,
  kind: "update",
  patches: [{ n: 1, tool: "fuzzy_patch", diff: "@@ -1 +1 @@\n-old\n+new" }],
  adds: 1,
  dels: 1,
  fuzzy: false,
});

const job = (): BgJob => ({
  id: "bg-1",
  command: "cargo test",
  state: { kind: "running" },
  started_at_ms: 1,
  elapsed_ms: 1000,
  notify: true,
  has_own_sink: false,
});

const artifact = (path: string): Artifact => ({
  path,
  name: path.split("/").pop()!,
  ext: path.split(".").pop()!,
  tool: "office_create",
  updatedAt: 0,
});

beforeEach(() => {
  localStorage.clear();
  useDockStore.setState({
    open: true,
    tab: "changes",
    browserOpen: false,
    width: DOCK_DEFAULT_WIDTH,
  });
  useLayoutStore.setState({ terminalOpen: false });
});

describe("unified session dock", () => {
  test("tab captions retain their complete names and counts", () => {
    render(<SessionDock root="/workspace" changes={[change("a.ts")]} artifacts={[artifact("report.pdf")]} jobs={[job()]} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.length).toBeGreaterThan(2);
    for (const tab of tabs) {
      expect(tab).toHaveClass("flex-none", "whitespace-nowrap");
      expect(tab.querySelector("span")).not.toHaveClass("truncate");
      expect(tab.querySelector("svg")).toHaveClass("flex-none");
    }
    expect(screen.getByRole("tab", { name: "改动（1 个文件）" })).toBeVisible();
    expect(screen.getByRole("button", { name: "工作区工具" })).toHaveClass("flex-none");
  });

  test("the width handle uses themed hover and drag feedback", () => {
    const style = document.createElement("style");
    style.textContent = readFileSync(new NodeURL("../dock.css", import.meta.url), "utf8");
    document.head.append(style);
    try {
      const rules = Array.from(style.sheet?.cssRules ?? []);
      const rule = (selector: string) => rules.find(
        (entry) => "selectorText" in entry &&
          (entry as CSSStyleRule).selectorText.split(",").some((s) => s.trim() === selector),
      ) as CSSStyleRule | undefined;
      expect(rule(".workspace-dock__separator::after")?.style.background).toBe("transparent");
      for (const selector of [
        ".workspace-dock__separator:hover::after",
        ".workspace-dock__separator:focus-visible::after",
        ".workspace-dock__separator[data-resizing]::after",
      ]) {
        expect(rule(selector)?.style.background).toBe("var(--husk-accent)");
      }
    } finally {
      style.remove();
    }
  });

  test.each(["pointerup", "pointercancel", "lostpointercapture", "hide"])(
    "width feedback and pending frames clear on %s",
    async (ending) => {
      const { rerender } = render(<SessionDock changes={[]} artifacts={[]} open />);
      const handle = screen.getByRole("separator", { name: "调整侧栏宽度" });
      expect(handle).toHaveClass("workspace-dock__separator");
      expect(handle).not.toHaveAttribute("data-resizing");
      fireEvent.pointerDown(handle, { clientX: 500 });
      expect(handle).toHaveAttribute("data-resizing");
      fireEvent.pointerLeave(handle);
      expect(handle).toHaveAttribute("data-resizing");
      fireEvent.pointerMove(window, { clientX: 460 });
      expect(localStorage.getItem("husk.dock.v1")).toBeNull();
      const cancel = vi.spyOn(window, "cancelAnimationFrame");
      if (ending === "hide") {
        rerender(<SessionDock changes={[]} artifacts={[]} open={false} />);
      } else {
        fireEvent(ending === "lostpointercapture" ? handle : window, new PointerEvent(ending));
      }
      expect(handle).not.toHaveAttribute("data-resizing");
      expect(cancel).toHaveBeenCalled();
      cancel.mockRestore();
      const endedWidth = useDockStore.getState().width;
      fireEvent.pointerMove(window, { clientX: 400 });
      fireEvent.pointerUp(window);
      await act(() => new Promise((r) => requestAnimationFrame(() => r(undefined))));
      expect(useDockStore.getState().width).toBe(endedWidth);
      expect(handle).toHaveAttribute("aria-valuenow", String(endedWidth));
      rerender(<SessionDock changes={[]} artifacts={[]} open />);
      expect(handle).not.toHaveAttribute("data-resizing");
    },
  );

  test("the workspace tools home uses a house icon without changing tool cards", () => {
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[job()]} />);
    const home = screen.getByRole("button", { name: "工作区工具" });
    expect(home.querySelector("svg")).toHaveClass("lucide-house", "h-3.5", "w-3.5");
    expect(screen.getByRole("tab", { name: /后台/ }).querySelector("svg")).toHaveClass(
      "lucide-square-terminal",
    );
    fireEvent.click(home);
    expect(
      screen.getByRole("button", { name: /打开终端/ }).querySelector("svg"),
    ).toHaveClass("lucide-square-terminal");
  });

  test("tools and the file browser preserve the remembered deliverable view", () => {
    act(() => useDockStore.getState().setTab("artifacts"));
    act(() => useDockStore.getState().setTab("tools"));
    expect(useDockStore.getState().filesTab).toBe("artifacts");
    act(() => useDockStore.getState().setTab("files"));
    expect(useDockStore.getState().filesTab).toBe("artifacts");
  });

  test("holds changes and artifacts as two views of one sidebar", () => {
    render(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[artifact("out/report.docx")]}
      />,
    );
    // Both views are always present, whatever the workspace mode.
    expect(screen.getByRole("tab", { name: /改动/ })).toBeTruthy();
    expect(screen.getByRole("tab", { name: /产物/ })).toBeTruthy();
    expect(screen.getAllByText("a.ts").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("tab", { name: /产物/ }));
    expect(screen.getByText("report.docx")).toBeInTheDocument();
    expect(screen.queryByText("a.ts")).not.toBeInTheDocument();
  });

  test("a view with nothing in it is not offered", () => {
    render(<SessionDock changes={[]} artifacts={[artifact("out/a.pdf")]} />);
    expect(screen.queryByRole("tab", { name: /改动/ })).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /产物/ })).toBeInTheDocument();
    expect(screen.getByText("a.pdf")).toBeInTheDocument();
  });

  test("a stored view whose list emptied out falls back to the other one", () => {
    useDockStore.setState({ tab: "artifacts" });
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} />);
    expect(screen.getAllByText("a.ts").length).toBeGreaterThan(0);
    expect(
      screen.getByRole("tab", { name: /改动/ }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  test("an empty session still offers usable workspace tools", () => {
    render(<SessionDock changes={[]} artifacts={[]} />);
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    expect(screen.getByText("工作区工具")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /查看工作区改动/ }),
    ).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /打开终端/ }));
    expect(useLayoutStore.getState().terminalOpen).toBe(true);
  });

  test("closing reports back so the page can silence this session's auto-open", () => {
    let closed = 0;
    render(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[]}
        onClose={() => (closed += 1)}
      />,
    );
    act(() => useDockStore.getState().setOpen(false));
    expect(closed).toBe(1);
    expect(useDockStore.getState().open).toBe(false);
    expect(screen.queryByRole("button", { name: "关闭侧栏" })).toBeNull();
  });

  test("the file browser is created by its home action and can return to the empty home", async () => {
    mockCommand("workspace_list_files", () => ({
      path: "",
      entries: [],
      truncated: false,
    }));
    render(<SessionDock root="/fixture" changes={[]} artifacts={[]} />);
    expect(screen.queryByRole("tab")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "预览工作区文件" }));
    expect(await screen.findByRole("tab", { name: "文件" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.queryByRole("tab", { name: "工具" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "返回侧栏" }));
    expect(screen.queryByRole("tab")).toBeNull();
    expect(screen.getByText("工作区工具")).toBeVisible();
  });

  test("the tools home stays reachable once lists have content", async () => {
    mockCommand("workspace_list_files", () => ({
      path: "",
      entries: [],
      truncated: false,
    }));
    render(
      <SessionDock
        root="/fixture"
        changes={[change("a.ts")]}
        artifacts={[]}
        jobs={[job()]}
      />,
    );
    expect(screen.getByRole("tab", { name: /^改动/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.click(screen.getByRole("button", { name: "工作区工具" }));
    expect(screen.getByRole("heading", { name: "工作区工具" })).toBeVisible();
    expect(screen.getByRole("tab", { name: /^改动/ })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: "预览工作区文件" }));
    expect(await screen.findByRole("tab", { name: "文件" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("working mode filters file and changes tabs even when the browser was open", () => {
    useDockStore.setState({ tab: "files", browserOpen: true });
    render(
      <SessionDock
        officeMode
        changes={[change("a.ts")]}
        artifacts={[artifact("report.docx")]}
      />,
    );
    expect(screen.queryByRole("tab", { name: /^文件/ })).toBeNull();
    expect(screen.queryByRole("tab", { name: /^改动/ })).toBeNull();
    expect(screen.getByRole("tab", { name: /^产物/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("working mode's empty home does not offer file or git actions", () => {
    render(<SessionDock officeMode changes={[]} artifacts={[]} />);
    expect(screen.queryByRole("tab")).toBeNull();
    expect(screen.queryByRole("button", { name: "预览工作区文件" })).toBeNull();
    expect(screen.queryByRole("button", { name: "查看工作区改动" })).toBeNull();
    expect(screen.getByRole("button", { name: "打开终端" })).toBeEnabled();
  });

  test("selected tabs have no bottom underline or sidebar close button", () => {
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} />);
    const selected = screen.getByRole("tab", { name: /^改动/ });
    expect(selected).not.toHaveClass("border-b-2");
    expect(selected).not.toHaveClass("border-[var(--husk-accent)]");
    expect(screen.queryByRole("button", { name: "关闭侧栏" })).toBeNull();
  });

  test("the page's `open` prop wins over the persisted flag", () => {
    // A new session inherits the previous one's persisted `open`; the page
    // ANDs in "has content", so an empty dock must not occupy the column.
    const { container } = render(
      <SessionDock changes={[]} artifacts={[]} open={false} />,
    );
    const frame = container.firstElementChild as HTMLElement;
    expect(frame.getAttribute("aria-hidden")).toBe("true");
    expect(frame.style.width).toBe("0px");
    // The store still says open — this is the page's decision, not the dock's.
    expect(useDockStore.getState().open).toBe(true);
  });

  test("width drags within bounds and persists", () => {
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} />);
    const handle = screen.getByRole("separator", { name: "调整侧栏宽度" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useDockStore.getState().width).toBe(DOCK_DEFAULT_WIDTH - 24);
    for (let i = 0; i < 40; i++)
      fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useDockStore.getState().width).toBe(DOCK_MIN_WIDTH);
    for (let i = 0; i < 40; i++)
      fireEvent.keyDown(handle, { key: "ArrowLeft" });
    expect(useDockStore.getState().width).toBe(DOCK_MAX_WIDTH);
    const saved = JSON.parse(localStorage.getItem("husk.dock.v1")!);
    expect(saved.width).toBe(DOCK_MAX_WIDTH);
    expect(saved.open).toBe(true);
  });

  test("a pointer drag commits its width once, on release", async () => {
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} />);
    const handle = screen.getByRole("separator", { name: "调整侧栏宽度" });
    fireEvent.pointerDown(handle, { clientX: 500 });
    fireEvent.pointerMove(window, { clientX: 460 });
    // The panel follows the pointer on the next frame…
    await act(
      () => new Promise((r) => requestAnimationFrame(() => r(undefined))),
    );
    expect(handle.getAttribute("aria-valuenow")).toBe(
      String(DOCK_DEFAULT_WIDTH + 40),
    );
    // …but nothing is written while the drag is live.
    expect(useDockStore.getState().width).toBe(DOCK_DEFAULT_WIDTH);
    expect(localStorage.getItem("husk.dock.v1")).toBeNull();
    fireEvent.pointerMove(window, { clientX: 450 });
    fireEvent.pointerUp(window);
    expect(useDockStore.getState().width).toBe(DOCK_DEFAULT_WIDTH + 50);
    expect(JSON.parse(localStorage.getItem("husk.dock.v1")!).width).toBe(
      DOCK_DEFAULT_WIDTH + 50,
    );
  });

  test("the column eases open and shut, except under a drag", () => {
    const { container, rerender } = render(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} open={false} />,
    );
    const frame = container.firstElementChild as HTMLElement;
    // Eased, not snapped: a one-frame re-flow of the stream throws the
    // reader's place in WebKit, which has no scroll anchoring.
    expect(frame.className).toMatch(/transition-\[width\]/);
    expect(frame.style.width).toBe("0px");
    rerender(<SessionDock changes={[change("a.ts")]} artifacts={[]} open />);
    expect(frame.style.width).toBe(`${DOCK_DEFAULT_WIDTH}px`);
    fireEvent.pointerDown(
      screen.getByRole("separator", { name: "调整侧栏宽度" }),
      { clientX: 500 },
    );
    expect(frame.className).not.toMatch(/transition/);
    fireEvent.pointerUp(window);
  });

  test("a snapped switch applies open and shut without the slide", () => {
    const { container, rerender } = render(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[]}
        open={false}
        snap
      />,
    );
    const frame = container.firstElementChild as HTMLElement;
    const aside = frame.firstElementChild as HTMLElement;
    // A session switch is a jump: neither the column nor the slide animates.
    expect(frame.className).not.toMatch(/transition/);
    expect(aside.className).not.toMatch(/transition/);
    rerender(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} open snap />,
    );
    expect(frame.style.width).toBe(`${DOCK_DEFAULT_WIDTH}px`);
    expect(frame.className).not.toMatch(/transition/);
    expect(aside.className).not.toMatch(/transition/);
    // Without the snap the same change keeps its animation.
    rerender(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} open={false} />,
    );
    expect(frame.className).toMatch(/transition-\[width\]/);
    expect(aside.className).toMatch(/transition-transform/);
  });

  test("background jobs get their own 后台 view", () => {
    const job: BgJob = {
      id: "bg-1",
      command: "cargo test",
      state: { kind: "done", label: "exited(0)", code: 0, reason: "exited" },
      started_at_ms: 1,
      elapsed_ms: 2000,
      notify: true,
      has_own_sink: false,
    };
    // The job rows carry tooltips, which need the app's providers.
    renderWithProviders(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[job]} />,
    );
    const tab = screen.getByRole("tab", { name: "后台（1 个任务）" });
    fireEvent.click(tab);
    expect(useDockStore.getState().tab).toBe("jobs");
    expect(screen.getByText("cargo test")).toBeInTheDocument();
    expect(screen.getByText("清除已结束")).toBeInTheDocument();
    expect(screen.queryByText("a.ts")).not.toBeInTheDocument();
  });

  test("a restored job is a record: readable, prunable, not controllable", () => {
    const job: BgJob = {
      id: "bg-1",
      command: "cargo test",
      state: { kind: "done", label: "应用已退出", code: null, reason: "error" },
      started_at_ms: 1,
      elapsed_ms: 2000,
      notify: true,
      has_own_sink: false,
      restored: true,
      log_tail: "test result: FAILED. 3 passed; 1 failed\n",
    };
    renderWithProviders(
      <SessionDock changes={[]} artifacts={[]} jobs={[job]} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "后台（1 个任务）" }));
    // The record can be dropped, so the bulk action counts it…
    expect(screen.getByText("清除已结束")).toBeInTheDocument();
    // …and the row offers its own removal, but nothing that needs a process.
    expect(
      screen.queryByRole("button", { name: "终止" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /bg-1/ }));
    // The log file went with the process; the saved tail is what is left.
    expect(
      screen.getByText("test result: FAILED. 3 passed; 1 failed"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "移除记录" }),
    ).toBeInTheDocument();
  });

  test("a restored job saved before tails existed says so", () => {
    const job: BgJob = {
      id: "bg-1@1",
      command: "cargo test",
      state: { kind: "done", label: "exited(0)", code: 0, reason: "exited" },
      started_at_ms: 1,
      elapsed_ms: 2000,
      notify: true,
      has_own_sink: true,
      restored: true,
    };
    renderWithProviders(
      <SessionDock changes={[]} artifacts={[]} jobs={[job]} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "后台（1 个任务）" }));
    fireEvent.click(screen.getByRole("button", { name: /bg-1@1/ }));
    expect(screen.getByText("（未保存输出）")).toBeInTheDocument();
  });

  test("改动 and 后台 switch back and forth in one sidebar", () => {
    const job: BgJob = {
      id: "bg-1",
      command: "cargo test",
      state: { kind: "running" },
      started_at_ms: 1,
      elapsed_ms: 1000,
      notify: true,
      has_own_sink: false,
    };
    renderWithProviders(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[job]} />,
    );
    const changesTab = screen.getByRole("tab", { name: /改动/ });
    const jobsTab = screen.getByRole("tab", { name: "后台（1 个任务）" });
    // The file list is what the dock opens on.
    expect(changesTab.getAttribute("aria-selected")).toBe("true");

    fireEvent.click(jobsTab);
    expect(useDockStore.getState().tab).toBe("jobs");
    expect(jobsTab.getAttribute("aria-selected")).toBe("true");
    expect(changesTab.getAttribute("aria-selected")).toBe("false");
    expect(screen.getByText("cargo test")).toBeInTheDocument();
    expect(screen.queryByText("a.ts")).not.toBeInTheDocument();

    fireEvent.click(changesTab);
    expect(useDockStore.getState().tab).toBe("changes");
    expect(screen.getAllByText("a.ts").length).toBeGreaterThan(0);
    expect(screen.queryByText("cargo test")).not.toBeInTheDocument();
  });

  test("a stored 后台 view falls back once the jobs are gone", () => {
    useDockStore.setState({ tab: "jobs" });
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[]} />);
    expect(screen.queryByRole("tab", { name: /后台/ })).not.toBeInTheDocument();
    expect(screen.getAllByText("a.ts").length).toBeGreaterThan(0);
  });

  test("后台 is a detour: the file view you were on is the one you return to", () => {
    useDockStore.setState({ tab: "artifacts", filesTab: "artifacts" });
    renderWithProviders(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[artifact("b.pdf")]}
        jobs={[job()]}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "后台（1 个任务）" }));
    expect(useDockStore.getState()).toMatchObject({
      tab: "jobs",
      filesTab: "artifacts",
    });
    fireEvent.click(screen.getByRole("tab", { name: /产物/ }));
    expect(useDockStore.getState().filesTab).toBe("artifacts");
    // Persisted, so the next launch comes back to 产物 too.
    expect(JSON.parse(localStorage.getItem("husk.dock.v1")!)).toMatchObject({
      tab: "artifacts",
      filesTab: "artifacts",
    });
  });

  test("running children get their own 子代理 view", () => {
    renderWithProviders(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[]}
        subagents={[
          {
            childId: 1,
            name: "review #1",
            task: "检查鉴权",
            status: "running",
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "子代理（1 个）" }));
    expect(useDockStore.getState().tab).toBe("subagents");
    expect(screen.getByText("review #1")).toBeInTheDocument();
    expect(screen.getByText("检查鉴权")).toBeInTheDocument();
    // The steer rides with the row, in the sidebar instead of a popover.
    expect(screen.getByPlaceholderText("插话…")).toBeInTheDocument();
    expect(screen.queryByText("a.ts")).not.toBeInTheDocument();
  });

  test("子代理 is a detour too: the file view you were on is the one you return to", () => {
    useDockStore.setState({ tab: "artifacts", filesTab: "artifacts" });
    renderWithProviders(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[artifact("b.pdf")]}
        subagents={[
          {
            childId: 2,
            name: "review #1",
            task: "检查鉴权",
            status: "running",
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /子代理/ }));
    expect(useDockStore.getState()).toMatchObject({
      tab: "subagents",
      filesTab: "artifacts",
    });
  });

  test("a finished child keeps the 子代理 view: verdict and report, no steer", () => {
    renderWithProviders(
      <SessionDock
        changes={[]}
        artifacts={[]}
        subagents={[
          {
            childId: 3,
            name: "review #1",
            task: "检查鉴权",
            status: "done",
            report: "结论：无问题",
          },
        ]}
      />,
    );
    expect(
      screen
        .getByRole("tab", { name: "子代理（1 个）" })
        .getAttribute("aria-selected"),
    ).toBe("true");
    expect(screen.getByText("已完成")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("插话…")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "报告" }));
    expect(screen.getByText("结论：无问题")).toBeInTheDocument();
  });

  test("a stored 子代理 view falls back once the children are gone", () => {
    useDockStore.setState({ tab: "subagents" });
    render(
      <SessionDock changes={[change("a.ts")]} artifacts={[]} subagents={[]} />,
    );
    expect(
      screen.queryByRole("tab", { name: /子代理/ }),
    ).not.toBeInTheDocument();
    expect(screen.getAllByText("a.ts").length).toBeGreaterThan(0);
  });

  test("the active view and open state survive a remount", () => {
    const first = render(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[artifact("b.pdf")]}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /产物/ }));
    first.unmount();
    render(
      <SessionDock
        changes={[change("a.ts")]}
        artifacts={[artifact("b.pdf")]}
      />,
    );
    expect(
      screen.getByRole("tab", { name: /产物/ }).getAttribute("aria-selected"),
    ).toBe("true");
    expect(screen.getByText("b.pdf")).toBeInTheDocument();
  });
});
