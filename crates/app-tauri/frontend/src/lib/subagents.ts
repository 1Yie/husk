// Subagent wording shared by the composer capsule and the sidebar row — one
// place decides how a running child reads.

import { childIsLive, type ChildStatus, type SessionChild } from "@/features/chat/hooks/running-children";

export interface ChildrenSummary {
  /** What the capsule says — `null` from `summarizeChildren` means no capsule. */
  text: string;
  count: number;
  /** The capsule's glyph: the orb while any child is out, a verdict after. */
  tone: "running" | "failed" | "ok";
}

/** A card's name without its run suffix: 「review #1」 reads as 「review」. */
export function childLabel(name: string): string {
  return name.replace(/\s*#\d+\s*$/, "").trim() || "子代理";
}

/** Shown beside a steer box while the child is parked: the kernel holds the
 *  text and injects it only once the child resumes. */
export const WAITING_STEER_HINT = "等待主代理回复中，插话会在子代理恢复后生效";

/** How a child's state reads in a row. */
export const CHILD_STATUS_LABEL: Record<ChildStatus, string> = {
  running: "运行中",
  waiting: "等待主代理回复",
  done: "已完成",
  failed: "失败",
  aborted: "已中断",
};

/** The capsule's one line. Children still out come first — something is still
 *  happening; once all of them ended, a failure outranks the successes, the
 *  same order the jobs capsule uses. */
export function summarizeChildren(children: readonly SessionChild[]): ChildrenSummary | null {
  if (children.length === 0) return null;
  const live = children.filter(childIsLive);
  // A parked child is stuck on its own question — the one state worth saying
  // out loud, even while its siblings keep working.
  const parked = live.filter((c) => c.status === "waiting");
  if (parked.length === 1) {
    return { text: `子代理 ${childLabel(parked[0].name)} 等待主代理回复`, count: live.length, tone: "running" };
  }
  if (parked.length > 1) {
    return { text: `${parked.length} 个子代理等待主代理`, count: live.length, tone: "running" };
  }
  if (live.length === 1) {
    return { text: `子代理 ${childLabel(live[0].name)} 运行中`, count: 1, tone: "running" };
  }
  if (live.length > 1) {
    return { text: `${live.length} 个子代理运行中`, count: live.length, tone: "running" };
  }
  const failed = children.filter((c) => c.status === "failed" || c.status === "aborted").length;
  if (failed > 0) return { text: `${failed} 个子代理未完成`, count: children.length, tone: "failed" };
  return { text: `${children.length} 个子代理已完成`, count: children.length, tone: "ok" };
}
