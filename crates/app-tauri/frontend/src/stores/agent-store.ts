// agent-store — the kernel event stream plus the pointers it drives.
//
// This was `useAgentEvents`, a hook only App called: every ~60 fps batch
// replaced the views map, so App re-rendered (and dragged its subtree with it)
// for the whole duration of a turn. The stream belongs to no component — the
// pump is module-level here, installed once (`initAgentStore`), and consumers
// subscribe to the slice they actually read.
//
// Views are keyed `${workspaceRoot}:${sessionId}`: a parked workspace's actors
// keep streaming after a switch, so the envelope's root routes each event to the
// right buffer.

import { create } from "zustand";
import * as agent from "@/lib/agent-ipc";
import type { AgentEventEnvelope } from "@/types";
import { applyEvent } from "@/features/chat/hooks/apply-event";
import { emptyView, type SessionView, type StreamItem } from "@/features/chat/hooks/stream-view";
import { notify } from "@/lib/notifications";

/** Composite view key — workspace root + per-workspace session id. Internal:
 *  callers go through the selectors below, which own that lookup. */
const viewKey = (root: string, id: number) => `${root}:${id}`;

/** Stable "no view yet" handle: a selector must not mint a fresh object. */
const EMPTY_VIEW: SessionView = emptyView();
const EMPTY_KEYS: ReadonlySet<string> = new Set();

/** Running-session keys. `AgentState::is_active` mirror — everything except
 *  Idle/Finished/Failed; a parked workspace's turn counts too, so its sidebar
 *  row keeps the orb while it streams in the background. */
function runningKeysOf(views: Map<string, SessionView>): Set<string> {
  const out = new Set<string>();
  for (const [k, v] of views) {
    const s = v.state;
    if (s !== null && s !== "Idle" && s !== "Finished" && !(typeof s === "object" && "Failed" in s)) {
      out.add(k);
    }
  }
  return out;
}

function sameKeys(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const k of a) if (!b.has(k)) return false;
  return true;
}

interface AgentStore {
  views: Map<string, SessionView>;
  /** Kept in sync with `views` so its IDENTITY is stable while the same
   *  sessions are running — a selector returning a fresh Set re-renders on
   *  every batch. */
  runningKeys: ReadonlySet<string>;
  /** Foreground session + the root it lives in. Set by the app shell. */
  workspaceRoot: string;
  activeId: number;
  // Header chips.
  gitInfo: agent.GitInfo | null;
  ctxWindow: number;
  modelCost: agent.ModelItem["cost"];

  setWorkspaceRoot: (root: string) => void;
  setActiveId: (id: number) => void;
  /** Install a rebuilt view for `id` — used when a session has no live buffer.
   *  Never overwrites an existing one: a live buffer is newer than the last
   *  persisted snapshot. */
  loadView: (
    root: string,
    id: number,
    view: SessionView,
    meta?: { historyStart?: number; historyTotal?: number; turnTotal?: number; turnOffset?: number }
  ) => void;
  /** Prepend an older-history page — `items` are folded StreamItems already
   *  stamped with global `hi`s. Idempotent: a stale `before` resolves to a page
   *  whose `historyStart` is no longer older than the loaded window's, and
   *  prepending it again would duplicate items AND their `hi` stamps →
   *  duplicate turn/mark ids → several rail marks matching `activeId`. */
  prependItems: (
    root: string,
    id: number,
    items: StreamItem[],
    historyStart: number,
    turnOffset?: number
  ) => void;
}

export const useAgentStore = create<AgentStore>((set) => ({
  views: new Map(),
  runningKeys: EMPTY_KEYS,
  workspaceRoot: "",
  activeId: 0,
  gitInfo: null,
  ctxWindow: 0,
  modelCost: undefined,

  setWorkspaceRoot: (root) => set({ workspaceRoot: root }),
  setActiveId: (id) => set({ activeId: id }),

  loadView: (root, id, view, meta) =>
    set((s) => {
      const k = viewKey(root, id);
      if (s.views.has(k)) return s;
      const next = new Map(s.views);
      next.set(k, meta ? { ...view, ...meta } : view);
      const keys = runningKeysOf(next);
      return { views: next, runningKeys: sameKeys(s.runningKeys, keys) ? s.runningKeys : keys };
    }),

  prependItems: (root, id, items, historyStart, turnOffset) =>
    set((s) => {
      const k = viewKey(root, id);
      const v = s.views.get(k);
      if (!v) return s;
      if (v.historyStart != null && historyStart >= v.historyStart) return s;
      const next = new Map(s.views);
      next.set(k, { ...v, items: [...items, ...v.items], historyStart, turnOffset: turnOffset ?? v.turnOffset });
      return { views: next };
    }),
}));

/** The foreground session's view — a stable reference until that session's own
 *  buffer changes. */
export function useActiveView(): SessionView {
  return useAgentStore(
    (s) => s.views.get(viewKey(s.workspaceRoot, s.activeId)) ?? EMPTY_VIEW
  );
}

