import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Profiler, useState } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { TerminalPanel } from "../terminal-panel";
import { TerminalDock } from "../terminal-dock";
import { renderWithProviders as render } from "@/test/render";
import { TerminalToggle } from "@/components/layout-controls";
import { useLayoutStore } from "@/app/layout/layout-store";
import { isLayoutResizing } from "@/lib/layout-resize";
import { DEFAULT_TERMINAL_PREFS, useTerminalPrefs } from "../preferences";
import type { SessionState, TerminalAppearance } from "../terminal-session";

type FakeSession = {
  cwd?: string;
  update: (state: SessionState) => void;
  appearance: TerminalAppearance;
  setVisible: ReturnType<typeof vi.fn>;
  setAppearance: ReturnType<typeof vi.fn>;
  dispose: ReturnType<typeof vi.fn>;
  scheduleFit: ReturnType<typeof vi.fn>;
};
const sessions = vi.hoisted(() => [] as FakeSession[]);
let paintedHeight: number | undefined;
vi.mock("../terminal-session", () => ({
  TerminalSession: class {
    setVisible = vi.fn();
    focus = vi.fn();
    dispose = vi.fn();
    scheduleFit = vi.fn();
    remeasureFonts = vi.fn();
    setAppearance = vi.fn();
    hasSelection = vi.fn(() => false);
    constructor(
      _host: HTMLElement,
      public cwd: string | undefined,
      public update: (state: unknown) => void,
      public appearance: unknown,
    ) {
      sessions.push(this as unknown as FakeSession);
      update({ status: "running", cwd, shell: "/bin/sh" });
    }
  },
}));
vi.mock("@/lib/settings-bus", () => ({ openSettings: vi.fn() }));

beforeEach(() => {
  sessions.length = 0;
  localStorage.clear();
  useTerminalPrefs.setState(DEFAULT_TERMINAL_PREFS);
  useLayoutStore.setState({ terminalOpen: false });
  paintedHeight = undefined;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const panel = this.closest(".terminal-panel") as HTMLElement | null;
    const height = paintedHeight ?? (Number.parseFloat((panel ?? this).style.height) || 0);
    return new DOMRect(0, 280 - height, 800, height);
  });
});
afterEach(() => vi.useRealTimers());

test("settings update visible panes only and hidden tabs catch up once when selected", async () => {
  render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  sessions.forEach((session) => session.setAppearance.mockClear());
  act(() => useTerminalPrefs.setState({ fontSize: 18 }));
  act(() => useTerminalPrefs.setState({ fontSize: 20 }));
  expect(sessions[0].setAppearance).not.toHaveBeenCalled();
  expect(sessions[1].setAppearance).toHaveBeenLastCalledWith(expect.objectContaining({ fontSize: 20 }));
  fireEvent.click(screen.getAllByRole("tab")[0]);
  expect(sessions[0].setAppearance).toHaveBeenCalledOnce();
  expect(sessions[0].setAppearance).toHaveBeenCalledWith(expect.objectContaining({ fontSize: 20 }));
  expect(sessions).toHaveLength(2);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
});

test("terminal font changes are independent of the app code font and retain the live session", async () => {
  document.documentElement.style.setProperty("--husk-code-font", "Consolas");
  render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  expect(sessions[0].appearance.fontFamily).toBe('"Husk Terminal Mono", monospace');
  act(() => useTerminalPrefs.getState().update({ fontFamily: '"Iosevka Nerd Font Mono"' }));
  await waitFor(() => expect(sessions[0].setAppearance).toHaveBeenLastCalledWith(
    expect.objectContaining({ fontFamily: '"Iosevka Nerd Font Mono", "Husk Terminal Mono", monospace' }),
  ));
  expect(sessions).toHaveLength(1);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  document.documentElement.style.removeProperty("--husk-code-font");
});

