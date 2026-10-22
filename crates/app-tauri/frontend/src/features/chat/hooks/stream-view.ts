// Stream view model — the display-only shape every render path folds
// into (`applyEvent` for the live stream, `viewFromHistory` for a
// reopened session). Pure data: no React, no IPC.

import type { AgentState, ApprovalActions, PlanPayload } from "@/types";

/** One run inside a delegated child's body. A text run is the child's streamed
 *  answer; a tool run points at the child's `children` entry by index, so a
 *  tool's data is stored once. */
export type ChildSegment =
  | { kind: "text"; text: string }
  | { kind: "tool"; index: number };

export type StreamItem =
  /** `hi` = global history index — stamped by `viewFromHistory` for
   * persisted messages; live items leave it undefined. It makes turn
   * keys stable when older pages prepend above.
   *
   * `steer` marks a mid-turn interjection: drawn as a bubble, but it belongs
   * to the turn already running, so it never advances a turn ordinal — the
   * same rule as the store's `counts_as_turn`. */
  | { kind: "user"; text: string; ts?: number; hi?: number; steer?: boolean }
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
      /** A delegated child's body in arrival order: text runs interleaved with
       *  its own tool calls, so the card reads top-to-bottom like a session. */
      segments?: ChildSegment[];
      /** Set on calls emitted inside another tool (`batch_execute`
       * items) — the stream nests them under the parent capsule. */
      parent?: string;
      /** Set on a delegated subagent's card: the id a `SteerChild` names. */
      childId?: number;
      hi?: number;
      approval?: {
        requestId: number;
        diff?: string;
        fuzzy?: boolean;
        /** Grant buttons this card offers (associate another directory). */
        actions?: ApprovalActions;
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
      actions?: ApprovalActions;
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

/** A workspace file the session produced — office documents
 *  (pptx/docx/xlsx), downloaded assets, etc. Office mode's counterpart to
 *  `changes`: those tools write whole files, never emit diffs. Registered
 *  by `recordArtifact` on mutating tool results (the args locator IS the
 *  file path), rebuilt from persisted history the same way. */
export interface Artifact {
  /** Workspace-relative path (office sessions resolve it under the
   *  office workspace root). */
  path: string;
  /** Basename — the row label. */
  name: string;
  /** Lowercased extension — picks the row icon; gates the ready-toast to
   *  document types (asset downloads list quietly). */
  ext: string;
  /** Tool that last wrote it (`office_create`, `web_download`, …). */
  tool: string;
  /** Epoch ms of the last write event. */
  updatedAt: number;
}

/** Prompt composition by category, in estimated tokens. */
export interface ContextBreakdown {
  system: number;
  tools: number;
  user: number;
  memory: number;
  assistant: number;
  reasoning: number;
  tool_calls: number;
  tool_results: number;
  other: number;
}

/** One background job (`bg_run`) as the host reports it — `husk:bg_jobs`
 *  carries the session's whole list. */
export interface BgJob {
  id: string;
  command: string;
  state:
    | { kind: "running" }
    | { kind: "paused" }
    /** `label` is the tool's wording (`exited(0)`, `killed(timeout)`), shown
     *  as is; `code` is set only for a normal exit. `reason` is what the view
     *  branches on — never the label's text. */
    | { kind: "done"; label: string; code: number | null; reason: BgEndReason };
  /** Wall-clock start, epoch ms. */
  started_at_ms: number;
  /** Run time at the snapshot, paused time excluded — a running job's clock
   *  ticks on from here. */
  elapsed_ms: number;
  notify: boolean;
  /** The context that started the job installed its own completion sink.
   *  `false` in a host that installs one per session means a delegated
   *  child's job — it reports to this list and nobody else. */
  has_own_sink: boolean;
  /** Read back from the session's saved record: the run is over and its
   *  process is gone, so the row can be looked at but not acted on. */
  restored?: boolean;
  /** A restored row's saved log tail — its log file went with the process. */
  log_tail?: string;
}

/** Why a job ended, as the host's monitor decided it. */
export type BgEndReason = "exited" | "signaled" | "killed" | "timeout" | "log_cap" | "error";

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
  /** Real model runs (assistant rounds) in the whole session — not user
   *  prompts. Absent on old snapshots; the header falls back to loaded
   *  assistant rows. */
  runTotal?: number;
  /** Real tool/task steps in the whole session. */
  taskTotal?: number;
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
    /** Prompt tokens served from the provider cache (cache hits) —
     * `prompt - cachedTokens - cacheWriteTokens` is the uncached share billed
     * at full input rate. */
    cachedTokens: number;
    /** Prompt tokens written to the provider cache this round (Anthropic);
     * 0 when the wire does not report writes. */
    cacheWriteTokens: number;
  };
  /** Estimated composition of the last request's prompt (backend
   * heuristic, `husk:context_breakdown`). Proportions only — the card
   * scales them to `usage.prompt`. Absent until the first live round. */
  contextBreakdown?: ContextBreakdown;
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
  /** Files the session PRODUCED whole — office docs + downloads. Feeds
   *  the office artifacts dock; a new document registration also fires
   *  the ready-toast. */
  artifacts: Artifact[];
  /** Parked follow-up prompts — the kernel's queue (`SetQueued` writes,
   *  `QueuedPrompts` events echo it back, `open` seeds it from
   *  `SessionMeta`). Lives on the view so a session switch preserves it. */
  queuedPrompts: string[];
  /** The session's background jobs — replaced whole by every `husk:bg_jobs`
   *  snapshot, seeded by `bg_list` when a view is rebuilt. `bgJobsAt` is
   *  when the list arrived, the base the running clocks tick from. */
  bgJobs: BgJob[];
  bgJobsAt: number;
}

