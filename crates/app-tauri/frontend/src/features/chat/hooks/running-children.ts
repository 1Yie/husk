import type { SessionView } from "@/features/chat/hooks/stream-view";

/** Subagents still running in this session — the ones a mid-run steer can reach.
 * A card is running until its report lands (`content` unset). */
export function extractRunningChildren(view: SessionView): { childId: number; name: string; task: string }[] {
  if (!view || !view.items) return [];
  return view.items.flatMap((it) =>
    it.kind === "tool" && it.parent === "delegate" && it.childId != null && it.content === undefined
      ? [{ childId: it.childId, name: it.name, task: it.args }]
      : [],
  );
}
