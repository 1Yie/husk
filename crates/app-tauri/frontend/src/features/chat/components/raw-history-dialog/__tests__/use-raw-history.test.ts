import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { historyRawPage, type RawHistoryPage } from "@/lib/agent-ipc/sessions";
import { readRawHistoryJson, useRawHistory } from "../use-raw-history";

vi.mock("@/lib/agent-ipc/sessions", () => ({ historyRawPage: vi.fn() }));
const readPage = vi.mocked(historyRawPage);

/** A page whose prefix anchor covers everything but its last message. */
function page(start: number, end: number, total: number, anchor = "anchor-a"): RawHistoryPage {
  return {
    history: Array.from({ length: end - start }, (_, offset) => ({
      role: "user", content: String(start + offset),
    })),
    history_start: start, history_end: end, history_total: total,
    history_revision: "whole-a", history_anchor_len: Math.max(end - 1, 0), history_anchor: anchor,
    turn_total: total, turn_offset: start,
  };
}

beforeEach(() => vi.resetAllMocks());

describe("execution trace snapshots", () => {
  it("copies pages in chronological order and chains each request to the previous page's anchor", async () => {
    readPage.mockResolvedValueOnce(page(2, 4, 4)).mockResolvedValueOnce(page(0, 2, 4, "anchor-b"));
    const progress = vi.fn();
    const json = await readRawHistoryJson(7, "/workspace", new AbortController().signal, progress);
    expect(JSON.parse(json).map((message: { content: string }) => message.content)).toEqual(["0", "1", "2", "3"]);
    expect(readPage.mock.calls).toEqual([
      [7, "/workspace", undefined, undefined], [7, "/workspace", 2, { len: 3, revision: "anchor-a" }],
    ]);
    expect(progress.mock.calls).toEqual([[2, 4], [4, 4]]);
  });

  it("copies the total fixed by the first page when the run appends during the copy", async () => {
    readPage.mockResolvedValueOnce(page(3, 5, 5)).mockResolvedValueOnce(page(1, 3, 6))
      .mockResolvedValueOnce(page(0, 1, 6));
    const json = await readRawHistoryJson(7, "/workspace", new AbortController().signal, vi.fn());
    expect(JSON.parse(json).map((message: { content: string }) => message.content)).toEqual(["0", "1", "2", "3", "4"]);
  });

  it("refuses a page whose window does not line up during a complete copy", async () => {
    readPage.mockResolvedValueOnce(page(2, 4, 4)).mockResolvedValueOnce(page(0, 1, 4));
    await expect(readRawHistoryJson(7, "/workspace", new AbortController().signal, vi.fn()))
      .rejects.toThrow("会话记录已变化");
  });

  it("propagates a stale page error without copying a mixed result", async () => {
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

  it("keeps a rejected page visible and retries with the same window and anchor", async () => {
    readPage.mockResolvedValueOnce(page(40, 80, 80))
      .mockRejectedValueOnce(new Error("会话历史已被其他窗口更新"))
      .mockResolvedValueOnce(page(0, 40, 80, "anchor-b"));
    const { result } = renderHook(() => useRawHistory(true, 7, "/workspace"));
    await waitFor(() => expect(result.current.page?.history_start).toBe(40));
    act(() => result.current.older());
    await waitFor(() => expect(result.current.error).toContain("会话历史已被其他窗口更新"));
    expect(result.current.page?.history_start).toBe(40);
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 40, { len: 79, revision: "anchor-a" });
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.page?.history_start).toBe(0));
    expect(result.current.error).toBe("");
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 40, { len: 79, revision: "anchor-a" });
  });

  it("keeps paging older and newer while the run appends messages", async () => {
    readPage.mockResolvedValueOnce(page(40, 80, 80))
      .mockResolvedValueOnce(page(0, 40, 85))
      .mockResolvedValueOnce(page(40, 80, 85))
      .mockResolvedValueOnce(page(80, 85, 85));
    const { result } = renderHook(() => useRawHistory(true, 7, "/workspace"));
    await waitFor(() => expect(result.current.page?.history_start).toBe(40));
    act(() => result.current.older());
    await waitFor(() => expect(result.current.page?.history_start).toBe(0));
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 40, { len: 79, revision: "anchor-a" });
    expect(result.current.page?.history_total).toBe(85);
    act(() => result.current.newer());
    await waitFor(() => expect(result.current.page?.history_end).toBe(80));
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 80, { len: 39, revision: "anchor-a" });
    act(() => result.current.newer());
    await waitFor(() => expect(result.current.page?.history_end).toBe(85));
    expect(readPage).toHaveBeenLastCalledWith(7, "/workspace", 85, { len: 79, revision: "anchor-a" });
    expect(result.current.error).toBe("");
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
    await waitFor(() => expect(result.current.page?.history_anchor).toBe("new-workspace"));
    await act(async () => resolveOld(page(0, 1, 1, "old-workspace")));
    expect(result.current.page?.history_anchor).toBe("new-workspace");
  });
});
