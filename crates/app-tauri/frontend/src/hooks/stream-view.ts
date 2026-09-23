// Stream view model — the display-only shape every render path folds
// into (`applyEvent` for the live stream, `viewFromHistory` for a
// reopened session). Pure data: no React, no IPC.

import type { AgentState } from "../types";

export type StreamItem =
  /** `hi` = global history index — stamped by `viewFromHistory` for
   * persisted messages; live items leave it undefined. It makes turn
   * keys stable when older pages prepend above. */
  | { kind: "user"; text: string; ts?: number; hi?: number }
  | { kind: "assistant"; text: string; streaming: boolean; ts?: number; hi?: number }
  | { kind: "thinking"; text: string; done: boolean; hi?: number }
  | {
      kind: "tool";
      name: string;
      args: string;
      content?: string;
      ok?: boolean;
      uiType?: string;
      /** A delegated child's streamed answer text, before its report lands. */
      live?: string;
      /** The same child's streamed reasoning trace. */
      liveReasoning?: string;
      /** Set on calls emitted inside another tool (`batch_execute`
       * items) — the stream nests them under the parent capsule. */
      parent?: string;
      hi?: number;
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
      hi?: number;
    }
  | { kind: "system"; text: string; hi?: number };

export interface SessionView {
  /** An unresolved `QuestionAsked` — the composer renders it in the same
   * slot as an approval strip. Cleared when answered (locally), when the
   * turn ends, or when a new prompt lands. */
  pendingQuestion?: { requestId: number; question: string; options: import("../types").AskOption[] };
  items: StreamItem[];
  /** Index into the full persisted history where `items[0]` starts —
   * `> 0` means older pages exist and can be fetched via `history_page`.
   * Undefined for live-only views. */
  historyStart?: number;
  /** Full persisted message count — the rail uses it to size the
   * overview track against the whole session, not just loaded pages. */
  historyTotal?: number;
  /** Full turn count (non-hidden user messages) — drives the rail's
   * unloaded placeholder marks. */
  turnTotal?: number;
  /** Ordinal (among non-hidden user turns) of the first turn inside the
   * loaded slice — the exact position of the loaded/unloaded boundary on
   * the rail. The unloaded dots are ordinals `0 .. turnOffset-1`, and a
   * dot click seeks straight to its own ordinal. */
  turnOffset?: number;
  state: AgentState | null;
  streaming: boolean;
  usage: {
    prompt: number;
    completion: number;
    contextWindow: number;
    /** Prompt tokens served from the provider cache — `prompt -
     * cachedTokens` is the uncached share billed at full rate. */
    cachedTokens: number;
  };
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
  usage: { prompt: 0, completion: 0, contextWindow: 0, cachedTokens: 0 },
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
