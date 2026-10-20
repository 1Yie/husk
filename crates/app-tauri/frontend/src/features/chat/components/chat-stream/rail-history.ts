interface RailHistory {
  historyStart?: number;
  turnOffset?: number;
  turnTotal?: number;
}
interface HistoryTurn {
  userText?: string;
  steer?: boolean;
  steps: readonly { type: string }[];
}

/** Whether a turn's bubble opens a user turn — the backend's `counts_as_turn`
 * on this side. A mid-turn steer is drawn as a bubble but belongs to the turn
 * it interrupted, so it never counts toward `turnTotal` / `turnOffset`. */
export function opensTurn(turn: { userText?: string; steer?: boolean }): boolean {
  return Boolean(turn.userText) && !turn.steer;
}

/** Only a response fragment at a real page boundary can have a missing
 * prompt. A standalone notice/compaction block also has no userText, but
 * is fully loaded content and must never create a history placeholder. */
export function railHistoryBoundary<T extends HistoryTurn>(
  turns: readonly T[],
  history: RailHistory,
) {
  const loadedUserTurns = turns.filter(opensTurn).length;
  const unloaded = history.historyStart === 0 ? 0 : Math.max(
    0,
    history.turnOffset ?? (history.turnTotal ?? loadedUserTurns) - loadedUserTurns,
  );
  const first = turns[0];
  const boundaryTurn = unloaded > 0 && !first?.userText && first?.steps.some((step) =>
    step.type === "text" || step.type === "thinking" || step.type === "tools" || step.type === "plan",
  ) ? first : null;
  return { unloaded, boundaryTurn };
}
