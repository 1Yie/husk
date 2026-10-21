import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test } from "vitest";
import { renderWithProviders } from "@/test/render";
import { SessionDock } from "../index";
import { DOCK_DEFAULT_WIDTH, DOCK_MAX_WIDTH, DOCK_MIN_WIDTH, useDockStore } from "../state";
import type { Artifact, BgJob, FileChange } from "@/features/chat/hooks/stream-view";

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
  useDockStore.setState({ open: true, tab: "changes", width: DOCK_DEFAULT_WIDTH });
});

describe("unified session dock", () => {
  test("holds changes and artifacts as two views of one sidebar", () => {
    render(<SessionDock changes={[change("a.ts")]} artifacts={[artifact("out/report.docx")]} />);
    // Both views are always present, whatever the workspace mode.
    expect(screen.getByRole("tab", { name: /改动/ })).toBeTruthy();
    expect(screen.getByRole("tab", { name: /产物/ })).toBeTruthy();
    expect(screen.getByText("a.ts")).toBeInTheDocument();
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
    expect(screen.getByText("a.ts")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /改动/ }).getAttribute("aria-selected")).toBe("true");
  });

  test("nothing on either side reads as an empty dock", () => {
    render(<SessionDock changes={[]} artifacts={[]} />);
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    expect(screen.getByText("本会话还没有文件产出")).toBeInTheDocument();
  });

  test("closing reports back so the page can silence this session's auto-open", () => {
    let closed = 0;
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} onClose={() => (closed += 1)} />);
    fireEvent.click(screen.getByRole("button", { name: "关闭侧栏" }));
    expect(closed).toBe(1);
    expect(useDockStore.getState().open).toBe(false);
  });

  test("the page's `open` prop wins over the persisted flag", () => {
    // A new session inherits the previous one's persisted `open`; the page
    // ANDs in "has content", so an empty dock must not occupy the column.
    const { container } = render(<SessionDock changes={[]} artifacts={[]} open={false} />);
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
    for (let i = 0; i < 40; i++) fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useDockStore.getState().width).toBe(DOCK_MIN_WIDTH);
    for (let i = 0; i < 40; i++) fireEvent.keyDown(handle, { key: "ArrowLeft" });
    expect(useDockStore.getState().width).toBe(DOCK_MAX_WIDTH);
    const saved = JSON.parse(localStorage.getItem("husk.dock.v1")!);
    expect(saved.width).toBe(DOCK_MAX_WIDTH);
    expect(saved.open).toBe(true);
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
    renderWithProviders(<SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[job]} />);
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
    };
    renderWithProviders(<SessionDock changes={[]} artifacts={[]} jobs={[job]} />);
    fireEvent.click(screen.getByRole("tab", { name: "后台（1 个任务）" }));
    // The record can be dropped, so the bulk action counts it…
    expect(screen.getByText("清除已结束")).toBeInTheDocument();
    // …and the row offers its own removal, but nothing that needs a process.
    expect(screen.queryByRole("button", { name: "终止" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /bg-1/ }));
    expect(screen.getByText("日志随任务进程一起丢失")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "移除记录" })).toBeInTheDocument();
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
    expect(screen.getByText("a.ts")).toBeInTheDocument();
    expect(screen.queryByText("cargo test")).not.toBeInTheDocument();
  });

  test("a stored 后台 view falls back once the jobs are gone", () => {
    useDockStore.setState({ tab: "jobs" });
    render(<SessionDock changes={[change("a.ts")]} artifacts={[]} jobs={[]} />);
    expect(screen.queryByRole("tab", { name: /后台/ })).not.toBeInTheDocument();
    expect(screen.getByText("a.ts")).toBeInTheDocument();
  });

  test("后台 is a detour: the file view you were on is the one you return to", () => {
    useDockStore.setState({ tab: "artifacts", filesTab: "artifacts" });
    renderWithProviders(
      <SessionDock changes={[change("a.ts")]} artifacts={[artifact("b.pdf")]} jobs={[job()]} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "后台（1 个任务）" }));
    expect(useDockStore.getState()).toMatchObject({ tab: "jobs", filesTab: "artifacts" });
    fireEvent.click(screen.getByRole("tab", { name: /产物/ }));
    expect(useDockStore.getState().filesTab).toBe("artifacts");
    // Persisted, so the next launch comes back to 产物 too.
    expect(JSON.parse(localStorage.getItem("husk.dock.v1")!)).toMatchObject({
      tab: "artifacts",
      filesTab: "artifacts",
    });
  });

  test("the active view and open state survive a remount", () => {
    const first = render(
      <SessionDock changes={[change("a.ts")]} artifacts={[artifact("b.pdf")]} />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /产物/ }));
    first.unmount();
    render(<SessionDock changes={[change("a.ts")]} artifacts={[artifact("b.pdf")]} />);
    expect(screen.getByRole("tab", { name: /产物/ }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("b.pdf")).toBeInTheDocument();
  });
});
