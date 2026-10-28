import type { ComponentProps, ReactNode } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { App } from "../App";
import { useAgentStore } from "@/stores/agent-store";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { deferred, mockCommand, tauri } from "@/test/tauri";
import type { ProjectSessionRow, SidebarSummary } from "@/types";

// Keep the real App, session hook, view store and Tauri boundary. Only replace
// heavy rendering surfaces; these tests exercise boot/navigation/eviction.
vi.mock("@/features/terminal/terminal-dock", () => ({ TerminalDock: () => null }));
vi.mock("@/app/layout/main-layout/index", () => ({ MainLayout: ({ sidebar, children }: { sidebar: ReactNode; children: ReactNode }) => <>{sidebar}{children}</> }));
vi.mock("@/features/chat/components/session-sidebar/index", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/features/chat/components/session-sidebar/index")>();
  return { SessionSidebar: (props: ComponentProps<typeof actual.SessionSidebar>) => <>
    <button onClick={props.onNew}>new session</button><button onClick={() => props.onOpenSession("/a", 70)}>open 70</button>
    <actual.SessionSidebar {...props} />
  </> };
});
vi.mock("@/components/title-bar", () => ({ TitleBar: ({ title }: { title: string }) => <div data-testid="active-title">{title}</div> }));
vi.mock("@/features/chat/components/chat-stream", () => ({ ChatStream: ({ loading }: { loading: boolean }) => <div data-testid="chat-stream" data-loading={String(loading)} /> }));
vi.mock("@/features/chat/components/composer-bar", () => ({ ComposerBar: () => <button data-testid="composer">send</button> }));
vi.mock("@/features/chat/components/dock", () => ({ SessionDock: () => null }));
vi.mock("@/features/settings/pages/index", () => ({ SettingsPage: () => null }));
vi.mock("@/features/update/use-update", () => ({ useStartupUpdateCheck: () => {} }));
vi.mock("@/features/update/update-dialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/features/chat/components/workspace-loading-veil/index", () => ({ WorkspaceLoadingVeil: ({ visible }: { visible: boolean }) => visible ? <div data-testid="switch-veil" /> : null }));
vi.mock("@/components/window-resize-edges", () => ({ WindowResizeEdges: () => null }));

const row = (id: number): ProjectSessionRow => ({ id, title: `session ${id}`, preview: "", updated_at: id,
  active: true, running: false, pinned: false, archived: false });
beforeEach(() => {
  useAgentStore.setState(useAgentStore.getInitialState(), true);
});
function install() {
  const state = { active: 2, generation: "1" };
  mockCommand("agent_session", () => ({ root: "/a", name: "a", recents: [] }), "workspace_info");
  const summary = (): SidebarSummary => ({ generation: state.generation,
    projects: [{ root: "/a", name: "a", last_opened: 0, current: true, total: 100, archived_total: 0 }],
    recents: [{ root: "/a", project: "a", current: true, row: { ...row(70), active: false } }],
    active: { root: "/a", project: "a", current: true, row: row(state.active) } });
  mockCommand("agent_session", summary, "sidebar");
  mockCommand("agent_session", () => ({ generation: state.generation, root: "/a", archived: false, rows: [], next_cursor: null }), "sidebar_page");
  const open = vi.fn((args: Record<string, unknown> | undefined) => {
    state.active = args!.id as number; state.generation = String(Number(state.generation) + 1);
    return { active: state.active, history: [] };
  });
  mockCommand("agent_session", open, "open");
  mockCommand("agent_session", () => { state.active = 101; state.generation = "9"; return { active: 101 }; }, "new");
  return { state, open, summary };
}
const flush = async () => { await act(async () => { await Promise.resolve(); }); };

test("boot restores the independently returned older active row, not the first visible recent", async () => {
  const { open } = install();
  render(<App />);
  await flush();
  expect(open.mock.calls[0][0]?.id).toBe(2);
  expect(useAgentStore.getState().activeId).toBe(2);
  expect(screen.getByTestId("active-title")).toHaveTextContent("session 2");
  expect(tauri.invoke.mock.calls.some(([, args]) => args?.op === "projects")).toBe(false);
});

test("leaving an off-page persisted conversation does not delete its cached view", async () => {
  install();
  useAgentStore.getState().loadView("/a", 2, emptyView());
  const exists = vi.fn(() => ({ known_project: true, row: row(2) }));
  mockCommand("agent_session", exists, "sidebar_row");
  render(<App />);
  await flush();
  fireEvent.click(screen.getByText("open 70"));
  await flush();
  expect(exists).toHaveBeenCalledWith(expect.objectContaining({ path: "/a", id: 2 }));
  expect(useAgentStore.getState().views.has("/a:2")).toBe(true);
  expect(useAgentStore.getState().activeId).toBe(70);
});

test("new-session eviction uses an explicit tombstone and never the same id in another workspace", async () => {
  install();
  useAgentStore.getState().loadView("/a", 2, emptyView());
  useAgentStore.getState().loadView("/b", 2, emptyView());
  const exists = vi.fn(() => ({ known_project: true, row: null }));
  mockCommand("agent_session", exists, "sidebar_row");
  render(<App />);
  await flush();
  fireEvent.click(screen.getByText("new session"));
  await flush();
  expect(exists).toHaveBeenCalledWith(expect.objectContaining({ path: "/a", id: 2 }));
  expect(useAgentStore.getState().views.has("/a:2")).toBe(false);
  expect(useAgentStore.getState().views.has("/b:2")).toBe(true);
  expect(useAgentStore.getState().activeId).toBe(101);
});

