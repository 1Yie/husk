import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { useAgentSession } from "../use-agent-session";
import { deferred, mockCommand, tauri } from "@/test/tauri";
import type { ProjectSessionRow, SidebarPage, SidebarSummary } from "@/types";
import { SessionSidebar } from "@/features/chat/components/session-sidebar";

type Args = Record<string, unknown> | undefined;
type PageArgs = { generation: string; archived: boolean; cursor?: string | null; limit: number };
const row = (id: number, archived = false): ProjectSessionRow => ({
  id, title: `row ${id}`, preview: "", updated_at: id, active: id === 1,
  running: false, pinned: false, pinned_global: id === 2, archived,
});
function fixture() {
  const state = {
    generation: "1", current: "/a", active: 1,
    rows: new Map([ ["/a", Array.from({ length: 110 }, (_, i) => row(i + 1, i >= 70))],
      ["/b", Array.from({ length: 40 }, (_, i) => row(i + 1))] ]),
  };
  const ordered = (root: string, archived: boolean) => state.rows.get(root)!.filter((r) => Boolean(r.archived) === archived)
    .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updated_at - a.updated_at || b.id - a.id);
  const summary = (): SidebarSummary => {
    const projects = [...state.rows].map(([root, rows]) => ({ root, name: root.slice(1), current: root === state.current,
      last_opened: 0, total: rows.filter((r) => !r.archived).length, archived_total: rows.filter((r) => r.archived).length }));
    const all = projects.flatMap((p) => state.rows.get(p.root)!.filter((r) => !r.archived)
      .map((r) => ({ root: p.root, project: p.name, current: p.current, row: { ...r, active: p.current && r.id === state.active } })))
      .sort((a, b) => b.row.updated_at - a.row.updated_at || a.root.localeCompare(b.root) || b.row.id - a.row.id);
    return { generation: state.generation, projects,
      recents: [...all.filter((e) => e.row.pinned_global), ...all.filter((e) => !e.row.pinned_global).slice(0, 6)],
      active: { root: state.current, project: state.current.slice(1), current: true,
        row: { ...state.rows.get(state.current)!.find((r) => r.id === state.active)!, active: true } } };
  };
  const page = (args: Args): SidebarPage => {
    const p = args!.payload as PageArgs;
    const root = args!.path as string;
    const offset = Number(p.cursor ?? 0);
    const rows = ordered(root, p.archived);
    return { generation: p.generation, root, archived: p.archived, rows: rows.slice(offset, offset + p.limit),
      next_cursor: offset + p.limit < rows.length ? String(offset + p.limit) : null };
  };
  const pages = vi.fn(page);
  const summaries = vi.fn(summary);
  mockCommand("agent_session", summaries, "sidebar");
  mockCommand("agent_session", pages, "sidebar_page");
  const updates = vi.fn((args: Args) => {
    const p = args!.payload as { generation: string; ids: number[] };
    const all = state.rows.get(args!.path as string) ?? [];
    return { generation: p.generation, rows: all.filter((r) => p.ids.includes(r.id)), missing: p.ids.filter((id) => !all.some((r) => r.id === id)) };
  });
  mockCommand("agent_session", updates, "sidebar_rows");
  return { rows: state.rows, state, page, pages, summaries, updates };
}
const flush = async () => { await act(async () => { await Promise.resolve(); }); };

test("a mounted expired first page recovers after a fresh summary even when the generation string stays the same", async () => {
  vi.useFakeTimers();
  const f = fixture();
  f.pages.mockImplementationOnce(() => { throw new Error("sidebar_snapshot_expired"); });
  const { result } = renderHook(() => useAgentSession());
  await flush();
  let stop!: () => void;
  act(() => { stop = result.current.paging.ensurePage("/a"); });
  await flush();
  expect(result.current.paging.get("/a").error).toContain("sidebar_snapshot_expired");
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "1", pages: 1 });
  expect(result.current.paging.get("/a").error).toBeUndefined();
  stop();
});

test("explicitly retrying a mounted first page reads only that page, not an extra unread page", async () => {
  vi.useFakeTimers();
  const f = fixture();
  f.pages.mockImplementationOnce(() => { throw new Error("offline"); });
  const { result } = renderHook(() => useAgentSession());
  await flush();
  let stop!: () => void;
  act(() => { stop = result.current.paging.ensurePage("/a"); });
  await flush();
  await act(async () => { await result.current.paging.retryPage("/a"); });
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(result.current.paging.get("/a")).toMatchObject({ pages: 1 });
  expect(result.current.paging.get("/a").error).toBeUndefined();
  stop();
});

