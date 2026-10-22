import type { SessionView } from "@/features/chat/hooks/stream-view";

/** The composer reads control rows (approvals, todos, plans), question/queue
 * state, and the streaming flag. Text/reasoning updates don't change any of
 * those inputs. Keep its draft, pickers, and history scans out of that path.
 * Compare every other row by identity so changes to earlier tool approvals
 * still reach the composer even while a later reasoning row is streaming. */
export function sameComposerView(previous: SessionView, next: SessionView): boolean {
  if (previous === next) return true;
  if (
    previous.streaming !== next.streaming ||
    previous.pendingQuestion !== next.pendingQuestion ||
    previous.queuedPrompts !== next.queuedPrompts ||
    previous.pendingSteers !== next.pendingSteers ||
    previous.items.length !== next.items.length
  ) return false;

  for (let i = 0; i < previous.items.length; i++) {
    const before = previous.items[i];
    const after = next.items[i];
    if (before === after) continue;
    if (
      before.kind === after.kind &&
      (before.kind === "assistant" || before.kind === "thinking")
    ) continue;
    return false;
  }
  return true;
}