test("archive-only boot resolves to welcome, preserves archive navigation, and new restores a real composer", async () => {
  vi.useFakeTimers();
  const { state, open } = install();
  const archived = { ...row(9), active: false, archived: true };
  const summary = (): SidebarSummary => ({ generation: state.generation,
    projects: [{ root: "/a", name: "a", last_opened: 0, current: true, total: state.active ? 1 : 0, archived_total: 1 }],
    recents: [], active: state.active ? { root: "/a", project: "a", current: true, row: row(state.active) } : null });
  state.active = 0;
  const startup = deferred<SidebarSummary>();
  mockCommand("agent_session", () => startup.promise, "sidebar");
  const pages = vi.fn((args: Record<string, unknown> | undefined) => ({ generation: state.generation, root: "/a",
    archived: Boolean((args?.payload as { archived: boolean }).archived), rows: (args?.payload as { archived: boolean }).archived ? [archived] : [], next_cursor: null }));
  mockCommand("agent_session", pages, "sidebar_page");
  render(<App />);
  await flush();
  expect(screen.queryByText("或从左侧「会话」列表新建")).not.toBeInTheDocument();
  await act(async () => { startup.resolve(summary()); });
  await act(async () => { await vi.advanceTimersByTimeAsync(80); });
  expect(open).not.toHaveBeenCalled();
  expect(useAgentStore.getState().activeId).toBe(0);
  expect(screen.queryByTestId("switch-veil")).not.toBeInTheDocument();
  expect(screen.getByText("或从左侧「会话」列表新建")).toBeInTheDocument();
  expect(screen.queryByTestId("composer")).not.toBeInTheDocument();
  expect(screen.queryByTestId("chat-stream")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "已归档 1" }));
  await flush();
  expect(screen.getByText("session 9")).toBeInTheDocument();
  expect(pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ archived: true });
  mockCommand("agent_session", summary, "sidebar");
  fireEvent.click(screen.getAllByRole("button", { name: "新建会话" }).at(-1)!);
  await flush();
  await act(async () => { await vi.advanceTimersByTimeAsync(80); });
  expect(useAgentStore.getState().activeId).toBe(101);
  expect(screen.getByTestId("composer")).toBeInTheDocument();
  expect(screen.getByTestId("chat-stream")).toHaveAttribute("data-loading", "false");
  expect(open).not.toHaveBeenCalled(); // A late boot summary cannot reopen over new.
});

test("a failed startup summary is not mistaken for a successful no-active welcome", async () => {
  install();
  mockCommand("agent_session", () => Promise.reject(new Error("offline")), "sidebar");
  render(<App />);
  await flush();
  expect(screen.queryByText("或从左侧「会话」列表新建")).not.toBeInTheDocument();
  expect(screen.getByText("会话列表加载失败，重试")).toBeInTheDocument();
});

test("even a workspace-less welcome waits for a successful summary instead of flashing during boot", async () => {
  install();
  mockCommand("agent_session", () => ({ root: "", name: "", recents: [] }), "workspace_info");
  const startup = deferred<SidebarSummary>();
  mockCommand("agent_session", () => startup.promise, "sidebar");
  render(<App />);
  await flush();
  expect(screen.queryByRole("button", { name: "打开工作区…" })).not.toBeInTheDocument();
  await act(async () => { startup.resolve({ generation: "empty", projects: [], recents: [], active: null }); });
  expect(screen.getByRole("button", { name: "打开工作区…" })).toBeInTheDocument();
  expect(screen.queryByTestId("composer")).not.toBeInTheDocument();
});

test("a local active pointer is not replaced by an older successful no-active summary", async () => {
  install();
  useAgentStore.setState({ activeId: 101 });
  mockCommand("agent_session", () => ({ generation: "old", projects: [{ root: "/a", name: "a", current: true, last_opened: 0, total: 0, archived_total: 1 }], recents: [], active: null }), "sidebar");
  render(<App />);
  await flush();
  expect(useAgentStore.getState().activeId).toBe(101);
  expect(screen.queryByText("或从左侧「会话」列表新建")).not.toBeInTheDocument();
  expect(screen.getByTestId("composer")).toBeInTheDocument();
});

test("renaming a bounded sidebar title reads and saves the complete Unicode title on demand", async () => {
  const { summary } = install();
  const fullTitle = "中文🙂 long title ".repeat(100);
  const displayTitle = [...fullTitle].slice(0, 512).join("");
  mockCommand("agent_session", () => { const s = summary(); s.active!.row.title = displayTitle; return s; }, "sidebar");
  const title = vi.fn(() => ({ title: fullTitle }));
  const save = vi.fn(() => ({ renamed: true }));
  mockCommand("agent_session", title, "session_title");
  mockCommand("agent_session", save, "rename");
  render(<App />);
  await flush();
  expect(title).not.toHaveBeenCalled();
  const sidebarTitle = screen.getAllByText(displayTitle).find((el) => el.closest('[role="button"]'))!;
  fireEvent.contextMenu(sidebarTitle);
  await flush();
  fireEvent.click(screen.getByRole("menuitem", { name: "重命名" }));
  await flush();
  expect(title).toHaveBeenCalledWith(expect.objectContaining({ path: "/a", id: 2 }));
  expect(screen.getByRole("textbox")).toHaveValue(fullTitle);
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await flush();
  expect(save).toHaveBeenCalledWith(expect.objectContaining({ path: "/a", id: 2, payload: { title: fullTitle } }));
});
