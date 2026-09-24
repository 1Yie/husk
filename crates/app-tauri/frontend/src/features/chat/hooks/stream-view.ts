// Stream view model — the display-only shape every render path folds
// into (`applyEvent` for the live stream, `viewFromHistory` for a
// reopened session). Pure data: no React, no IPC.

import type { AgentState } from "@/types";
import type { PlanPayload } from "@/types";

export type StreamItem =
  /** `hi` = global history index — stamped by `viewFromHistory` for
   * persisted messages; live items leave it undefined. It makes turn
   * keys stable when older pages prepend above. */
  | { kind: "user"; text: string; ts?: number; hi?: number }
  | {
      kind: "assistant";
      text: string;
      streaming: boolean;
      /** First-delta arrival (epoch ms) — distinct from `ts` (commit stamp):
       *  a tools step's phase end is "when the answer STARTED streaming",
       *  not when it finished writing. */
      startedAt?: number;
      ts?: number;
      hi?: number;
    }
  | {
      kind: "thinking";
      text: string;
      done: boolean;
      /** Epoch ms when this thinking block STARTED — stamped by `applyEvent`
       *  on the first reasoning delta. Drives the status row's "思考中 Xs"
       *  clock so a session switch doesn't restart it at 0. */
      startedAt?: number;
      /** Epoch ms when the block CLOSED — stamped by `closeOpenThinking`
       *  live, or persisted `m.ts` on replay. `ts - startedAt` is the settled
       *  span, same formula the tools step uses. */
      ts?: number;
      hi?: number;
    }
  | {
      kind: "tool";
      name: string;
      args: string;
      content?: string;
      ok?: boolean;
      uiType?: string;
      /** Epoch ms when the call STARTED (`ToolCallStarted`) — the tools
       *  status row's clock anchor, stable across a session switch. */
      startedAt?: number;
      /** Result timestamp (epoch ms) — persisted on replayed items; the
       *  tools step derives its end stamp from the latest one. */
      ts?: number;
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
  | {
      kind: "system";
      text: string;
      /** Emitted between turns — folds into its own block. */
      standalone?: boolean;
      /** Arrival stamp — a system notice after a tools phase ends that
       *  phase's clock (`phaseEnd` on the step). */
      ts?: number;
      hi?: number;
    }
  | {
      kind: "compaction";
      /** Context estimate before the pass, in tokens. */
      before: number;
      /** Context estimate after the splice. */
      after: number;
      /** Messages folded into the summary note. */
      removed: number;
      /** The compactor's summary — expanded in the card on demand. */
      note: string;
      /** True while the pass is still running (`Compacting` saw it start,
       *  no `Compacted` yet) — the card renders as a progress placeholder. */
      pending?: boolean;
      /** `true` = the user ran `/compact` — the card's "手动" badge. */
      manual?: boolean;
      ts?: number;
      hi?: number;
    }
  | {
      kind: "plan";
      /** The `submit_plan` payload — parsed once at ingest. */
      plan: PlanPayload;
      ts?: number;
      hi?: number;
    };

/** How a file landed in the changes panel (add/update/delete). */
export type ChangeKind = "add" | "update" | "delete";

/** One patch a write tool applied to `path`; `n` is the touch counter. */
export interface ChangePatch {
  tool: string;
  diff: string;
  n: number;
}

/** A file the agent touched this session. */
export interface FileChange {
  path: string;
  kind: ChangeKind;
  patches: ChangePatch[];
  adds: number;
  dels: number;
  fuzzy: boolean;
  /** True while the only diffs came from `ApprovalRequested` previews —
   * not yet committed (pending or denied). Cleared on ToolCallFinished. */
  pending?: boolean;
}

export interface SessionView {
  /** An unresolved `QuestionAsked` — the composer renders it in the same
   * slot as an approval strip. Cleared when answered (locally), when the
   * turn ends, or when a new prompt lands. */
  pendingQuestion?: { requestId: number; question: string; options: import("@/types").AskOption[] };
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
  /** A `UserPrompt` opened a turn no settled `StateChanged` has closed —
   *  what `standalone` tags off (state alone can't tell: a manual
   *  `/compact` is Compacting between turns). */
  turnOpen: boolean;
  /** Epoch ms when the CURRENT turn began — stamped on `UserPrompt` /
   *  `TurnRetry`. The "this turn has been running Xs" statistic reads
   *  `Date.now() - turnStartedAt`; being a view field, it survives a session
   *  switch (the old per-mount wall clock reset to 0). Undefined once the
   *  turn settles. */
  turnStartedAt?: number;
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
  /** Files the agent wrote this session — feeds the changes panel.
   *  Accumulated live by `applyEvent`, rebuilt by `viewFromHistory`. */
  changes: FileChange[];
  /** Parked follow-up prompts — the kernel's queue (`SetQueued` writes,
   *  `QueuedPrompts` events echo it back, `open` seeds it from
   *  `SessionMeta`). Lives on the view so a session switch preserves it. */
  queuedPrompts: string[];
}

export const emptyView = (): SessionView => ({
  items: [],
  changes: [],
  queuedPrompts: [],
  state: null,
  streaming: false,
  turnOpen: false,
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