test("shows the real panel before initializing xterm at its final size", async () => {
  paintedHeight = 40;
  const { container } = render(<TerminalPanel open onOpenChange={() => {}} />);
  expect(screen.getByRole("button", { name: "新建终端" })).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent("正在启动终端");
  expect(screen.getByRole("status")).toHaveClass("terminal-panel__loading");
  expect(container.querySelector(".terminal-panel__status")).toBeNull();
  expect(screen.getByRole("status").querySelector(".sr-only")).toHaveTextContent("正在启动终端");
  expect(screen.getByRole("status").querySelectorAll(".terminal-panel__skeleton-line").length).toBeGreaterThanOrEqual(4);
  expect(container.querySelector<HTMLElement>(".terminal-panel__inner")?.style.height).toBe("");
  expect(container.querySelector(".terminal-panel")).toHaveStyle({ height: "280px" });
  expect(sessions).toHaveLength(0);
  await new Promise((resolve) => setTimeout(resolve, 60));
  expect(sessions).toHaveLength(0);
  paintedHeight = 280;
  await waitFor(() => expect(sessions).toHaveLength(1));
  expect(container.querySelector(".terminal-panel__loading")).toBeNull();
  expect(container.querySelector(".terminal-panel")).toHaveStyle({ height: "280px" });
});

test("closing during the reveal cancels startup instead of creating a hidden shell", () => {
  vi.useFakeTimers();
  paintedHeight = 40;
  const view = render(<TerminalPanel open onOpenChange={() => {}} />);
  view.rerender(<TerminalPanel open={false} onOpenChange={() => {}} />);
  paintedHeight = 280;
  act(() => vi.advanceTimersByTime(1000));
  expect(sessions).toHaveLength(0);
});

test("only starts after opening, and hiding/reopening preserves the process", async () => {
  const onOpenChange = vi.fn();
  const view = render(
    <TerminalPanel open={false} onOpenChange={onOpenChange} cwd="/first" />,
  );
  expect(sessions).toHaveLength(0);
  expect(screen.getByLabelText("终端面板")).toHaveAttribute("inert");
  view.rerender(
    <TerminalPanel open onOpenChange={onOpenChange} cwd="/first" />,
  );
  await waitFor(() => expect(sessions).toHaveLength(1));
  view.rerender(
    <TerminalPanel open={false} onOpenChange={onOpenChange} cwd="/first" />,
  );
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  view.rerender(
    <TerminalPanel open onOpenChange={onOpenChange} cwd="/second" />,
  );
  expect(sessions).toHaveLength(1);
  expect(sessions[0].cwd).toBe("/first");
  view.unmount();
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
});