export const emptyView = (): SessionView => ({
  items: [],
  changes: [],
  artifacts: [],
  queuedPrompts: [],
  bgJobs: [],
  bgJobsAt: 0,
  state: null,
  streaming: false,
  turnOpen: false,
  usage: {
    prompt: 0,
    completion: 0,
    contextWindow: 0,
    cachedTokens: 0,
    cacheWriteTokens: 0,
  },
  toksPerSec: 0,
  rate: { chars: 0, activeMs: 0, lastAt: null },
});

/** Whether the kernel is parked on a tool approval. It pauses in exactly this
 *  state and nowhere else, so it is the authoritative answer to "is anything
 *  waiting for a verdict right now" — a card's own `resolved` flag is not,
 *  because it depends on a `ToolCallFinished` that a cancelled or superseded
 *  call may never emit. */
export function isAwaitingApproval(state: AgentState | null): boolean {
  return typeof state === "object" && state !== null && "AwaitingToolConfirmation" in state;
}

/** The pending approval a composer permission strip should surface —
 *  the most recent unresolved `ApprovalRequested`. `ToolCallStarted`
 *  marks it resolved/approved, `ToolDecision` (deny) resolves via state
 *  transitions. Returns null when nothing is awaiting a verdict.
 *
 *  Gated on the kernel state, not just on the card: once the turn leaves
 *  `AwaitingToolConfirmation` nothing is waiting, and a leftover card would
 *  send a verdict for a request the engine has already left behind. The
 *  engine discards such a stale id by design, which would strand the approval
 *  that IS pending until the 300 s fail-closed timeout. `null` state is a
 *  rebuilt historical view, which has no live request to respect. */
export function pendingApprovalOf(view: SessionView): {
  requestId: number;
  toolName: string;
  args?: string;
  diff: string;
  /** Grant buttons this card offers, if the host asked a question that can
   *  be answered by one (associating a directory). */
  actions?: { id: string; label: string; description?: string }[];
} | null {
  if (view.state !== null && !isAwaitingApproval(view.state)) return null;
  for (let i = view.items.length - 1; i >= 0; i--) {
    const it = view.items[i];
    if (it.kind === "tool" && it.approval && !it.approval.resolved) {
      return {
        requestId: it.approval.requestId,
        toolName: it.name,
        args: it.args,
        diff: it.approval.diff || "",
        actions: it.approval.actions,
      };
    }
    if (it.kind === "approval" && !it.resolved) {
      return {
        requestId: it.requestId,
        toolName: it.toolName,
        diff: it.diff,
        actions: it.actions,
      };
    }
  }
  return null;
}
