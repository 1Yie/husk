// The 改动 panel's feed: the workspace's uncommitted git files, re-read when
// they can have changed — at mount, when the window comes back, and whenever a
// write-capable tool finishes in ANY session of that workspace.

import { useEffect } from "react";
import { useAgentStore } from "@/stores/agent-store";
import { SUMMARY_TOOLS } from "@/features/chat/hooks/apply-event";
import {
  EMPTY_WORKSPACE_CHANGES,
  refreshWorkspaceChanges,
  scheduleWorkspaceChanges,
  useWorkspaceChangesStore,
  type WorkspaceChanges,
} from "@/features/chat/components/changes-panel/workspace-changes";

/** Tools that write without emitting a diff of their own — a shell command
 *  (formatter, `git checkout`, build script) and a background job both rewrite
 *  the worktree. A `batch_execute` capsule can carry either. */
const SHELL_TOOLS = new Set(["bash", "bg_run", "batch_execute"]);

const WRITERS = new Set([...SUMMARY_TOOLS, ...SHELL_TOOLS]);

/** What a write leaves behind in the fold: finished writer tool calls plus each
 *  session's streaming flag. Compared as one string, so the subscription below
 *  can tell "something was written" from the 60 fps text deltas — and the
 *  turn-end edge (`streaming` → false) covers a write no tool of ours claims. */
function writeMark(root: string): string {
  const { views } = useAgentStore.getState();
  const prefix = `${root}:`;
  let tools = 0;
  let streaming = 0;
  for (const [key, view] of views) {
    if (!key.startsWith(prefix)) continue;
    if (view.streaming) streaming += 1;
    for (const item of view.items) {
      if (item.kind === "tool" && item.ts !== undefined && WRITERS.has(item.name)) tools += 1;
    }
  }
  return `${tools}:${streaming}`;
}

/** The workspace's uncommitted files. Rows come from git, so the list does not
 *  belong to a session: switching sessions keeps it, reopening one cannot
 *  shrink it. `repo === false` is the caller's cue to fall back to the
 *  session's own `view.changes` (a plain folder has no git to ask). */
export function useWorkspaceChanges(root: string): WorkspaceChanges {
  const slice = useWorkspaceChangesStore((s) => s.byRoot[root] ?? EMPTY_WORKSPACE_CHANGES);

  useEffect(() => {
    if (!root) return;
    void refreshWorkspaceChanges(root);
  }, [root]);

  // The window coming back is the moment the reader looks — and a `git commit`
  // in a terminal, or an editor save, changed the tree with no event of ours.
  useEffect(() => {
    if (!root) return;
    const reread = () => {
      if (document.visibilityState === "hidden") return;
      void refreshWorkspaceChanges(root);
    };
    window.addEventListener("focus", reread);
    document.addEventListener("visibilitychange", reread);
    return () => {
      window.removeEventListener("focus", reread);
      document.removeEventListener("visibilitychange", reread);
    };
  }, [root]);

  // Watched on the store rather than through a React selector: a parked
  // workspace's session writes into this root too, and the panel is
  // workspace-level.
  useEffect(() => {
    if (!root) return;
    let mark = writeMark(root);
    return useAgentStore.subscribe(() => {
      const next = writeMark(root);
      if (next === mark) return;
      mark = next;
      scheduleWorkspaceChanges(root);
    });
  }, [root]);

  return slice;
}
