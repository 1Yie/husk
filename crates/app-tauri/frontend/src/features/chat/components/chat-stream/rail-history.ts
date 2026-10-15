interface RailHistory {
  historyStart?: number;
  turnOffset?: number;
  turnTotal?: number;
}
interface HistoryTurn {
  userText?: string;
  steps: readonly { type: string }[];
}

/** Only a response fragment at a real page boundary can have a missing
 * prompt. A standalone notice/compaction block also has no userText, but
 * is fully loaded content and must never create a history placeholder. */
export function railHistoryBoundary<T extends HistoryTurn>(
  turns: readonly T[],
  history: RailHistory,
) {
  const loadedUserTurns = turns.filter((turn) => Boolean(turn.userText)).length;
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