test("explicit row-refresh recovery does not advance the already-read page budget", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.updates.mockImplementationOnce(() => { throw new Error("offline rows"); });
  await act(async () => { await result.current.refresh(); });
  f.state.generation = "2";
  await act(async () => { await result.current.paging.retryPage("/a"); });
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "2", pages: 1 });
  expect(result.current.paging.get("/a").error).toBeUndefined();
});

test("an unread-page error survives an expired rebase until that unread page is explicitly retried", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.pages.mockImplementationOnce(() => { throw new Error("offline unread page"); });
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.state.generation = "2";
  f.pages.mockImplementationOnce(() => {
    f.state.generation = "3";
    throw new Error("sidebar_snapshot_expired");
  });
  await act(async () => { await result.current.refresh(); });
  expect(result.current.paging.get("/a").generation).toBe("1");
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "3", pages: 1 });
  expect(result.current.paging.get("/a").error).toContain("offline unread page");
  const reads = f.pages.mock.calls.length;
  await act(async () => { await result.current.paging.loadMore("/a"); });
  expect(f.pages).toHaveBeenCalledTimes(reads);
  await act(async () => { await result.current.paging.retryPage("/a"); });
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "3", pages: 2 });
  expect(result.current.paging.get("/a").error).toBeUndefined();
});

function Harness() {
  const s = useAgentSession();
  return <SessionSidebar projects={s.projects} recents={s.recents} paging={s.paging}
    projectsLoaded={s.loaded} error={s.error} onRetry={() => void s.refresh({ force: true })}
    activeId={s.active?.row.id} version="test" onNew={() => {}} onOpenSession={() => {}} />;
}

test("boot uses the sidebar summary, never the full project list, and does not drain pages while idle", async () => {
  vi.useFakeTimers();
  const summary = vi.fn(() => ({ generation: "1", projects: [], recents: [], active: null }));
  mockCommand("agent_session", summary, "sidebar");
  const full = vi.fn(() => []);
  mockCommand("agent_session", full, "projects");
  const { result } = renderHook(() => useAgentSession());
  await act(async () => { await Promise.resolve(); });
  expect(summary).toHaveBeenCalledOnce();
  expect(full).not.toHaveBeenCalled();
  expect(result.current.loaded).toBe(true);
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(tauri.invoke.mock.calls.filter(([, args]) => args?.op === "sidebar_page")).toHaveLength(0);
});

test("independent pages are single-flight, stop at the end, and ordinary polling only refreshes loaded ids", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  expect(f.pages).not.toHaveBeenCalled();
  expect(result.current.sessions.find((r) => r.active)?.id).toBe(1);
  await act(async () => { await result.current.paging.loadMore("/a"); });
  expect(result.current.paging.get("/a").rows).toHaveLength(30);
  expect(result.current.projects[0].total).toBe(70);
  expect(result.current.recents.filter((e) => e.row.pinned_global)).toHaveLength(2);
  const pending = deferred<SidebarPage>();
  mockCommand("agent_session", (args) => (args!.payload as PageArgs).cursor ? pending.promise : f.page(args), "sidebar_page");
  let first!: Promise<void>;
  act(() => {
    first = result.current.paging.loadMore("/a");
    expect(result.current.paging.loadMore("/a")).toBe(first);
  });
  await act(async () => { pending.resolve(f.page({ path: "/a", payload: { generation: "1", archived: false, cursor: "30", limit: 30 } })); await first; });
  expect(result.current.paging.get("/a").rows).toHaveLength(60);
  mockCommand("agent_session", f.pages, "sidebar_page");
  await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
  expect(f.pages).toHaveBeenCalledTimes(1);
  expect(f.updates).toHaveBeenCalledTimes(4);
  expect(f.updates.mock.calls.every(([args]) => (args!.payload as { ids: number[] }).ids.length <= 30)).toBe(true);
  await act(async () => { await result.current.paging.loadMore("/a"); });
  expect(result.current.paging.get("/a").rows).toHaveLength(70);
  await act(async () => { await result.current.paging.loadMore("/a"); });
  expect(f.pages).toHaveBeenCalledTimes(2);
});

