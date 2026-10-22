import type { SessionView } from "@/features/chat/hooks/stream-view";

/** A subagent still running in this session — a mid-run steer names its `childId`. */
export interface RunningChild {
  childId: number;
  name: string;
  task: string;
  /** The question it parked on for the parent agent, while it waits. */
  waiting?: string;
}

/** Subagents still running in this session — the ones a mid-run steer can reach.
 * A card is running until its report lands (`content` unset). */
export function extractRunningChildren(view: SessionView): RunningChild[] {
  if (!view || !view.items) return [];
  return view.items.flatMap((it) =>
    it.kind === "tool" && it.parent === "delegate" && it.childId != null && it.content === undefined
      ? [{ childId: it.childId, name: it.name, task: it.args, waiting: it.waiting }]
      : [],
  );
}

/** The same question as a boolean — a sidebar row only switches glyphs, and
 *  must not allocate a list per store batch. */
export function hasRunningChildren(view: SessionView): boolean {
  if (!view || !view.items) return false;
  return view.items.some(
    (it) => it.kind === "tool" && it.parent === "delegate" && it.childId != null && it.content === undefined,
  );
}
