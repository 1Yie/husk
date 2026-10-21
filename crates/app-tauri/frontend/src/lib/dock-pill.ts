// Which file view the dock pill speaks for, and whether the 改动 view still
// has something uncommitted to report. One rule, so the page and its tests
// cannot drift: 后台 is a detour and never names the pill, 改动 counts while
// the session has written files, 产物 whenever it has any.
//
// The worktree decides the 改动 view's NUMBERS, never its existence: a commit
// stops it claiming `+N −M`, but the list stays reachable.

import type { FilesTab } from "@/features/chat/components/dock/state";

export interface DockPill {
  tab: FilesTab | null;
  /** Files in `tab` — 0 hides the pill. */
  count: number;
}

/** Whether the 改动 view still owes the worktree something. Inside a
 *  repository that is "uncommitted changes exist"; outside one there is
 *  nothing to commit, so the session's own list is the answer. `null` (the
 *  read has not landed yet) keeps the session's list — a pill the reader is
 *  already looking at must not blink out while the first `git_info` resolves. */
export function changesStillLive(
  git: { repo: boolean; dirty: number } | null,
  changes: number,
): boolean {
  if (!git) return changes > 0;
  return git.repo ? git.dirty > 0 : changes > 0;
}

/** The view the pill names and its file count. The stored file tab wins while
 *  it still has content; 改动 yields to 产物 only when it has nothing left. */
export function dockPill(input: {
  filesTab: FilesTab;
  changes: number;
  artifacts: number;
}): DockPill {
  const { filesTab, changes, artifacts } = input;
  if (filesTab === "artifacts" && artifacts > 0) return { tab: "artifacts", count: artifacts };
  if (changes > 0) return { tab: "changes", count: changes };
  if (artifacts > 0) return { tab: "artifacts", count: artifacts };
  return { tab: null, count: 0 };
}
