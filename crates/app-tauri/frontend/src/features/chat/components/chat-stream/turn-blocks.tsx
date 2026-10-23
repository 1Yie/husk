// One phase of an assistant turn, as a block. Shared by the main turn and a
// delegated child's card, so a child's transcript is drawn by the same pieces
// in the same order as the turn that produced it.

import { AssistantStatus } from "@/features/chat/components/assistant-status/index";

/** A reasoning block: live 「思考中」 while it is still arriving, 「思考过程」
 *  once the phase after it landed. */
export function ThinkingBlock({
  text,
  done,
  startedAt,
  elapsedMs,
}: {
  text: string;
  done: boolean;
  /** Epoch ms the block opened — keeps the settled `· 20s` across a remount. */
  startedAt?: number;
  /** Persisted span for a block replayed from history. */
  elapsedMs?: number;
}) {
  const thinkingText = text.trim();
  if (!done && !thinkingText) return null;
  return (
    <div className="w-full">
      <AssistantStatus
        mode={done ? "thought" : "thinking"}
        thinkingText={thinkingText}
        startedAt={startedAt}
        elapsedMs={elapsedMs}
      />
    </div>
  );
}
