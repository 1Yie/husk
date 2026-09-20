// useAgentEvents — kernel event stream → per-session `SessionView` map.
// Owns the ~60fps event batching, the active-session pointer, and the
// header git/model chips that refresh on session/turn edges.
//
// Views are keyed `${workspaceRoot}:${sessionId}` — session ids are
// per-workspace, and a parked workspace's actors keep streaming after a
// switch ("switching never kills the turn"), so the root in each event
// envelope is what routes it to the right buffer.

import { useCallback, useEffect, useState } from "react";
import * as agent from "../invoke/agent";
import type { AgentEventEnvelope } from "../types";
import { applyEvent } from "./apply-event";
import { emptyView, type SessionView } from "./stream-view";

/** Composite view key — workspace root + per-workspace session id. */
export const viewKey = (root: string, id: number) => `${root}:${id}`;

export function useAgentEvents(workspaceRoot: string) {
  const [views, setViews] = useState<Map<string, SessionView>>(new Map());
  const [activeId, setActiveId] = useState(0);
  const [gitInfo, setGitInfo] = useState<agent.GitInfo | null>(null);
  // Context-window size of the active model — lets the header meter show
  // the real denominator before the first `Usage` event of a session.
  const [ctxWindow, setCtxWindow] = useState(0);

  /** Install a rebuilt view for `id` — used by `openSession` when the
   * session has no live event buffer. Never overwrites an existing view:
   * a live buffer is newer than the last persisted snapshot. */
  const loadView = useCallback((root: string, id: number, view: SessionView) => {
    setViews((m) => {
      const k = viewKey(root, id);
      if (m.has(k)) return m;
      const next = new Map(m);
      next.set(k, view);
      return next;
    });
  }, []);

  useEffect(() => {
    // Coalesce the kernel event stream into ~60fps batches — the engine
    // already merges deltas (~120 chars/event), but turn-end flushes and
    // tool transitions still land as bursts; folding them into ONE
    // `setViews` renders once per frame instead of once per event.
    const queue: AgentEventEnvelope[] = [];
    let timer: ReturnType<typeof setTimeout> | null = null;
    const flush = () => {
      timer = null;
      if (queue.length === 0) return;
      const batch = queue.splice(0, queue.length);
      setViews((m) => {
        const next = new Map(m);
        for (const { session, event, root } of batch) {
          const k = viewKey(root ?? "", session);
          next.set(k, applyEvent(next.get(k) ?? emptyView(), event));
        }
        return next;
      });
      // NOTE: `activeId` is deliberately NOT driven by events. The old
      // `setActiveId(batch.last.session)` let any background session's
      // traffic yank the visible session — switching away from a running
      // turn snapped the view straight back, and worse, the frontend's
      // pointer diverged from the backend's `active_id`, so a prompt
      // typed into one session's stream was routed to a different actor.
      // The visible session only changes through user ops (open/new/
      // delete/fork/workspace switch), which keep both sides in sync.
    };
    const un = agent.onAgentEvent((env) => {
      queue.push(env);
      if (timer == null) timer = setTimeout(flush, 16);
    });
    return () => {
      if (timer != null) clearTimeout(timer);
      un.then((f) => f());
    };
  }, []);

  const active = views.get(viewKey(workspaceRoot, activeId)) ?? emptyView();

  // Git chip in the header — refreshed on session switch and at every
  // streaming edge, since a turn is the main thing that dirties the tree.
  // Same cadence for the model's context window (model may switch between
  // turns via the composer picker).
  useEffect(() => {
    let dead = false;
    void agent
      .getGitInfo()
      .then((g) => {
        if (!dead) setGitInfo(g);
      })
      .catch(() => {});
    void agent
      .getModelInfo()
      .then((info) => {
        if (dead || !info) return;
        const cur =
          info.models.find(
            (m) => m.model === info.active_model && m.provider === info.active_provider
          ) ?? info.models.find((m) => m.model === info.active_model);
        const cw = cur?.context_window;
        if (!dead && cw) setCtxWindow(cw);
      })
      .catch(() => {});
    return () => {
      dead = true;
    };
  }, [activeId, active.streaming]);

  // Sidebar running flags — derived live from each session's last
  // `StateChanged`, so the orb reacts on the event itself instead of
  // waiting for the next `listSessions` refresh. Mirrors
  // `AgentState::is_active`: everything except Idle/Finished/Failed.
  // Keys are `root:id` — a parked workspace's turn counts too, so its
  // sidebar row keeps the orb while it streams in the background.
  const runningKeys = new Set<string>();
  for (const [k, v] of views) {
    const s = v.state;
    if (
      s !== null &&
      s !== "Idle" &&
      s !== "Finished" &&
      !(typeof s === "object" && "Failed" in s)
    ) {
      runningKeys.add(k);
    }
  }

  return {
    views,
    activeId,
    active,
    setActiveId,
    loadView,
    runningKeys,
    gitInfo,
    ctxWindow,
  };
}