test("expansion and archive expansion read their own first pages; scrolling reaches the preceding project's tail", async () => {
  const f = fixture();
  render(<Harness />);
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(1);
  expect(f.pages.mock.calls[0][0]?.path).toBe("/a");
  expect(screen.getByText("70", { selector: "span" })).toBeInTheDocument();
  expect(f.pages.mock.calls.some(([a]) => (a!.payload as PageArgs).archived)).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "b 40" }));
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(2);
  const scroll = screen.getByTestId("sidebar-project-scroll");
  const rect = (top: number, bottom: number) => ({ top, bottom, left: 0, right: 200, width: 200, height: bottom - top, x: 0, y: top, toJSON() {} });
  vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue(rect(0, 500));
  const tails = scroll.querySelectorAll<HTMLElement>("[data-sidebar-tail]");
  expect(tails).toHaveLength(2);
  vi.spyOn(tails[0], "getBoundingClientRect").mockReturnValue(rect(650, 651));
  vi.spyOn(tails[1], "getBoundingClientRect").mockReturnValue(rect(680, 681));
  const pending = deferred<SidebarPage>();
  const next = vi.fn(() => pending.promise);
  mockCommand("agent_session", next, "sidebar_page");
  fireEvent.scroll(scroll); fireEvent.scroll(scroll);
  expect(next).toHaveBeenCalledOnce();
  const calls = tauri.invoke.mock.calls.filter(([, a]) => a?.op === "sidebar_page");
  expect(calls.at(-1)?.[1]?.path).toBe("/a");
  await act(async () => { pending.resolve(f.page({ path: "/a", payload: { generation: "1", archived: false, cursor: "30", limit: 30 } })); });
  expect(next).toHaveBeenCalledOnce();
  mockCommand("agent_session", f.pages, "sidebar_page");
  fireEvent.click(screen.getByRole("button", { name: "已归档 40" }));
  await flush();
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ archived: true, cursor: null, limit: 30 });
  expect(screen.getByText("row 110")).toBeInTheDocument();
  const liveTail = scroll.querySelector<HTMLElement>('[data-sidebar-tail][data-root="/a"][data-archived="false"]')!;
  vi.spyOn(liveTail, "getBoundingClientRect").mockReturnValue(rect(-100, -99));
  const archiveTail = scroll.querySelector<HTMLElement>('[data-sidebar-tail][data-root="/a"][data-archived="true"]')!;
  vi.spyOn(archiveTail, "getBoundingClientRect").mockReturnValue(rect(650, 651));
  vi.spyOn(tails[1], "getBoundingClientRect").mockReturnValue(rect(1000, 1001));
  fireEvent.scroll(scroll);
  await flush();
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ archived: true, cursor: "30" });
  const endCount = f.pages.mock.calls.length;
  fireEvent.scroll(scroll);
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(endCount);
  fireEvent.click(screen.getByRole("button", { name: "a 70" }));
  expect(screen.queryByText("row 110")).not.toBeInTheDocument();
  const count = f.pages.mock.calls.length;
  fireEvent.click(screen.getByRole("button", { name: "a 70" }));
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(count);
});

test("busy feedback waits 300ms and failed pages wait for an explicit retry", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const pending = deferred<SidebarPage>();
  const pages = vi.fn(() => pending.promise);
  mockCommand("agent_session", pages, "sidebar_page");
  render(<Harness />);
  await flush();
  expect(screen.queryByText("加载中…")).not.toBeInTheDocument();
  await act(async () => { await vi.advanceTimersByTimeAsync(299); });
  expect(screen.queryByText("加载中…")).not.toBeInTheDocument();
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(screen.getByText("加载中…")).toBeInTheDocument();
  await act(async () => { pending.reject(new Error("offline")); });
  expect(screen.queryByText("加载中…")).not.toBeInTheDocument();
  expect(screen.getByText("加载失败，重试")).toBeInTheDocument();
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  fireEvent.scroll(screen.getByTestId("sidebar-project-scroll"));
  expect(pages).toHaveBeenCalledOnce();
  mockCommand("agent_session", f.pages, "sidebar_page");
  fireEvent.click(screen.getByText("加载失败，重试"));
  await flush();
  expect(f.pages).toHaveBeenCalledOnce();
  expect(screen.queryByText("加载失败，重试")).not.toBeInTheDocument();
});