test("tabs are named after their directory and new ones use the current workspace", async () => {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();
  const view = render(
    <TerminalPanel open onOpenChange={onOpenChange} cwd="/work/first" />,
  );
  await waitFor(() => expect(sessions).toHaveLength(1));
  expect(screen.getByRole("tab", { name: /first/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  view.rerender(
    <TerminalPanel open onOpenChange={onOpenChange} cwd="/work/second" />,
  );
  await user.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  expect(sessions[1].cwd).toBe("/work/second");
  expect(screen.getByRole("tab", { name: /second/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await user.click(screen.getByRole("button", { name: "关闭终端 second" }));
  expect(sessions[1].dispose).toHaveBeenCalledOnce();
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "隐藏终端面板" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
});

test("a title set by the shell replaces the directory name", async () => {
  render(<TerminalPanel open onOpenChange={() => {}} cwd="/work/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  act(() =>
    sessions[0].update({ status: "running", cwd: "/work/repo", title: "htop" }),
  );
  expect(screen.getByRole("tab", { name: /htop/ })).toBeInTheDocument();
});

test("a clean exit closes its tab; closing the last tab hides the panel", async () => {
  const onOpenChange = vi.fn();
  render(<TerminalPanel open onOpenChange={onOpenChange} cwd="/work/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  act(() =>
    sessions[0].update({
      status: "exited",
      cwd: "/work/repo",
      code: 0,
      signal: null,
    }),
  );
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  expect(screen.queryByRole("tab")).toBeNull();
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
});

test("a failed exit keeps the tab and offers a restart", async () => {
  const user = userEvent.setup();
  render(<TerminalPanel open onOpenChange={() => {}} cwd="/work/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  act(() =>
    sessions[0].update({
      status: "exited",
      cwd: "/work/repo",
      code: 2,
      signal: null,
    }),
  );
  expect(screen.getByRole("status")).toHaveTextContent("退出码 2");
  await user.click(screen.getByRole("button", { name: /重新启动/ }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
});

test("closing the last tab keeps the shell until the closing surface has painted", async () => {
  function Controlled() {
    const [open, setOpen] = useState(true);
    return <TerminalPanel open={open} onOpenChange={setOpen} cwd="/repo" />;
  }
  const view = render(<Controlled />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  vi.useFakeTimers();
  fireEvent.click(screen.getByRole("button", { name: "关闭终端 repo" }));
  expect(view.container.querySelector(".terminal-panel__inner")).not.toBeNull();
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
  act(() => vi.advanceTimersByTime(100));
  expect(view.container.querySelector(".terminal-panel__inner")).toBeNull();
});

test("preference changes reach the open terminals without restarting them", async () => {
  render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  expect(sessions[0].appearance.fontSize).toBe(12);
  act(() =>
    useTerminalPrefs.getState().update({ fontSize: 16, cursorStyle: "bar" }),
  );
  await waitFor(() =>
    expect(sessions[0].setAppearance).toHaveBeenLastCalledWith(
      expect.objectContaining({ fontSize: 16, cursorStyle: "bar" }),
    ),
  );
  expect(sessions).toHaveLength(1);
});

test("the height separator is keyboard operable and remembered", () => {
  render(<TerminalPanel open onOpenChange={() => {}} />);
  const handle = screen.getByRole("separator", { name: "调整终端高度" });
  const before = Number(handle.getAttribute("aria-valuenow"));
  fireEvent.keyDown(handle, { key: "ArrowUp" });
  expect(Number(handle.getAttribute("aria-valuenow"))).toBe(before + 20);
  expect(useTerminalPrefs.getState().panelHeight).toBe(before + 20);
  fireEvent.keyDown(handle, { key: "ArrowDown" });
  expect(Number(handle.getAttribute("aria-valuenow"))).toBe(before);
});

test("pointer moves preview at frame rate without React commits until release", async () => {
  const onRender = vi.fn();
  const { container } = render(
    <Profiler id="terminal-resize" onRender={onRender}>
      <TerminalPanel open onOpenChange={() => {}} />
    </Profiler>,
  );
  await waitFor(() => expect(sessions).toHaveLength(1));
  vi.useFakeTimers();
  const handle = screen.getByRole("separator", { name: "调整终端高度" });
  fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientY: 500 });
  onRender.mockClear();
  for (let n = 1; n <= 20; n++) {
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 500 - n * 2 });
  }
  expect(onRender).not.toHaveBeenCalled();
  act(() => vi.advanceTimersByTime(20));
  expect(container.querySelector(".terminal-panel")).toHaveStyle({ height: "320px" });
  expect(container.querySelector<HTMLElement>(".terminal-panel__inner")?.style.height).toBe("");
  expect(handle).toHaveAttribute("aria-valuenow", "320");
  expect(useTerminalPrefs.getState().panelHeight).toBe(280);
  expect(onRender).not.toHaveBeenCalled();
  fireEvent.pointerUp(handle, { pointerId: 1, clientY: 460 });
  expect(useTerminalPrefs.getState().panelHeight).toBe(320);
  expect(onRender).toHaveBeenCalledTimes(1);
});

test("hiding during a drag discards the preview and cancels later frame writes", async () => {
  const onOpenChange = vi.fn();
  const view = render(<TerminalPanel open onOpenChange={onOpenChange} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  vi.useFakeTimers();
  const handle = screen.getByRole("separator", { name: "调整终端高度" });
  fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientY: 500 });
  fireEvent.pointerMove(handle, { pointerId: 1, clientY: 400 });
  act(() => vi.advanceTimersByTime(20));
  expect(handle).toHaveAttribute("aria-valuenow", "380");
  view.rerender(<TerminalPanel open={false} onOpenChange={onOpenChange} />);
  act(() => vi.advanceTimersByTime(100));
  expect(view.container.querySelector(".terminal-panel")).toHaveStyle({ height: "0px" });
  expect(view.container.querySelector<HTMLElement>(".terminal-panel__inner")?.style.height).toBe("");
  expect(handle).toHaveAttribute("aria-valuenow", "280");
  expect(useTerminalPrefs.getState().panelHeight).toBe(280);
  view.rerender(<TerminalPanel open onOpenChange={onOpenChange} />);
  // jsdom applies the target height immediately; the shell must still survive.
  expect(view.container.querySelector(".terminal-panel")).toHaveStyle({ height: "280px" });
  act(() => vi.advanceTimersByTime(100));
  expect(view.container.querySelector(".terminal-panel")).toHaveStyle({ height: "280px" });
  expect(sessions).toHaveLength(1);
});

test("the active screen stays rendered while closing, without accepting input or restarting", async () => {
  const view = render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const screen = view.container.querySelector('[role="tabpanel"]');
  view.rerender(<TerminalPanel open={false} onOpenChange={() => {}} />);
  expect(screen).not.toHaveAttribute("hidden");
  expect(screen).toHaveAttribute("inert");
  expect(sessions[0].setVisible).toHaveBeenLastCalledWith(false);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  view.unmount();
  expect(isLayoutResizing()).toBe(false);
});

test("hiding the terminal dismisses its portalled context menu without restarting xterm", async () => {
  const view = render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.contextMenu(view.container.querySelector(".terminal-panel__screen")!, { clientX: 50, clientY: 50 });
  expect(await screen.findByRole("menu")).toHaveClass("min-w-[160px]");
  for (const item of screen.getAllByRole("menuitem")) expect(item).toHaveClass("text-xs");
  view.rerender(<TerminalPanel open={false} onOpenChange={() => {}} />);
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  expect(sessions[0].dispose).not.toHaveBeenCalled();
});

test("the connected terminal toggles without rendering its app shell", async () => {
  const renders = vi.fn();
  function Shell() {
    renders();
    return <TerminalDock cwd="/repo" />;
  }
  render(<Shell />);
  act(() => useLayoutStore.getState().toggleTerminal());
  await waitFor(() => expect(sessions).toHaveLength(1));
  act(() => useLayoutStore.getState().toggleTerminal());
  expect(renders).toHaveBeenCalledOnce();
  expect(sessions[0].dispose).not.toHaveBeenCalled();
});

test("new terminal tabs respect the backend's eight-session limit", async () => {
  const user = userEvent.setup();
  render(<TerminalPanel open onOpenChange={() => {}} />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const add = screen.getByRole("button", { name: "新建终端" });
  for (let count = 1; count < 8; count++) await user.click(add);
  await waitFor(() => expect(sessions).toHaveLength(8));
  expect(add).toBeDisabled();
});

test("a new tab paints its pending view before constructing another renderer", async () => {
  render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.click(screen.getByRole("button", { name: "新建终端" }));
  expect(screen.getAllByRole("tab")).toHaveLength(2);
  expect(screen.getByRole("status")).toHaveTextContent("正在启动终端");
  expect(sessions).toHaveLength(1);
  await waitFor(() => expect(sessions).toHaveLength(2));
});

test("nested splits retain existing host nodes and close only their own pane", async () => {
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const host = view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen');
  await user.click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  expect(screen.getAllByRole("tab")).toHaveLength(1);
  expect(within(screen.getByRole("tabpanel")).getAllByRole("region")).toHaveLength(2);
  expect(view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen')).toBe(host);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  const first = view.container.querySelector('[data-pane-key="1"]')!;
  fireEvent.pointerDown(first, { button: 0, pointerId: 40 });
  expect(sessions[0].setVisible).toHaveBeenLastCalledWith(true, true);
  expect(sessions[1].setVisible).toHaveBeenLastCalledWith(true, false);
  await user.click(screen.getByRole("button", { name: "上下分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  expect(within(screen.getByRole("tabpanel")).getAllByRole("region")).toHaveLength(3);
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "50%", height: "50%" });
  expect(view.container.querySelector('[data-pane-key="2"]')).toHaveStyle({ left: "50%", height: "100%" });
  expect(view.container.querySelector('[data-pane-key="3"]')).toHaveStyle({ top: "50%", width: "50%" });
  await user.click(within(view.container.querySelector('[data-pane-key="3"]') as HTMLElement).getByRole("button", { name: "关闭窗格 repo" }));
  expect(sessions[2].dispose).toHaveBeenCalledOnce();
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  expect(sessions[1].dispose).not.toHaveBeenCalled();
  expect(view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen')).toBe(host);
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "50%", height: "100%" });
  await user.click(within(view.container.querySelector('[data-pane-key="2"]') as HTMLElement).getByRole("button", { name: "关闭窗格 repo" }));
  expect(view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen')).toBe(host);
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "100%", height: "100%" });
  expect(screen.queryByRole("separator", { name: "调整终端分屏宽度" })).toBeNull();
});

test("switching split groups remembers the focused pane and group close ends only that group", async () => {
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  await user.click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  await user.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  const newest = view.container.querySelector('[data-pane-key="3"] .terminal-panel__screen');
  await user.click(screen.getAllByRole("tab")[0]);
  expect(view.container.querySelector('[data-pane-key="2"]')).toHaveAttribute("data-focused", "true");
  expect(sessions[0].setVisible).toHaveBeenLastCalledWith(true, false);
  expect(sessions[1].setVisible).toHaveBeenLastCalledWith(true, true);
  expect(sessions[2].setVisible).toHaveBeenLastCalledWith(false);
  await user.click(screen.getByRole("button", { name: "关闭终端组 repo" }));
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
  expect(sessions[1].dispose).toHaveBeenCalledOnce();
  expect(sessions[2].dispose).not.toHaveBeenCalled();
  expect(view.container.querySelector('[data-pane-key="3"] .terminal-panel__screen')).toBe(newest);
  expect(screen.getAllByRole("tab")).toHaveLength(1);
});

test("closing a pending split cancels startup without creating a hidden shell", async () => {
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.click(screen.getByRole("button", { name: "左右分屏" }));
  const pending = view.container.querySelector('[data-pane-key="2"]') as HTMLElement;
  fireEvent.click(within(pending).getByRole("button", { name: "关闭窗格 repo" }));
  await new Promise((resolve) => setTimeout(resolve, 150));
  expect(sessions).toHaveLength(1);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  expect(view.container.querySelectorAll("[data-pane-key]")).toHaveLength(1);
});

test("tabs and split panes share the same eight-terminal limit", async () => {
  const user = userEvent.setup();
  render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  for (let group = 0; group < 4; group++) {
    if (group) await user.click(screen.getByRole("button", { name: "新建终端" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "左右分屏" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "左右分屏" }));
  }
  await waitFor(() => expect(sessions).toHaveLength(8));
  expect(screen.getAllByRole("tab")).toHaveLength(4);
  expect(screen.getByRole("button", { name: "新建终端" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "左右分屏" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "上下分屏" })).toBeDisabled();
});

test("split dragging previews without React renders and fits only after release", async () => {
  const onRender = vi.fn();
  const view = render(<Profiler id="split" onRender={onRender}><TerminalPanel open onOpenChange={() => {}} cwd="/repo" /></Profiler>);
  await waitFor(() => expect(sessions).toHaveLength(1));
  await userEvent.setup().click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  vi.useFakeTimers();
  const handle = screen.getByRole("separator", { name: "调整终端分屏宽度" });
  fireEvent.pointerDown(handle, { button: 0, pointerId: 30, clientX: 400, clientY: 100 });
  onRender.mockClear();
  for (const session of sessions) session.scheduleFit.mockClear();
  fireEvent.pointerMove(handle, { pointerId: 30, clientX: 520, clientY: 100 });
  act(() => vi.advanceTimersByTime(20));
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "65%" });
  expect(onRender).not.toHaveBeenCalled();
  for (const session of sessions) expect(session.scheduleFit).not.toHaveBeenCalled();
  fireEvent.pointerUp(handle, { pointerId: 30, clientX: 520, clientY: 100 });
  act(() => vi.advanceTimersByTime(100));
  for (const session of sessions) {
    expect(session.scheduleFit).toHaveBeenCalledWith({ immediate: true });
    expect(session.dispose).not.toHaveBeenCalled();
  }
});

test("tab menus rename and restore labels without recreating the renderer", async () => {
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const host = view.container.querySelector(".terminal-panel__screen");
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  expect(await screen.findByRole("menu")).toHaveClass("min-w-[160px]");
  for (const item of screen.getAllByRole("menuitem")) expect(item).toHaveClass("text-xs");
  await user.click(screen.getByRole("menuitem", { name: "重命名" }));
  const input = screen.getByRole("textbox", { name: "终端标签名称" });
  await user.clear(input);
  await user.type(input, "API");
  await user.keyboard("{Enter}");
  expect(screen.getByRole("tab", { name: "API" })).toBeInTheDocument();
  expect(view.container.querySelector(".terminal-panel__screen")).toBe(host);
  expect(sessions).toHaveLength(1);
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  await user.click(await screen.findByRole("menuitem", { name: "恢复默认名称" }));
  expect(screen.getByRole("tab", { name: "repo" })).toBeInTheDocument();
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  await user.click(await screen.findByRole("menuitem", { name: "重命名" }));
  await user.clear(screen.getByRole("textbox", { name: "终端标签名称" }));
  await user.type(screen.getByRole("textbox", { name: "终端标签名称" }), "cancelled");
  await user.keyboard("{Escape}");
  expect(screen.getByRole("tab", { name: "repo" })).toBeInTheDocument();
});

test("an inactive tab's menu splits the targeted group without replacing either existing shell", async () => {
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const first = view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen');
  await user.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  fireEvent.contextMenu(screen.getAllByRole("tab")[0], { clientX: 50, clientY: 50 });
  expect(screen.getAllByRole("tab")[1]).toHaveAttribute("aria-selected", "true");
  await user.click(await screen.findByRole("menuitem", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  expect(screen.getAllByRole("tab")[0]).toHaveAttribute("aria-selected", "true");
  expect(within(screen.getByRole("tabpanel")).getAllByRole("region")).toHaveLength(2);
  expect(view.container.querySelector('[data-pane-key="1"] .terminal-panel__screen')).toBe(first);
  for (const session of sessions) expect(session.dispose).not.toHaveBeenCalled();
});

test("tab context actions close other groups or all groups, not the target group's individual panes", async () => {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();
  render(<TerminalPanel open onOpenChange={onOpenChange} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  await user.click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  await user.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  fireEvent.contextMenu(screen.getAllByRole("tab")[0], { clientX: 50, clientY: 50 });
  await user.click(await screen.findByRole("menuitem", { name: "关闭其他标签" }));
  expect(screen.getAllByRole("tab")).toHaveLength(1);
  expect(within(screen.getByRole("tabpanel")).getAllByRole("region")).toHaveLength(2);
  expect(sessions[2].dispose).toHaveBeenCalledOnce();
  expect(sessions[0].dispose).not.toHaveBeenCalled();
  expect(sessions[1].dispose).not.toHaveBeenCalled();
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  await user.click(await screen.findByRole("menuitem", { name: "关闭全部终端" }));
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
  expect(sessions[1].dispose).toHaveBeenCalledOnce();
});

test("hiding the panel also dismisses a portalled tab context menu", async () => {
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  await screen.findByRole("menu");
  view.rerender(<TerminalPanel open={false} onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  expect(sessions[0].dispose).not.toHaveBeenCalled();
});

test("split keyboard bounds preserve usable pane sizes and double click balances them", async () => {
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  await user.click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  const handle = screen.getByRole("separator", { name: "调整终端分屏宽度" });
  fireEvent.keyDown(handle, { key: "Home" });
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "15%" });
  fireEvent.keyDown(handle, { key: "End" });
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "85%" });
  fireEvent.doubleClick(handle);
  expect(view.container.querySelector('[data-pane-key="1"]')).toHaveStyle({ width: "50%" });
  expect(sessions).toHaveLength(2);
});

test.each([
  { button: "左右分屏", separator: "调整终端分屏宽度", dimension: "width" as const, minimum: 120 },
  { button: "上下分屏", separator: "调整终端分屏高度", dimension: "height" as const, minimum: 70 },
])("resizing an ancestor preserves minimum $dimension in an uneven nested split", async ({ button, separator, dimension, minimum }) => {
  useTerminalPrefs.setState({ panelHeight: 560 });
  paintedHeight = 560;
  const user = userEvent.setup();
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  await user.click(screen.getByRole("button", { name: button }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  await user.click(screen.getByRole("button", { name: button }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  const [outer, inner] = screen.getAllByRole("separator", { name: separator });
  fireEvent.keyDown(inner, { key: "Home" });
  fireEvent.keyDown(outer, { key: "End" });
  for (const pane of view.container.querySelectorAll<HTMLElement>("[data-pane-key]")) {
    const pixels = Number.parseFloat(pane.style[dimension]) * (dimension === "width" ? 800 : 560) / 100;
    expect(pixels).toBeGreaterThanOrEqual(minimum - 0.01);
  }
  expect(sessions).toHaveLength(3);
  for (const session of sessions) expect(session.dispose).not.toHaveBeenCalled();
});

test("saving a tab name on blur does not pull focus back from the clicked control", async () => {
  const user = userEvent.setup();
  render(<TerminalPanel open onOpenChange={() => {}} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.contextMenu(screen.getByRole("tab"), { clientX: 50, clientY: 50 });
  await user.click(await screen.findByRole("menuitem", { name: "重命名" }));
  const input = screen.getByRole("textbox", { name: "终端标签名称" });
  await user.clear(input);
  await user.type(input, "saved");
  const settings = screen.getByRole("button", { name: "终端设置" });
  await user.click(settings);
  await act(async () => {
    await new Promise<void>((resolve) => setTimeout(resolve, 40));
  });
  expect(settings).toHaveFocus();
  expect(screen.getByRole("tab", { name: "saved" })).toBeInTheDocument();
});

test("terminal screen menu has aligned decorative icons and a destructive close action", async () => {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();
  const view = render(<TerminalPanel open onOpenChange={onOpenChange} cwd="/repo" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  fireEvent.contextMenu(view.container.querySelector(".terminal-panel__screen")!, { clientX: 50, clientY: 50 });
  const menu = await screen.findByRole("menu");
  const items = within(menu).getAllByRole("menuitem");
  expect(items).toHaveLength(6);
  for (const item of items) {
    const icon = item.querySelector("svg");
    expect(icon).toHaveAttribute("aria-hidden", "true");
    expect(icon).toHaveClass("h-3.5", "w-3.5", "shrink-0");
    expect(item).toHaveClass("gap-2", "text-xs");
  }
  const close = within(menu).getByRole("menuitem", { name: "关闭终端" });
  expect(close).toHaveClass("text-red-600", "focus:text-red-600", "dark:text-red-400", "dark:focus:text-red-400");
  await user.click(close);
  expect(sessions[0].dispose).toHaveBeenCalledOnce();
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("sidebar and terminal toolbar share the hide-terminal tooltip and shortcut styling", async () => {
  const user = userEvent.setup();
  render(<>
    <TerminalToggle open onToggle={() => {}} />
    <TerminalPanel open onOpenChange={() => {}} cwd="/repo" />
  </>);
  await waitFor(() => expect(sessions).toHaveLength(1));
  const inspect = async (name: string) => {
    const button = screen.getByRole("button", { name });
    await user.hover(button);
    const tooltip = await screen.findByRole("tooltip");
    const result = { text: tooltip.textContent, shortcut: tooltip.querySelector("kbd")?.className };
    expect(result.shortcut).toBeTruthy();
    await user.unhover(button);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
    return result;
  };
  expect(await inspect("隐藏终端面板")).toEqual(await inspect("隐藏终端"));
  expect(screen.getByRole("button", { name: "隐藏终端面板" })).not.toHaveAttribute("title");
});

test("reordering split groups retains every renderer, process and the selected pane", async () => {
  const view = render(<TerminalPanel open onOpenChange={() => {}} cwd="/first" />);
  await waitFor(() => expect(sessions).toHaveLength(1));
  view.rerender(<TerminalPanel open onOpenChange={() => {}} cwd="/second" />);
  fireEvent.click(screen.getByRole("button", { name: "新建终端" }));
  await waitFor(() => expect(sessions).toHaveLength(2));
  fireEvent.click(screen.getByRole("button", { name: "左右分屏" }));
  await waitFor(() => expect(sessions).toHaveLength(3));
  const hosts = [...view.container.querySelectorAll(".terminal-panel__screen")];
  const second = screen.getByRole("tab", { name: /second/ });
  const selectedPane = view.container.querySelector('[data-focused="true"][data-pane-key]');
  second.focus();
  fireEvent.keyDown(second, { key: "ArrowLeft", altKey: true });
  expect(screen.getAllByRole("tab")[0]).toBe(second);
  expect(second).toHaveAttribute("aria-selected", "true");
  expect(second).toHaveFocus();
  expect(view.container.querySelector('[data-focused="true"][data-pane-key]')).toBe(selectedPane);
  expect(hosts.every((host) => host.isConnected)).toBe(true);
  fireEvent.contextMenu(second);
  fireEvent.click(screen.getByRole("menuitem", { name: "向右移动标签" }));
  expect(screen.getAllByRole("tab")[1]).toBe(second);
  expect(sessions).toHaveLength(3);
  for (const session of sessions) expect(session.dispose).not.toHaveBeenCalled();
  expect(hosts.every((host) => host.isConnected)).toBe(true);
});
