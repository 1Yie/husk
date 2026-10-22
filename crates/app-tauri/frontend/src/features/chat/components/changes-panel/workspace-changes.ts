// 改动 at the workspace level — the checkout's uncommitted git files, one entry
// per workspace root. `view.changes` records what a SESSION wrote (it is folded
// from write-tool diffs, so it shrinks to whatever a reopened session replays);
// the panel shows what the repository looks like right now, which is the same
// list for every session of that workspace and survives a restart.

import { create } from "zustand";
import { workspaceChanges, type WorkspaceChangeRow } from "@/lib/agent-ipc";
import type { FileChange } from "@/features/chat/hooks/stream-view";

/** One workspace's slice of the panel. */
export interface WorkspaceChanges {
  files: FileChange[];
  /** False = the root is not a git repository; the page then falls back to the
   *  session's own change list. */
  repo: boolean;
  loading: boolean;
  /** When the list last landed (epoch ms) — 0 before the first read. */
  at: number;
}

/** Stable "nothing yet" handle: a selector must not mint a fresh object. */
export const EMPTY_WORKSPACE_CHANGES: WorkspaceChanges = {
  files: [],
  repo: true,
  loading: false,
  at: 0,
};

/** One git row as the panel's row: the file's whole uncommitted delta as a
 *  single patch. The per-call stacking of `view.changes` (`tool · 第 N 次修改`)
 *  has no meaning here — git reports one diff per file, not one per write. */
export function toFileChange(row: WorkspaceChangeRow): FileChange {
  return {
    path: row.path,
    kind: row.kind,
    patches: row.diff ? [{ tool: "git", diff: row.diff, n: 1 }] : [],
    adds: row.adds,
    dels: row.dels,
    fuzzy: false,
    binary: row.binary,
    truncated: row.truncated,
  };
}

interface WorkspaceChangesStore {
  byRoot: Record<string, WorkspaceChanges>;
  write: (root: string, patch: Partial<WorkspaceChanges>) => void;
}

export const useWorkspaceChangesStore = create<WorkspaceChangesStore>((set) => ({
  byRoot: {},
  write: (root, patch) =>
    set((s) => ({
      byRoot: { ...s.byRoot, [root]: { ...(s.byRoot[root] ?? EMPTY_WORKSPACE_CHANGES), ...patch } },
    })),
}));

/** The workspace's list outside React. */
export function workspaceChangesNow(root: string): WorkspaceChanges {
  return useWorkspaceChangesStore.getState().byRoot[root] ?? EMPTY_WORKSPACE_CHANGES;
}

/** A turn ends with a burst of tool results and the read behind this walks
 *  every dirty file, so a burst earns one read, this long after its last ask. */
const DEBOUNCE_MS = 300;

/** The read in flight per root. */
const inflight = new Map<string, Promise<void>>();
/** Roots asked for again while their read was running. */
const rerun = new Set<string>();
/** Debounce timers per root. */
const timers = new Map<string, ReturnType<typeof setTimeout>>();

async function read(root: string) {
  const { write } = useWorkspaceChangesStore.getState();
  write(root, { loading: true });
  try {
    const r = await workspaceChanges(root);
    write(root, {
      files: r.files.map(toFileChange),
      repo: r.repo,
      loading: false,
      at: Date.now(),
    });
  } catch {
    // A failed read keeps the previous list: an empty panel claims "the tree
    // is clean", which is a different thing from "could not tell".
    write(root, { loading: false });
  }
}

/** Read `root`'s uncommitted files now. One read runs at a time per root; a
 *  call made while it is in flight is answered by one more read once it lands,
 *  so no caller is left looking at a snapshot older than its own request. */
export function refreshWorkspaceChanges(root: string): Promise<void> {
  if (!root) return Promise.resolve();
  const running = inflight.get(root);
  if (running) {
    rerun.add(root);
    return running;
  }
  const run = (async () => {
    try {
      await read(root);
      // Loop, not a single re-run: the second read can be overtaken as well.
      while (rerun.delete(root)) await read(root);
    } finally {
      inflight.delete(root);
    }
  })();
  inflight.set(root, run);
  return run;
}

/** The event-driven entry point — see [`DEBOUNCE_MS`]. */
export function scheduleWorkspaceChanges(root: string) {
  if (!root || timers.has(root)) return;
  timers.set(
    root,
    setTimeout(() => {
      timers.delete(root);
      void refreshWorkspaceChanges(root);
    }, DEBOUNCE_MS),
  );
}

/** Drop every cached list and pending read — a launch reset, and test teardown. */
export function clearWorkspaceChanges() {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
  inflight.clear();
  rerun.clear();
  useWorkspaceChangesStore.setState({ byRoot: {} });
}