test("force refresh rebases only the read budget, keeps scrolled rows, applies tombstones/rename/pin/new and deduplicates", async () => {
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); await result.current.paging.loadMore("/a"); });
  expect(result.current.paging.get("/a").rows).toHaveLength(60);
  const oldLast = result.current.paging.get("/a").rows.at(-1)!.id;
  f.rows.set("/a", f.rows.get("/a")!.filter((r) => r.id !== 50).map((r) => r.id === 60 ? { ...r, title: "renamed" } : r.id === 3 ? { ...r, pinned: true } : r));
  f.rows.get("/a")!.push(row(200));
  f.state.generation = "2";
  const before = f.pages.mock.calls.length;
  await act(async () => { await result.current.refresh({ force: true }); });
  const loaded = result.current.paging.get("/a").rows;
  expect(loaded.some((r) => r.id === oldLast)).toBe(true);
  expect(loaded.some((r) => r.id === 50)).toBe(false);
  expect(loaded.find((r) => r.id === 60)?.title).toBe("renamed");
  expect(loaded[0].id).toBe(3);
  expect(loaded.some((r) => r.id === 200)).toBe(true);
  expect(new Set(loaded.map((r) => r.id)).size).toBe(loaded.length);
  expect(f.pages.mock.calls.length - before).toBe(2);
  expect(result.current.paging.get("/a").pages).toBe(2);
});

test.each(["deleted", "archived"])("an expired loaded non-live %s row recovers on the next generation within the read budget", async (change) => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); await result.current.paging.loadMore("/a"); });
  expect(result.current.paging.get("/a").rows.find((r) => r.id === 60)).toMatchObject({ active: false, running: false });
  f.rows.set("/a", f.rows.get("/a")!.flatMap((r) => r.id !== 60 ? [r] : change === "deleted" ? [] : [{ ...r, archived: true }]));
  // The real rows IPC rejects a dirty snapshot, rather than returning a
  // successful missing/tombstone response. Its next summary rebuilds it.
  f.updates.mockImplementationOnce(() => { f.state.generation = "2"; throw new Error("sidebar_snapshot_expired"); });
  const before = f.pages.mock.calls.length;
  await act(async () => { await result.current.refresh(); });
  expect(result.current.paging.get("/a").error).toContain("sidebar_snapshot_expired");
  expect(result.current.paging.get("/a").rows.some((r) => r.id === 60)).toBe(true);
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  const page = result.current.paging.get("/a");
  expect(page.error).toBeUndefined();
  expect(page.rows.some((r) => r.id === 60)).toBe(false);
  expect(page.pages).toBe(2);
  expect(page.generation).toBe("2");
  expect(f.pages.mock.calls.slice(before).map(([args]) => args!.payload)).toEqual([
    { generation: "2", archived: false, cursor: null, limit: 30 },
    { generation: "2", archived: false, cursor: "30", limit: 30 },
  ]);
  await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
  expect(f.pages).toHaveBeenCalledTimes(before + 2);
});

test("consecutive expired row refreshes have one automatic recovery even when summaries keep changing", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  const original = result.current.paging.get("/a").rows;
  const expired = vi.fn(() => { f.state.generation = String(Number(f.state.generation) + 1); throw new Error("sidebar_snapshot_expired"); });
  mockCommand("agent_session", expired, "sidebar_rows");
  await act(async () => { await result.current.refresh(); await vi.advanceTimersByTimeAsync(8000); });
  expect(expired).toHaveBeenCalledTimes(2);
  expect(result.current.paging.get("/a").rows).toEqual(original);
  expect(result.current.paging.get("/a").error).toContain("sidebar_snapshot_expired");
  mockCommand("agent_session", f.updates, "sidebar_rows");
  await act(async () => { await result.current.refresh({ force: true }); });
  expect(result.current.paging.get("/a").error).toBeUndefined();
});

test.each(["offline", "sidebar_unknown_project"])("a next-page %s error does not freeze loaded rows or automatically retry that page", async (failure) => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.pages.mockImplementationOnce(() => { throw new Error(failure); });
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.rows.get("/a")!.find((r) => r.id === 60)!.title = "updated despite page failure";
  await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
  expect(result.current.paging.get("/a").rows.find((r) => r.id === 60)?.title).toBe("updated despite page failure");
  expect(result.current.paging.get("/a").error).toContain(failure);
  expect(f.pages).toHaveBeenCalledTimes(2);
  f.state.generation = "2";
  await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
  expect(result.current.paging.get("/a").pages).toBe(1);
  expect(result.current.paging.get("/a").error).toContain(failure);
  expect(f.pages).toHaveBeenCalledTimes(3);
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ generation: "2", cursor: null });
  await act(async () => { await result.current.paging.loadMore("/a"); });
  expect(f.pages).toHaveBeenCalledTimes(3);
  await act(async () => { await result.current.paging.retryPage("/a"); });
  expect(result.current.paging.get("/a").error).toBeUndefined();
  expect(result.current.paging.get("/a").pages).toBe(2);
});

