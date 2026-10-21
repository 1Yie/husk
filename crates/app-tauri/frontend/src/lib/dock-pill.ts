// Which file view the dock pill speaks for. One rule, so the page and its
// tests cannot drift: 后台 is a detour and never names the pill, 改动 counts
// while the session has written files, 产物 whenever it has any.

import type { FilesTab } from "@/features/chat/components/dock/state";

export interface DockPill {
  tab: FilesTab | null;
  /** Files in `tab` — 0 hides the pill. */
  count: number;
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
