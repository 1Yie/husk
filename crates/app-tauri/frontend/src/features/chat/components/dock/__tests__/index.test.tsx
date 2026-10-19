import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test } from "vitest";
import { SessionDock } from "../index";
import { DOCK_DEFAULT_WIDTH, DOCK_MAX_WIDTH, DOCK_MIN_WIDTH, useDockStore } from "../state";
import type { Artifact, FileChange } from "@/features/chat/hooks/stream-view";

const change = (path: string): FileChange => ({
  path,
  kind: "update",
  patches: [{ n: 1, tool: "fuzzy_patch", diff: "@@ -1 +1 @@\n-old\n+new" }],
  adds: 1,
  dels: 1,
  fuzzy: false,
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
