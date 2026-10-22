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
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { hasRunningChildren } from "@/features/chat/hooks/running-children";
import {
  emptyView,
  type BgJob,
  type ChildSegment,
  type SessionView,
  type StreamItem,
} from "@/features/chat/hooks/stream-view";
import { markSessionRead, notifyForEvent } from "@/lib/notifications";
import { isAppInFront, onAppFront } from "@/lib/desktop-notify";

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
   *  An existing live buffer keeps its rows/state; only missing meter and
   *  pagination metadata are seeded from the persisted snapshot. */
  loadView: (
    root: string,
    id: number,
    view: SessionView,
    meta?: {
      historyStart?: number;
      historyTotal?: number;
      turnTotal?: number;
      turnOffset?: number;
      runTotal?: number;
      taskTotal?: number;
    }
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
  /** Drop a session's cached view — session ids are never reused after
   *  deletion, but the map would otherwise keep dead buffers forever, and
   *  `loadView`'s no-overwrite rule would resurrect one if an id ever did
   *  collide again. */
  dropView: (root: string, id: number) => void;
  /** Fold a finished subagent's own history onto its card: the body segments,
   *  and its tool rows spliced in right behind the card so the stream's fold
   *  nests them under it. A missing card or an already replayed one is a
   *  no-op — the fetch may land after the view was rebuilt or switched. */
  applyChildTranscript: (
    root: string,
    id: number,
    childId: number,
    segments: ChildSegment[],
    rows: StreamItem[]
  ) => void;
  /** Replace a session's background-job list — a `bg_list` seed or the list
   *  a `bg_control` call returned. Live updates arrive as `husk:bg_jobs`. */
  setBgJobs: (root: string, id: number, jobs: BgJob[]) => void;
  /** Park a「等待插入」row — a steer the composer just sent. The host says
   *  nothing when it takes one, so the row outlives the IPC call and drops
   *  only on the echo (`UserSteer` / a late `UserPrompt`) or a failed send. */
  addPendingSteer: (root: string, id: number, text: string) => void;
  /** Retire one pending steer — the echo landed or the send failed. */
  dropPendingSteer: (root: string, id: number, text: string) => void;
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
      const existing = s.views.get(k);
      if (existing) {
        // A live buffer can predate the open response. Never replace its
        // rows/state, but fill a missing meter/breakdown from the persisted
        // snapshot — otherwise a session opened while events were already
        // flowing shows 0 tokens/context until its next Usage event.
        const cur = existing.usage;
        const inc = view.usage;
        const hasReading =
          cur.prompt > 0 || cur.completion > 0 || cur.cachedTokens > 0 ||
          cur.cacheWriteTokens > 0 || cur.contextWindow > 0;
        const incomingReading =
          inc.prompt > 0 || inc.completion > 0 || inc.cachedTokens > 0 ||
          inc.cacheWriteTokens > 0 || inc.contextWindow > 0;
        const usage = !hasReading && incomingReading ? inc : cur;
        const contextBreakdown = existing.contextBreakdown ?? view.contextBreakdown;
        // Pagination is deliberately NOT seeded here. `historyStart` says
        // "items[0] is row historyStart of the full log", and a live buffer
        // built from events cannot promise that: its rows are whatever
        // streamed in since the webview attached, which may start, end or
        // overlap anywhere relative to the page `open` returned. Writing the
        // page's start anyway made `useHasMoreHistory` light up with phantom
        // rail dots and let `prependItems` splice in a page that duplicated
        // or skipped rows. A live buffer keeps its own value (undefined when
        // it has none), exactly as it did before this branch existed.
        const historyStart = existing.historyStart ?? view.historyStart;
        const historyTotal = existing.historyTotal ?? view.historyTotal;
        const turnTotal = existing.turnTotal ?? view.turnTotal;
        const turnOffset = existing.turnOffset ?? view.turnOffset;
        // Counts, unlike pagination, are absolute — the persisted snapshot is
        // the base a live buffer may be missing. A buffer that already tracks
        // them keeps its own figure: `husk:session_counts` re-bases it to the
        // committed history at every turn end, so it is never behind for long
        // and is exact after /clear or compaction.
        const runTotal = existing.runTotal ?? meta?.runTotal ?? view.runTotal;
        const taskTotal = existing.taskTotal ?? meta?.taskTotal ?? view.taskTotal;
        if (
          usage === existing.usage &&
          contextBreakdown === existing.contextBreakdown &&
          historyStart === existing.historyStart &&
          historyTotal === existing.historyTotal &&
          turnTotal === existing.turnTotal &&
          turnOffset === existing.turnOffset &&
          runTotal === existing.runTotal &&
          taskTotal === existing.taskTotal
        ) {
          return s;
        }
        const next = new Map(s.views);
        next.set(k, {
          ...existing,
          usage,
          contextBreakdown,
          historyStart,
          historyTotal,
          turnTotal,
          turnOffset,
          runTotal,
          taskTotal,
        });
        return { views: next };
      }
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

  dropView: (root, id) =>
    set((s) => {
      const k = viewKey(root, id);
      if (!s.views.has(k)) return s;
      const next = new Map(s.views);
      next.delete(k);
      const keys = runningKeysOf(next);
      return { views: next, runningKeys: sameKeys(s.runningKeys, keys) ? s.runningKeys : keys };
    }),

  applyChildTranscript: (root, id, childId, segments, rows) =>
    set((s) => {
      const k = viewKey(root, id);
      const v = s.views.get(k);
      if (!v) return s;
      const at = v.items.findIndex(
        (it) => it.kind === "tool" && it.parent === "delegate" && it.childId === childId,
      );
      const card = v.items[at];
      if (card?.kind !== "tool" || card.transcriptLoaded) return s;
      const items = v.items.slice();
      items[at] = { ...card, segments, transcriptLoaded: true };
      // Right AFTER the card: the stream's fold walks items in order and
      // attaches each row to the child its `childId` names.
      items.splice(at + 1, 0, ...rows);
      const next = new Map(s.views);
      next.set(k, { ...v, items });
      return { views: next };
    }),

  setBgJobs: (root, id, jobs) =>
    set((s) => {
      const k = viewKey(root, id);
      const v = s.views.get(k);
      if (!v) return s;
      const next = new Map(s.views);
      next.set(k, { ...v, bgJobs: jobs, bgJobsAt: Date.now() });
      return { views: next };
    }),

  addPendingSteer: (root, id, text) =>
    set((s) => {
      const k = viewKey(root, id);
      const v = s.views.get(k);
      if (!v) return s;
      const next = new Map(s.views);
      next.set(k, { ...v, pendingSteers: [...v.pendingSteers, text] });
      return { views: next };
    }),

  dropPendingSteer: (root, id, text) =>
    set((s) => {
      const k = viewKey(root, id);
      const v = s.views.get(k);
      if (!v) return s;
      const at = v.pendingSteers.indexOf(text);
      if (at < 0) return s;
      const next = new Map(s.views);
      next.set(k, {
        ...v,
        pendingSteers: [...v.pendingSteers.slice(0, at), ...v.pendingSteers.slice(at + 1)],
      });
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

/** Subscribe to a field of the foreground view without rerendering for
 * unrelated streaming text. Selectors should return stable references or
 * primitives, just like a regular Zustand selector. */
export function useActiveViewSelector<T>(select: (view: SessionView) => T): T {
  return useAgentStore(
    (s) => select(s.views.get(viewKey(s.workspaceRoot, s.activeId)) ?? EMPTY_VIEW)
  );
}

/** Whether the foreground session has older history to pull. A boolean, so a
 *  consumer re-renders when it flips, not on every delta. */
export function useHasMoreHistory(): boolean {
  return useAgentStore(
    (s) => (s.views.get(viewKey(s.workspaceRoot, s.activeId))?.historyStart ?? 0) > 0
  );
}

/** Whether a session's view exists yet. The view is folded from history
 *  asynchronously, so anything that has to write INTO it (the `bg_list` seed)
 *  must wait: a write with no view is dropped, not queued. */
export function useHasView(root: string, id: number): boolean {
  return useAgentStore((s) => s.views.has(viewKey(root, id)));
}

/** A session's view outside React, whether or not it is the foreground one. */
export function viewNow(root: string, id: number): SessionView | undefined {
  return useAgentStore.getState().views.get(viewKey(root, id));
}

/** Whether a session has a delegated subagent still running — a boolean, so a
 *  sidebar row re-renders when it flips rather than on every delta. */
export function useHasRunningChildren(root: string, id: number): boolean {
  return useAgentStore((s) => {
    const v = s.views.get(viewKey(root, id));
    return v ? hasRunningChildren(v) : false;
  });
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
/** Only the newest `getModelInfo` may write the chips. Session switches and
 *  streaming edges issue overlapping reads, and IPC resolves in completion
 *  order — a slow response for the session just left would otherwise pin the
 *  header to the previous model's window and price, which is the same A→B→A
 *  flip the composer picker already guards against with its own stamp. */
let chipsSeq = 0;

/** Exported for the composer: `setModel` is a fire-and-forget session command
 *  with no event to key off, so the picker triggers the refresh itself. */
export function refreshAgentChips() {
  void agent
    .getGitInfo()
    .then((g) => useAgentStore.setState({ gitInfo: g }))
    .catch(() => {});
  const seq = ++chipsSeq;
  void agent
    .getModelInfo()
    .then((info) => {
      // Stale: a newer read was issued after this one, so this response
      // describes a session that is no longer in front.
      if (!info || seq !== chipsSeq) return;
      const cur =
        info.models.find((m) => m.model === info.active_model && m.provider === info.active_provider) ??
        info.models.find((m) => m.model === info.active_model);
      useAgentStore.setState({
        // A model with no declared window (a bare id in `config.toml`, or one
        // whose provider was since removed) must NOT inherit the previous
        // model's — that is what pinned the header to a stale window while the
        // session actually ran something else. Clearing lets `TitleBar` fall
        // back to THIS session's own recorded usage.
        ctxWindow: cur?.context_window ?? 0,
        modelCost: cur?.cost,
      });
    })
    .catch(() => {});
}

let disposeStore: (() => void) | null = null;

/** Install the kernel subscription and chip refreshes once. Returning the
 * disposer lets app teardown/HMR and integration tests release every listener. */
export function initAgentStore(): () => void {
  if (disposeStore) return disposeStore;
  let disposed = false;
  let unlisten: (() => void) | undefined;

  // Coalesce the kernel stream into ~60 fps batches: the engine already merges
  // deltas (~120 chars/event), but turn-end flushes and tool transitions land
  // as bursts, and one commit per frame beats one per event.
  const queue: AgentEventEnvelope[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  const flush = () => {
    timer = null;
    if (queue.length === 0) return;
    const batch = queue.splice(0, queue.length);
    try {
      const s = useAgentStore.getState();
      const next = new Map(s.views);
      // Group by session so each view copies its history once per batch.
      const byView = new Map<string, AgentEventEnvelope["event"][]>();
      for (const { session, event, root } of batch) {
        const k = viewKey(root ?? "", session);
        const g = byView.get(k);
        if (g) g.push(event);
        else byView.set(k, [event]);
      }
      for (const [k, events] of byView) {
        next.set(k, applyEvents(next.get(k) ?? emptyView(), events));
      }
      // NOTE: `activeId` is deliberately NOT driven by events — otherwise a
      // background session's traffic yanks the visible session, and the
      // frontend's pointer diverges from the backend's `active_id`.
      useAgentStore.setState({ views: next, runningKeys: (() => {
        const keys = runningKeysOf(next);
        return sameKeys(s.runningKeys, keys) ? s.runningKeys : keys;
      })() });
    } catch (error) {
      // One malformed event must not kill the pump: the next batch still runs
      // and the rest of the kernel stream keeps folding.
      console.error("agent event batch failed:", error);
    }
  };

  void agent.onAgentEvent((env) => {
    if (disposed) return;
    queue.push(env);
    try {
      // The notification rule reads the pointer NOW rather than from the batch
      // below — "can the user see this" is about this instant, and a background
      // session finishing is exactly what the bell exists for.
      const s = useAgentStore.getState();
      notifyForEvent(env, { root: s.workspaceRoot, id: s.activeId });
    } catch (error) {
      console.error("agent event notification failed:", error);
    } finally {
      // `finally` is load-bearing: without it a throwing notification leaves
      // `timer` null forever, so every later event only grows `queue` and the
      // stream freezes while memory climbs.
      if (timer == null) timer = setTimeout(flush, 16);
    }
  }).then((stop) => {
    if (disposed) stop();
    else unlisten = stop;
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
  const unsubscribe = useAgentStore.subscribe(syncChips);
  syncChips();

  // A settings save broadcasts CONFIG_CHANGED_EVENT — re-read so a renamed /
  // repriced / re-windowed model shows immediately.
  window.addEventListener(agent.CONFIG_CHANGED_EVENT, refreshAgentChips);
  // A `git commit` in a terminal cleans the worktree without any event of
  // ours: re-read when the window comes back, which is when the reader looks
  // at the now-stale chip and the 改动 pill.
  window.addEventListener("focus", refreshAgentChips);

  // A foreground session that rang while the window was away is on screen
  // again the moment the window is — its rows count as read, the same as
  // opening it from the sidebar.
  const readForeground = () => {
    if (!isAppInFront()) return;
    const s = useAgentStore.getState();
    markSessionRead(s.workspaceRoot, s.activeId);
  };
  const stopFront = onAppFront(readForeground);
  disposeStore = () => {
    if (disposed) return;
    disposed = true;
    if (timer !== null) clearTimeout(timer);
    queue.length = 0;
    unlisten?.();
    unsubscribe();
    window.removeEventListener(agent.CONFIG_CHANGED_EVENT, refreshAgentChips);
    window.removeEventListener("focus", refreshAgentChips);
    stopFront();
    ++chipsSeq; // pending model reads from a disposed subscription are stale
    disposeStore = null;
  };
  return disposeStore;
}