/** Whether the foreground session has older history to pull. A boolean, so a
 *  consumer re-renders when it flips, not on every delta. */
export function useHasMoreHistory(): boolean {
  return useAgentStore(
    (s) => (s.views.get(viewKey(s.workspaceRoot, s.activeId))?.historyStart ?? 0) > 0
  );
}

export function useRunningKeys(): ReadonlySet<string> {
  return useAgentStore((s) => s.runningKeys);
}

/** Header chips: git state, the active model's window size and its $/1M. */
export function useAgentChips(): {
  gitInfo: agent.GitInfo | null;
  ctxWindow: number;
  modelCost: agent.ModelItem["cost"];
} {
  const gitInfo = useAgentStore((s) => s.gitInfo);
  const ctxWindow = useAgentStore((s) => s.ctxWindow);
  const modelCost = useAgentStore((s) => s.modelCost);
  return { gitInfo, ctxWindow, modelCost };
}

/** Read the foreground view outside React (callbacks that run after a render
 *  they captured — the stale-closure case a ref used to guard). */
export function activeViewNow(): SessionView {
  const s = useAgentStore.getState();
  return s.views.get(viewKey(s.workspaceRoot, s.activeId)) ?? EMPTY_VIEW;
}

// ---------------------------------------------------------------- the pump

/** Git + model chips: refreshed on session switch and at every streaming edge
 *  (a turn is the main thing that dirties the tree and may switch the model). */
function refreshAgentChips() {
  void agent
    .getGitInfo()
    .then((g) => useAgentStore.setState({ gitInfo: g }))
    .catch(() => {});
  void agent
    .getModelInfo()
    .then((info) => {
      if (!info) return;
      const cur =
        info.models.find((m) => m.model === info.active_model && m.provider === info.active_provider) ??
        info.models.find((m) => m.model === info.active_model);
      useAgentStore.setState((s) => ({
        ctxWindow: cur?.context_window ?? s.ctxWindow,
        modelCost: cur?.cost,
      }));
    })
    .catch(() => {});
}

let installed = false;

/** Install the kernel subscription and the chip refreshes. Idempotent — call
 *  once from the app shell, before React mounts. */
export function initAgentStore() {
  if (installed) return;
  installed = true;

  // Coalesce the kernel stream into ~60 fps batches: the engine already merges
  // deltas (~120 chars/event), but turn-end flushes and tool transitions land
  // as bursts, and one commit per frame beats one per event.
  const queue: AgentEventEnvelope[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  const flush = () => {
    timer = null;
    if (queue.length === 0) return;
    const batch = queue.splice(0, queue.length);
    const s = useAgentStore.getState();
    const next = new Map(s.views);
    for (const { session, event, root } of batch) {
      const k = viewKey(root ?? "", session);
      next.set(k, applyEvent(next.get(k) ?? emptyView(), event));
    }
    // NOTE: `activeId` is deliberately NOT driven by events — otherwise a
    // background session's traffic yanks the visible session, and the
    // frontend's pointer diverges from the backend's `active_id`.
    useAgentStore.setState({ views: next, runningKeys: (() => {
      const keys = runningKeysOf(next);
      return sameKeys(s.runningKeys, keys) ? s.runningKeys : keys;
    })() });
  };

  void agent.onAgentEvent((env) => {
    queue.push(env);
    const ev = env.event;
    if (typeof ev === "object" && "StateChanged" in ev) {
      const st = ev.StateChanged;
      const s = useAgentStore.getState();
      const key = viewKey(env.root ?? "", env.session);
      if (key !== viewKey(s.workspaceRoot, s.activeId)) {
        if (st === "Finished") {
          notify({ kind: "turn", title: "会话已完成", root: env.root ?? "", session: env.session });
        } else if (typeof st === "object" && "Failed" in st) {
          notify({ kind: "turn", title: "会话出错", body: st.Failed, root: env.root ?? "", session: env.session });
        }
      }
    }
    if (timer == null) timer = setTimeout(flush, 16);
  });

  // Chips follow the foreground session and its streaming edges. Watched here
  // rather than in an effect so a remount cannot drop the wiring.
  let chipKey = "";
  const syncChips = () => {
    const s = useAgentStore.getState();
    const streaming = s.views.get(viewKey(s.workspaceRoot, s.activeId))?.streaming ?? false;
    const key = `${s.workspaceRoot}:${s.activeId}:${streaming}`;
    if (key !== chipKey) {
      chipKey = key;
      refreshAgentChips();
    }
  };
  useAgentStore.subscribe(syncChips);
  syncChips();

  // A settings save broadcasts CONFIG_CHANGED_EVENT — re-read so a renamed /
  // repriced / re-windowed model shows immediately.
  window.addEventListener(agent.CONFIG_CHANGED_EVENT, refreshAgentChips);
}