test("an expired next page rebases the loaded budget, not the failed unread page", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  f.pages.mockImplementationOnce(() => { f.state.generation = "2"; throw new Error("sidebar_snapshot_expired"); });
  await act(async () => { await result.current.paging.loadMore("/a"); });
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(result.current.paging.get("/a").error).toBeUndefined();
  expect(result.current.paging.get("/a").pages).toBe(1);
  expect(result.current.paging.get("/a").rows).toHaveLength(30);
  expect(f.pages).toHaveBeenCalledTimes(3);
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ generation: "2", cursor: null });
});

test("an expired rebase page keeps the old cursor owned by its generation until a bounded recovery succeeds", async () => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); await result.current.paging.loadMore("/a"); });
  f.state.generation = "2";
  f.pages.mockImplementationOnce(() => { f.state.generation = "3"; throw new Error("sidebar_snapshot_expired"); });
  await act(async () => { await result.current.refresh(); });
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "1", cursor: "60", pages: 2 });
  const before = f.pages.mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "3", pages: 2, error: undefined });
  expect(f.pages.mock.calls.slice(before).map(([args]) => (args!.payload as PageArgs).cursor)).toEqual([null, "30"]);
});

test.each([false, true])("an expired first page recovers only while its list is mounted (collapsed=%s)", async (collapse) => {
  vi.useFakeTimers();
  const f = fixture();
  f.pages.mockImplementationOnce(() => { f.state.generation = "2"; throw new Error("sidebar_snapshot_expired"); });
  render(<Harness />);
  await flush();
  expect(screen.getByText("加载失败，重试")).toBeInTheDocument();
  if (collapse) fireEvent.click(screen.getByRole("button", { name: "a 70" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(f.pages).toHaveBeenCalledTimes(collapse ? 1 : 2);
  if (collapse) {
    fireEvent.click(screen.getByRole("button", { name: "a 70" }));
    await flush();
  }
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(screen.queryByText("加载失败，重试")).not.toBeInTheDocument();
});

test("consecutive expired first pages stop after one automatic retry instead of looping through ensurePage", async () => {
  vi.useFakeTimers();
  const f = fixture();
  f.pages.mockImplementation(() => { f.state.generation = String(Number(f.state.generation) + 1); throw new Error("sidebar_snapshot_expired"); });
  render(<Harness />);
  await flush();
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(screen.getByText("加载失败，重试")).toBeInTheDocument();
});

test.each(["offline", "sidebar_unknown_project"])("a loaded-row %s error keeps cached sessions and waits for explicit retry", async (failure) => {
  vi.useFakeTimers();
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  const rows = result.current.paging.get("/a").rows;
  const failed = vi.fn(() => { throw new Error(failure); });
  mockCommand("agent_session", failed, "sidebar_rows");
  await act(async () => { await result.current.refresh(); });
  f.state.generation = "2";
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(failed).toHaveBeenCalledOnce();
  expect(result.current.paging.get("/a").rows).toEqual(rows);
  expect(result.current.projects[0].sessions.some((r) => r.id === 60)).toBe(true);
  expect(result.current.active?.row.id).toBe(1);
  mockCommand("agent_session", f.updates, "sidebar_rows");
  await act(async () => { await result.current.paging.retryPage("/a"); });
  expect(result.current.paging.get("/a").error).toBeUndefined();
});

test("a late old page/finally cannot overwrite a root switch or clear a replacement flight", async () => {
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  const old = deferred<SidebarPage>();
  mockCommand("agent_session", () => old.promise, "sidebar_page");
  let oldFlight!: Promise<void>;
  act(() => { oldFlight = result.current.paging.loadMore("/a"); });
  f.state.current = "/b"; f.state.generation = "2";
  await act(async () => { await result.current.refresh({ force: true }); });
  const fresh = deferred<SidebarPage>();
  const reads = vi.fn(() => fresh.promise);
  mockCommand("agent_session", reads, "sidebar_page");
  let newFlight!: Promise<void>;
  act(() => { newFlight = result.current.paging.loadMore("/a"); });
  await act(async () => { old.resolve(f.page({ path: "/a", payload: { generation: "1", archived: false, cursor: null, limit: 30 } })); await oldFlight; });
  expect(result.current.paging.get("/a").loading).toBe(true);
  expect(result.current.paging.loadMore("/a")).toBe(newFlight);
  expect(reads).toHaveBeenCalledOnce();
  expect(result.current.projects.find((p) => p.current)?.root).toBe("/b");
  expect(result.current.paging.get("/a").rows).toHaveLength(0);
  await act(async () => { fresh.resolve(f.page({ path: "/a", payload: { generation: "2", archived: false, cursor: null, limit: 30 } })); await newFlight; });
  expect(result.current.paging.get("/a").rows).toHaveLength(30);
  expect(result.current.projects.find((p) => p.root === "/b")?.sessions.find((r) => r.active)?.id).toBe(1);
});

test("late summaries do not replace a newer workspace or release its single-flight flag", async () => {
  const f = fixture();
  const stale = deferred<SidebarSummary>();
  mockCommand("agent_session", () => stale.promise, "sidebar");
  const { result } = renderHook(() => useAgentSession());
  const fresh = deferred<SidebarSummary>();
  const reads = vi.fn(() => fresh.promise);
  mockCommand("agent_session", reads, "sidebar");
  let pending!: Promise<unknown>;
  act(() => { pending = result.current.refresh({ force: true }); });
  await act(async () => { stale.resolve({ generation: "old", projects: [], recents: [], active: null }); });
  expect(result.current.loaded).toBe(false);
  expect(result.current.refresh()).toBe(pending);
  expect(reads).toHaveBeenCalledOnce();
  f.state.current = "/b"; f.state.generation = "2";
  const summary = f.summaries();
  await act(async () => { fresh.resolve(summary); await pending; });
  expect(result.current.projects.find((p) => p.current)?.root).toBe("/b");
});

test("a late expired row response cannot overwrite or release a newer rebase flight", async () => {
  const f = fixture();
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.paging.loadMore("/a"); });
  type Updates = { generation: string; rows: ProjectSessionRow[]; missing: number[] };
  const stale = deferred<Updates>();
  mockCommand("agent_session", () => stale.promise, "sidebar_rows");
  let oldFlight!: Promise<unknown>;
  act(() => { oldFlight = result.current.refresh(); });
  await flush();
  f.state.generation = "2";
  const fresh = deferred<Updates>();
  mockCommand("agent_session", () => fresh.promise, "sidebar_rows");
  let newFlight!: Promise<unknown>;
  act(() => { newFlight = result.current.refresh({ force: true }); });
  await flush();
  await act(async () => { stale.reject(new Error("sidebar_snapshot_expired")); await oldFlight; });
  expect(result.current.paging.get("/a").loading).toBe(true);
  expect(result.current.paging.get("/a").error).toBeUndefined();
  expect(result.current.refresh()).toBe(newFlight);
  await act(async () => {
    fresh.resolve({ generation: "2", rows: result.current.paging.get("/a").rows, missing: [] });
    await newFlight;
  });
  expect(result.current.paging.get("/a")).toMatchObject({ generation: "2", loading: false, pages: 1, error: undefined });
});

test.each([false, true])("an expired summary has one automatic retry, then requires explicit recovery (persistent=%s)", async (persistent) => {
  vi.useFakeTimers();
  const f = fixture();
  const expired = () => { throw new Error("sidebar_snapshot_expired"); };
  if (persistent) f.summaries.mockImplementation(expired);
  else f.summaries.mockImplementationOnce(expired);
  const { result } = renderHook(() => useAgentSession());
  await flush();
  expect(result.current.error).toContain("sidebar_snapshot_expired");
  await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
  expect(f.summaries).toHaveBeenCalledTimes(2);
  if (persistent) {
    await act(async () => { await vi.advanceTimersByTimeAsync(8000); });
    expect(f.summaries).toHaveBeenCalledTimes(2);
    expect(result.current.error).toContain("sidebar_snapshot_expired");
  } else {
    expect(result.current.error).toBeUndefined();
    expect(result.current.projects).toHaveLength(2);
  }
});

test("a failed summary ends the skeleton but does not silently retry every two seconds", async () => {
  vi.useFakeTimers();
  const failure = vi.fn(() => Promise.reject(new Error("offline")));
  mockCommand("agent_session", failure, "sidebar");
  const { result } = renderHook(() => useAgentSession());
  await flush();
  expect(result.current.loaded).toBe(true);
  expect(result.current.error).toContain("offline");
  await act(async () => { await vi.advanceTimersByTimeAsync(8000); });
  expect(failure).toHaveBeenCalledOnce();
  fixture();
  await act(async () => { await result.current.refresh({ force: true }); });
  expect(result.current.error).toBeUndefined();
  expect(result.current.projects).toHaveLength(2);
});

test("an idempotent boot open reuses the snapshot instead of forcing a second cold scan", async () => {
  const f = fixture();
  mockCommand("agent_session", () => ({ active: 1, history: [] }), "open");
  const { result } = renderHook(() => useAgentSession());
  await flush();
  await act(async () => { await result.current.openSession(1); });
  expect(f.summaries).toHaveBeenCalledTimes(2);
  const calls = tauri.invoke.mock.calls.filter(([, args]) => args?.op === "sidebar");
  expect(calls.at(-1)?.[1]?.payload).toEqual({ force: false });
  expect(f.pages).not.toHaveBeenCalled();
});

test("office mode uses the same bounded pages and keeps an archived older active row visible independently", async () => {
  const f = fixture();
  f.rows.get("/a")![0].archived = true;
  mockCommand("agent_session", () => {
    const summary = f.summaries();
    summary.projects[0].office = true;
    return summary;
  }, "sidebar");
  mockCommand("agent_session", () => [], "recent_artifacts");
  render(<Harness />);
  await flush();
  expect(f.pages).toHaveBeenCalledOnce();
  expect(f.pages.mock.calls[0][0]?.payload).toMatchObject({ archived: false, limit: 30 });
  expect(screen.getByTestId("sidebar-office-scroll")).toBeInTheDocument();
  expect(screen.queryByTestId("sidebar-project-scroll")).not.toBeInTheDocument();
  expect(screen.getByText("row 1")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "已归档 41" }));
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(2);
  expect(f.pages.mock.calls[1][0]?.payload).toMatchObject({ archived: true, limit: 30 });
  expect(screen.getAllByText("row 1")).toHaveLength(1);
});

