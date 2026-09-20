// useAgentEvents — kernel event stream → per-session `SessionView` map.
// Owns the ~60fps event batching, the active-session pointer, and the
// header git/model chips that refresh on session/turn edges.

import { useCallback, useEffect, useState } from "react";
import * as agent from "../invoke/agent";
import type { AgentEventEnvelope } from "../types";
import { applyEvent } from "./apply-event";
import { emptyView, type SessionView } from "./stream-view";

export function useAgentEvents() {
  const [views, setViews] = useState<Map<number, SessionView>>(new Map());
  const [activeId, setActiveId] = useState(0);
  const [gitInfo, setGitInfo] = useState<agent.GitInfo | null>(null);
  // Context-window size of the active model — lets the header meter show
  // the real denominator before the first `Usage` event of a session.
  const [ctxWindow, setCtxWindow] = useState(0);

  /** Install a rebuilt view for `id` — used by `openSession` when the
   * session has no live event buffer. Never overwrites an existing view:
   * a live buffer is newer than the last persisted snapshot. */
  const loadView = useCallback((id: number, view: SessionView) => {
    setViews((m) => {
      if (m.has(id)) return m;
      const next = new Map(m);
      next.set(id, view);
      return next;
    });
  }, []);

  /** Drop every cached stream view — called whenever the active workspace
   * changes. Session ids are per-workspace, so keeping the map would let an
   * id that merely collides in the new workspace keep showing the previous
   * project's conversation (`loadView` never overwrites an existing key). */
  const clearViews = useCallback(() => setViews(new Map()), []);

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
        for (const { session, event } of batch) {
          next.set(session, applyEvent(next.get(session) ?? emptyView(), event));
        }
        return next;
      });
      setActiveId(batch[batch.length - 1].session);
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

  const active = views.get(activeId) ?? emptyView();

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
  const runningIds = new Set<number>();
  for (const [id, v] of views) {
    const s = v.state;
    if (
      s !== null &&
      s !== "Idle" &&
      s !== "Finished" &&
      !(typeof s === "object" && "Failed" in s)
    ) {
      runningIds.add(id);
    }
  }

  return {
    views,
    activeId,
    active,
    setActiveId,
    loadView,
    clearViews,
    runningIds,
    gitInfo,
    ctxWindow,
  };
}
