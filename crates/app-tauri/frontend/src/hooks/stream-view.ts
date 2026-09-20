// Stream view model — the display-only shape every render path folds
// into (`applyEvent` for the live stream, `viewFromHistory` for a
// reopened session). Pure data: no React, no IPC.

import type { AgentState } from "../types";

export type StreamItem =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; streaming: boolean }
  | { kind: "thinking"; text: string; done: boolean }
  | {
      kind: "tool";
      name: string;
      args: string;
      content?: string;
      ok?: boolean;
      uiType?: string;
      approval?: {
        requestId: number;
        diff?: string;
        fuzzy?: boolean;
        resolved?: boolean;
        approved?: boolean;
      };
    }
  | {
      kind: "approval";
      requestId: number;
      toolName: string;
      diff: string;
      fuzzy: boolean;
      resolved?: boolean;
      approved?: boolean;
    }
  | { kind: "system"; text: string };

export interface SessionView {
  items: StreamItem[];
  state: AgentState | null;
  streaming: boolean;
  usage: { prompt: number; completion: number; contextWindow: number };
  /** Assistant output rate (tok/s) — a live estimate while deltas flow
   * (chars/4 over active stream time), finalized from the real
   * `completion_tokens` when the `Usage` event lands. Resets each turn. */
  toksPerSec: number;
  /** Internal accumulators for `toksPerSec`. `activeMs` sums only the
   * gaps between consecutive deltas (capped per gap), so tool-execution
   * pauses don't drag the decode-rate reading down. */
  rate: { chars: number; activeMs: number; lastAt: number | null };
}

export const emptyView = (): SessionView => ({
  items: [],
  state: null,
  streaming: false,
  usage: { prompt: 0, completion: 0, contextWindow: 0 },
  toksPerSec: 0,
  rate: { chars: 0, activeMs: 0, lastAt: null },
});

/** The pending approval a composer permission strip should surface —
 * the most recent unresolved `ApprovalRequested`. `ToolCallStarted`
 * marks it resolved/approved, `ToolDecision` (deny) resolves via state
 * transitions. Returns null when nothing is awaiting a verdict. */
export function pendingApprovalOf(view: SessionView): {
  requestId: number;
  toolName: string;
  args?: string;
  diff: string;
} | null {
  for (let i = view.items.length - 1; i >= 0; i--) {
    const it = view.items[i];
    if (it.kind === "tool" && it.approval && !it.approval.resolved) {
      return {
        requestId: it.approval.requestId,
        toolName: it.name,
        args: it.args,
        diff: it.approval.diff || "",
      };
    }
    if (it.kind === "approval" && !it.resolved) {
      return {
        requestId: it.requestId,
        toolName: it.toolName,
        diff: it.diff,
      };
    }
  }
  return null;
}