const rect = (top: number, bottom: number) => ({ top, bottom, left: 0, right: 200, width: 200, height: bottom - top, x: 0, y: top, toJSON() {} });
function noOverflowGeometry(scroll: HTMLElement, height = 1200) {
  Object.defineProperties(scroll, { clientHeight: { configurable: true, value: height }, scrollHeight: { configurable: true, value: height } });
  vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue(rect(0, height));
  for (const tail of scroll.querySelectorAll<HTMLElement>("[data-sidebar-tail]")) {
    vi.spyOn(tail, "getBoundingClientRect").mockReturnValue(rect(height - 20, height - 19));
  }
}

test.each(["wheel", "PageDown", "End"])("a first-page error does not block the next project's no-overflow %s intent", async (intent) => {
  const f = fixture();
  f.pages.mockImplementation((args) => {
    if (args!.path === "/a") throw new Error("offline");
    return f.page(args);
  });
  render(<Harness />);
  await flush();
  fireEvent.click(screen.getByRole("button", { name: "b 40" }));
  await flush();
  expect(f.pages.mock.calls.filter(([args]) => args!.path === "/a")).toHaveLength(1);
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ cursor: null });
  const scroll = screen.getByTestId("sidebar-project-scroll");
  noOverflowGeometry(scroll);
  const pending = deferred<SidebarPage>();
  const next = vi.fn(() => pending.promise);
  mockCommand("agent_session", next, "sidebar_page");
  const sendIntent = () => intent === "wheel"
    ? fireEvent.wheel(scroll, { deltaY: 100, cancelable: true })
    : fireEvent.keyDown(scroll, { key: intent, cancelable: true });
  expect(sendIntent()).toBe(true); // Native defaults are never prevented.
  expect(sendIntent()).toBe(true);
  expect(next).toHaveBeenCalledOnce();
  expect(tauri.invoke.mock.calls.filter(([, args]) => args?.op === "sidebar_page").at(-1)?.[1]).toMatchObject({
    path: "/b", payload: { generation: "1", archived: false, cursor: "30", limit: 30 },
  });
  await act(async () => { pending.resolve(f.page({ path: "/b", payload: { generation: "1", archived: false, cursor: "30", limit: 30 } })); });
  expect(next).toHaveBeenCalledOnce();
  expect(screen.getByText("加载失败，重试")).toBeInTheDocument();
});

