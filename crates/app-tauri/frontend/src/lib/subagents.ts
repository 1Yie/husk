// Subagent wording shared by the composer capsule and the sidebar row — one
// place decides how a running child reads.

import type { RunningChild } from "@/features/chat/hooks/running-children";

export interface ChildrenSummary {
  /** What the capsule says — `null` from `summarizeChildren` means no capsule. */
  text: string;
  count: number;
}

/** A card's name without its run suffix: 「review #1」 reads as 「review」. */
export function childLabel(name: string): string {
  return name.replace(/\s*#\d+\s*$/, "").trim() || "子代理";
}

/** The capsule's one line. One child is worth naming; several are worth counting. */
export function summarizeChildren(children: readonly RunningChild[]): ChildrenSummary | null {
  if (children.length === 0) return null;
  // A parked child is stuck on its own question — the one state worth saying
  // out loud, even while its siblings keep working.
  const parked = children.filter((c) => c.waiting != null);
  if (parked.length === 1) {
    return { text: `子代理 ${childLabel(parked[0].name)} 等待主代理回复`, count: children.length };
  }
  if (parked.length > 1) {
    return { text: `${parked.length} 个子代理等待主代理`, count: children.length };
  }
  if (children.length === 1) {
    return { text: `子代理 ${childLabel(children[0].name)} 运行中`, count: 1 };
  }
  return { text: `${children.length} 个子代理运行中`, count: children.length };
}
