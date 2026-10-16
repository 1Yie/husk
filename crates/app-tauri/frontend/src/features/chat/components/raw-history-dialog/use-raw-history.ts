import { useCallback, useEffect, useRef, useState } from "react";
import { historyRawPage, type RawHistoryPage } from "@/lib/agent-ipc/sessions";

export const RAW_PAGE_SIZE = 40;

export function useRawHistory(open: boolean, sessionId: number | null, root: string) {
  const key = `${root}:${sessionId}`;
  const request = useRef(0);
  const inFlight = useRef(false);
  const cursor = useRef<number | undefined>();
  const snapshotTotal = useRef<number | null>(null);
  const [result, setResult] = useState<{ key: string; page: RawHistoryPage } | null>(null);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const [pending, setPending] = useState(false);

  const load = useCallback(async (before?: number) => {
    if (!open || sessionId === null || inFlight.current) return;
    const token = ++request.current;
    inFlight.current = true;
    cursor.current = before;
    setPending(true);
    setFailure(null);
    try {
      const next = await historyRawPage(sessionId, root, before);
      if (request.current !== token) return;
      // Keep page boundaries stable when a running session appends new rows.
      // Refresh explicitly to include rows written after this dialog opened.
      const total = before === undefined ? next.history_total : snapshotTotal.current ?? next.history_total;
      if (next.history_total < total || next.history_end !== (before ?? total)
        || next.history_start + next.history.length !== next.history_end) {
        throw new Error("会话记录已变化，请刷新执行轨迹");
      }
      snapshotTotal.current = total;
      setResult({ key, page: { ...next, history_total: total } });
    } catch (e) {
      if (request.current === token) setFailure({ key, message: String(e) });
    } finally {
      if (request.current === token) {
        inFlight.current = false;
        setPending(false);
      }
    }
  }, [open, sessionId, root, key]);

  useEffect(() => {
    setResult(null);
    setFailure(null);
    setPending(false);
    inFlight.current = false;
    snapshotTotal.current = null;
    if (open && sessionId !== null) void load();
    return () => { request.current += 1; inFlight.current = false; };
  }, [open, sessionId, root, load]);

  const page = open && result?.key === key ? result.page : null;
  const error = open && failure?.key === key ? failure.message : "";
  const loading = open && sessionId !== null && (pending || (!page && !error));
  return {
    page, error, loading,
    older: () => { if (page && page.history_start > 0 && !loading) void load(page.history_start); },
    newer: () => {
      if (page && page.history_end < page.history_total && !loading) {
        void load(Math.min(page.history_end + RAW_PAGE_SIZE, page.history_total));
      }
    },
    latest: () => { if (!loading) void load(); },
    retry: () => { if (!loading) void load(cursor.current); },
  };
}

/** Fetch and serialize the full history only for an explicit copy action.
 * Yield between pages and keep them out of the rendered dialog. */
export async function readRawHistoryJson(
  sessionId: number,
  root: string,
  signal: AbortSignal,
  onProgress: (read: number, total: number) => void,
): Promise<string> {
  const chunks: string[] = [];
  let before: number | undefined;
  let total: number | undefined;
  let read = 0;
  do {
    signal.throwIfAborted();
    const page = await historyRawPage(sessionId, root, before);
    signal.throwIfAborted();
    total ??= page.history_total;
    if (page.history_total < total || page.history_end !== (before ?? total)
      || page.history_start + page.history.length !== page.history_end
      || (page.history_start > 0 && page.history.length === 0)) {
      throw new Error("会话记录已变化，请重新复制");
    }
    if (page.history.length) chunks.push(page.history.map((m) => JSON.stringify(m, null, 2)).join(",\n"));
    read += page.history.length;
    onProgress(read, total);
    before = page.history_start;
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  } while (before > 0);
  signal.throwIfAborted();
  return chunks.length ? `[\n${chunks.reverse().join(",\n")}\n]` : "[]";
}