test.each([false, true])("a non-overflowing sidebar advances only on downward wheel/key intent (office=%s)", async (office) => {
  const f = fixture();
  if (office) {
    mockCommand("agent_session", () => { const summary = f.summaries(); summary.projects[0].office = true; return summary; }, "sidebar");
    mockCommand("agent_session", () => [], "recent_artifacts");
  }
  render(<Harness />);
  await flush();
  const scroll = screen.getByTestId(office ? "sidebar-office-scroll" : "sidebar-project-scroll");
  noOverflowGeometry(scroll);
  expect(f.pages).toHaveBeenCalledOnce();
  await flush();
  expect(f.pages).toHaveBeenCalledOnce(); // No mount/render/resize drain.
  fireEvent.wheel(scroll, { deltaY: -100 });
  fireEvent.keyDown(scroll, { key: "PageUp" });
  fireEvent.keyDown(scroll, { key: "Home" });
  expect(f.pages).toHaveBeenCalledOnce();
  const pending = deferred<SidebarPage>();
  const next = vi.fn(() => pending.promise);
  mockCommand("agent_session", next, "sidebar_page");
  // The intent is observed, never prevented; normal browser scrolling is intact.
  expect(fireEvent.wheel(scroll, { deltaY: 100, cancelable: true })).toBe(true);
  fireEvent.wheel(scroll, { deltaY: 100 });
  fireEvent.keyDown(scroll, { key: "PageDown" });
  fireEvent.keyDown(scroll, { key: "End" });
  expect(next).toHaveBeenCalledOnce();
  await act(async () => { pending.resolve(f.page({ path: "/a", payload: { generation: "1", archived: false, cursor: "30", limit: 30 } })); });
  expect(next).toHaveBeenCalledOnce();
  mockCommand("agent_session", f.pages, "sidebar_page");
  noOverflowGeometry(scroll);
  const beforeKey = f.pages.mock.calls.length;
  expect(fireEvent.keyDown(scroll, { key: "End", cancelable: true })).toBe(true);
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(beforeKey + 1);
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ archived: false, cursor: "60" });
  fireEvent.click(screen.getByRole("button", { name: "已归档 40" }));
  await flush();
  noOverflowGeometry(scroll);
  const beforeArchive = f.pages.mock.calls.length;
  fireEvent.keyDown(scroll, { key: "PageDown" });
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(beforeArchive + 1);
  expect(f.pages.mock.calls.at(-1)?.[0]?.payload).toMatchObject({ archived: true, cursor: "30" });
});

