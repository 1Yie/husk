import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { historyRawPage, type RawHistoryPage } from "@/lib/agent-ipc/sessions";
import { readRawHistoryJson, useRawHistory } from "../use-raw-history";

vi.mock("@/lib/agent-ipc/sessions", () => ({ historyRawPage: vi.fn() }));
const readPage = vi.mocked(historyRawPage);

function page(start: number, end: number, total: number, revision = "revision-a"): RawHistoryPage {
  return {
    history: Array.from({ length: end - start }, (_, offset) => ({
      role: "user", content: String(start + offset),
    })),
    history_start: start, history_end: end, history_total: total,
    history_revision: revision, turn_total: total, turn_offset: start,
  };
}

beforeEach(() => vi.resetAllMocks());

describe("execution trace snapshots", () => {
  it("copies pages in chronological order and pins every later request to the first revision", async () => {
    readPage.mockResolvedValueOnce(page(2, 4, 4)).mockResolvedValueOnce(page(0, 2, 4));
    const progress = vi.fn();
    const json = await readRawHistoryJson(7, "/workspace", new AbortController().signal, progress);
    expect(JSON.parse(json).map((message: { content: string }) => message.content)).toEqual(["0", "1", "2", "3"]);
    expect(readPage.mock.calls).toEqual([
      [7, "/workspace", undefined, undefined], [7, "/workspace", 2, "revision-a"],
    ]);
    expect(progress.mock.calls).toEqual([[2, 4], [4, 4]]);
  });

  it("refuses a same-length rewrite during a complete copy", async () => {
    readPage.mockResolvedValueOnce(page(2, 4, 4)).mockResolvedValueOnce(page(0, 2, 4, "revision-b"));
    await expect(readRawHistoryJson(7, "/workspace", new AbortController().signal, vi.fn()))
      .rejects.toThrow("会话记录已变化");
  });

  it("propagates a stale snapshot error without copying a mixed result", async () => {
    readPage.mockResolvedValueOnce(page(2, 4, 4)).mockRejectedValueOnce(new Error("请刷新后重试"));
    await expect(readRawHistoryJson(7, "/workspace", new AbortController().signal, vi.fn()))
      .rejects.toThrow("请刷新后重试");
    expect(readPage).toHaveBeenCalledTimes(2);
  });

  it("does not start a cancelled copy", async () => {
    const abort = new AbortController();
    abort.abort();
    await expect(readRawHistoryJson(7, "/workspace", abort.signal, vi.fn())).rejects.toThrow();
    expect(readPage).not.toHaveBeenCalled();
  });

  it("keeps a rejected page visible and refreshes explicitly to a new revision", async () => {
    readPage.mockResolvedValueOnce(page(40, 80, 80))
      .mockRejectedValueOnce(new Error("会话记录已变化"))
      .mockResolvedValueOnce(page(40, 80, 80, "revision-b"));
    const { result } = renderHook(() => useRawHistory(true, 7, "/workspace"));
    await waitFor(() => expect(result.current.page?.history_revision).toBe("revision-a"));
    act(() => result.current.older());
    await waitFor(() => expect(result.current.error).toContain("会话记录已变化"));
    expect(result.current.page?.history_start).toBe(40);
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 40, "revision-a");
    act(() => result.current.latest());
    await waitFor(() => expect(result.current.page?.history_revision).toBe("revision-b"));
    expect(result.current.error).toBe("");
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", undefined, undefined);
  });

  it("ignores an old response after switching the active workspace/session", async () => {
    let resolveOld!: (value: RawHistoryPage) => void;
    readPage.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }))
      .mockResolvedValueOnce(page(0, 1, 1, "new-workspace"));
    const { result, rerender } = renderHook(
      ({ root, id }) => useRawHistory(true, id, root),
      { initialProps: { root: "/old", id: 7 } },
    );
    rerender({ root: "/new", id: 8 });
    await waitFor(() => expect(result.current.page?.history_revision).toBe("new-workspace"));
    await act(async () => resolveOld(page(0, 1, 1, "old-workspace")));
    expect(result.current.page?.history_revision).toBe("new-workspace");
  });
});
