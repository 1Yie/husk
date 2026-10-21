// Which file view the dock pill speaks for, and whether the 改动 view still
// has anything to announce. One rule, so the page and its tests cannot drift:
// 后台 is a detour and never names the pill, 改动 counts while the worktree
// still has something uncommitted, and 产物 counts whenever it has files.

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
 *  it still has content; 改动 yields to 产物 once it went live-and-empty. */
export function dockPill(input: {
  filesTab: FilesTab;
  changes: number;
  artifacts: number;
  /** `changesStillLive` — the worktree's answer for the 改动 view. */
  changesLive: boolean;
}): DockPill {
  const { filesTab, changes, artifacts, changesLive } = input;
  if (filesTab === "artifacts" && artifacts > 0) return { tab: "artifacts", count: artifacts };
  if (changesLive && changes > 0) return { tab: "changes", count: changes };
  if (artifacts > 0) return { tab: "artifacts", count: artifacts };
  return { tab: null, count: 0 };
}