test("overflowing/hidden lists and editing/menu/activation events do not take the no-overflow shortcut", async () => {
  const f = fixture();
  render(<Harness />);
  await flush();
  const scroll = screen.getByTestId("sidebar-project-scroll");
  noOverflowGeometry(scroll);
  Object.defineProperty(scroll, "scrollHeight", { configurable: true, value: 3000 });
  fireEvent.wheel(scroll, { deltaY: 100 });
  fireEvent.keyDown(scroll, { key: "End" });
  expect(f.pages).toHaveBeenCalledOnce();
  Object.defineProperty(scroll, "scrollHeight", { configurable: true, value: 1200 });
  const input = document.createElement("input");
  const menu = document.createElement("div"); menu.setAttribute("role", "menu");
  const editable = document.createElement("div"); editable.setAttribute("contenteditable", "true");
  scroll.append(input, menu, editable);
  for (const target of [input, menu, editable]) {
    fireEvent.keyDown(target, { key: "PageDown" });
    fireEvent.wheel(target, { deltaY: 100 });
  }
  for (const key of ["Enter", " ", "ArrowDown", "PageUp"]) fireEvent.keyDown(scroll, { key });
  fireEvent.keyDown(scroll, { key: "End", ctrlKey: true });
  fireEvent.wheel(scroll, { deltaY: 100, ctrlKey: true });
  expect(f.pages).toHaveBeenCalledOnce();
  Object.defineProperty(scroll, "clientHeight", { configurable: true, value: 0 });
  fireEvent.wheel(scroll, { deltaY: 100 });
  expect(f.pages).toHaveBeenCalledOnce();
  Object.defineProperty(scroll, "clientHeight", { configurable: true, value: 1200 });
  for (const tail of scroll.querySelectorAll<HTMLElement>("[data-sidebar-tail]")) {
    vi.spyOn(tail, "getBoundingClientRect").mockReturnValue(rect(1500, 1501));
  }
  fireEvent.wheel(scroll, { deltaY: 100 });
  expect(f.pages).toHaveBeenCalledOnce();
  for (const tail of scroll.querySelectorAll<HTMLElement>("[data-sidebar-tail]")) {
    vi.spyOn(tail, "getBoundingClientRect").mockReturnValue(rect(1180, 1181));
  }
  Object.defineProperty(scroll, "scrollHeight", { configurable: true, value: 3000 });
  fireEvent.scroll(scroll);
  await flush();
  expect(f.pages).toHaveBeenCalledTimes(2); // Ordinary overflow retains onScroll.
});
