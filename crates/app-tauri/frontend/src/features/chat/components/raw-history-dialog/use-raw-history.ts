import { useCallback, useEffect, useRef, useState } from "react";
import { historyRawPage, type RawHistoryPage } from "@/lib/agent-ipc/sessions";

export const RAW_PAGE_SIZE = 40;

type Anchor = { len: number; revision: string };

/** The prefix a page was sliced under. Appends leave it valid, so the next
 * page can still be requested against it while a running turn grows. */
function anchorOf(page: RawHistoryPage): Anchor {
  return { len: page.history_anchor_len, revision: page.history_anchor };
}

export function useRawHistory(open: boolean, sessionId: number | null, root: string) {
  const key = `${root}:${sessionId}`;
  const request = useRef(0);
  const inFlight = useRef(false);
  const cursor = useRef<{ before?: number; anchor?: Anchor }>({});
  const [result, setResult] = useState<{ key: string; page: RawHistoryPage } | null>(null);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const [pending, setPending] = useState(false);

  const load = useCallback(async (before?: number, anchor?: Anchor) => {
    if (!open || sessionId === null || inFlight.current) return;
    const token = ++request.current;
    inFlight.current = true;
    cursor.current = { before, anchor };
    setPending(true);
    setFailure(null);
    try {
      const next = await historyRawPage(sessionId, root, before, anchor);
      if (request.current !== token) return;
      // The backend rejects a rewritten prefix. The total is live: appends
      // from a running turn are expected, so only the window is checked.
      const end = Math.min(before ?? next.history_total, next.history_total);
      if (next.history_end !== end
        || next.history_start + next.history.length !== next.history_end) {
        throw new Error("会话记录已变化，请刷新执行轨迹");
      }
      setResult({ key, page: next });
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
    if (open && sessionId !== null) void load();
    return () => { request.current += 1; inFlight.current = false; };
  }, [open, sessionId, root, load]);

  const page = open && result?.key === key ? result.page : null;
  const error = open && failure?.key === key ? failure.message : "";
  const loading = open && sessionId !== null && (pending || (!page && !error));
  return {
    page, error, loading,
    older: () => { if (page && page.history_start > 0 && !loading) void load(page.history_start, anchorOf(page)); },
    newer: () => {
      if (page && page.history_end < page.history_total && !loading) {
        void load(Math.min(page.history_end + RAW_PAGE_SIZE, page.history_total), anchorOf(page));
      }
    },
    latest: () => { if (!loading) void load(); },
    retry: () => { if (!loading) void load(cursor.current.before, cursor.current.anchor); },
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
  let anchor: Anchor | undefined;
  let read = 0;
  do {
    signal.throwIfAborted();
    const page = await historyRawPage(sessionId, root, before, anchor);
    signal.throwIfAborted();
    // The first page fixes the copy's bound; appends after it are not copied.
    total ??= page.history_total;
    if (page.history_end !== (before ?? total)
      || page.history_start + page.history.length !== page.history_end
      || (page.history_start > 0 && page.history.length === 0)) {
      throw new Error("会话记录已变化，请重新复制");
    }
    if (page.history.length) chunks.push(page.history.map((m) => JSON.stringify(m, null, 2)).join(",\n"));
    read += page.history.length;
    onProgress(read, total);
    anchor = anchorOf(page);
    before = page.history_start;
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  } while (before > 0);
  signal.throwIfAborted();
  return chunks.length ? `[\n${chunks.reverse().join(",\n")}\n]` : "[]";
}
