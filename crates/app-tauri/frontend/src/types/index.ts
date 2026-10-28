// Wire types — mirror `agent-ipc/src/events.rs` (serde JSON).
// The Rust side emits `{ session, event }` envelopes on `agent://event`;
// commands go back through the `agent_cmd` / `agent_session` invokes.

/** Kernel state machine — mirrors `AgentState`. */
export type AgentState =
  | "Idle"
  | "ScanningWorkspace"
  | "Reasoning"
  | "StreamingToken"
  | { AwaitingToolConfirmation: { tool_name: string; diff_summary: string } }
  | { AwaitingPluginConsent: { plugin_id: string; capability: string } }
  | { AwaitingConsent: { kind: string } }
  | { Branching: { candidates: number } }
  | { ExecutingTool: { tool_name: string } }
  | "Compacting"
  | "Finished"
  | { Failed: string };

/** Kernel → UI events — mirrors `UiEvent`. */
/** One offered answer on an `ask_question` card. */
export interface AskOption {
  label: string;
  description?: string;
}

/** The `submit_plan` contract payload — one step of the plan. */
export interface PlanStep {
  title: string;
  detail?: string;
  files?: string[];
  /** `.blueprint/` nodes the step changes. */
  nodes?: string[];
}

/** The structured plan the plan-mode turn delivers via `submit_plan`. */
export interface PlanPayload {
  summary: string;
  steps: PlanStep[];
  verification?: string[];
  risks?: string[];
  /** Proposed `.blueprint/` node-file changes — applied before any code. */
  architecture?: string;
}

/** Product events the kernel forwards untouched (`UiEvent::Custom`). */
export type CustomEvent =
  | { kind: "plan_submitted"; payload: { plan: string } }
  | { kind: "blueprint_report"; payload: { errors: number; warnings: number; text: string } }
  | { kind: string; payload: unknown };

export type UiEvent =
  | { StateChanged: AgentState }
  | {
      QuestionAsked: {
        request_id: number;
        question: string;
        options: AskOption[];
      };
    }
  | { QuestionAnswered: { request_id: number } }
  /** The card sat unanswered past the configured bound — the agent decides
   *  the rest of the turn on its own. */
  | { QuestionTimedOut: { request_id: number } }
  /** The approval card expired — the kernel auto-denied it and the tool call
   *  fails closed. */
  | { ApprovalTimedOut: { request_id: number; child_id?: number | null } }
  | { UserPrompt: string }
  | { UserSteer: string }
  | { TextDelta: { text: string; parent?: string | null; child_id?: number | null } }
  | { ReasoningDelta: { text: string; parent?: string | null; child_id?: number | null } }
  | {
      ToolCallStarted: {
        name: string;
        args_preview: string;
        parent?: string | null;
        child_id?: number | null;
      };
    }
  | {
      ToolCallFinished: {
        name: string;
        ok: boolean;
        content: string;
        ui_type?: string;
        parent?: string | null;
        child_id?: number | null;
      };
    }
  | {
      ApprovalRequested: {
        request_id: number;
        tool_name: string;
        diff: string;
        fuzzy: boolean;
        /** Alternatives to 允许/拒绝 — a grant the host offers when a tool
         *  refused a path. Empty or absent means an ordinary card. */
        actions?: ApprovalAction[];
        /** The delegated child that asked, when the card belongs to one. It
         *  drew on this stream, so the card is drawn on that child's row. */
        child_id?: number | null;
      };
    }
  | { AssistantMessage: string }
  | { SessionToolWhitelisted: { tool_name: string; stopped?: boolean } }
  | { SystemMessage: string }
  | { Usage: { prompt_tokens: number; completion_tokens: number; context_window: number; cached_tokens: number } }
  | { Error: string }
  | {
      Compacted: {
        before_tokens: number;
        after_tokens: number;
        removed_messages: number;
        context_window: number;
        manual: boolean;
        note: string;
      };
    }
  | { CompactionStarted: { manual: boolean } }
  | { Custom: CustomEvent }
  | "TurnRetry"
  | { QueuedPrompts: { items: string[] } };

/** UI → kernel commands — mirrors `UiCommand`. */
export type UiCommand =
  | { Prompt: { text: string } }
  | { Steer: { text: string } }
  | { SteerChild: { child_id: number; text: string } }
  | {
      ToolDecision: {
        request_id: number;
        approved: boolean;
        /** A grant the card offered (`ApprovalRequested.actions`). */
        action?: string;
      };
    }
  | { ApproveSessionTool: { tool_name: string } }
  | { RevokeSessionTool: { tool_name: string } }
  | { AnswerQuestion: { request_id: number; answer: string } }
  | "Cancel"
  | { SetModel: { provider: string; model: string } }
  | { SetPermissionMode: { mode: string } }
  | { SetAgentMode: { mode: string } }
  | { SetThinkingLevel: { level: string } }
  | "UndoLastTurn"
  | "Retry"
  | { SetQueued: { items: string[] } }
  | { Enqueue: { text: string } };

/** The `agent://event` envelope — owning workspace root + session id +
 * the event payload. Session ids are per-workspace; `root` keeps a
 * parked workspace's still-running actors from colliding with a
 * same-numbered session in the active one. */
export interface AgentEventEnvelope {
  /** Canonical root of the workspace that spawned the actor — captured
   * at spawn, so a background workspace's events keep their identity
   * after a workspace switch. */
  root?: string;
  session: number;
  event: UiEvent;
}

/** A sidebar row — mirrors `SessionManager::sidebar_rows`. */
export interface SessionRow {
  id: number;
  title: string;
  preview: string;
  active: boolean;
  running: boolean;
  /** Pinned within its workspace — leads the project's list. */
  pinned: boolean;
  /** Pinned globally — leads the cross-project 会话 list. */
  pinned_global?: boolean;
  /** Archived — listed only under 已归档. */
  archived?: boolean;
}

/** A session row inside a project — `SessionRow` plus the timestamp the
 * sidebar's cross-project "recent" list merges on. Ids are per-workspace,
 * so callers key rows by `root` + `id`. */
export interface ProjectSessionRow extends SessionRow {
  updated_at: number;
}

/** One project (workspace) with its full conversation list — mirrors
 * `SessionManager::projects_overview`. */
export interface ProjectOverview {
  root: string;
  name: string;
  last_opened: number;
  /** This is the kernel's active workspace. */
  current: boolean;
  /** The built-in office pseudo-workspace — rendered as the sidebar's
   *  fixed「工作」section rather than a regular project row. */
  office?: boolean;
  /** Every persisted conversation, newest first. */
  sessions: ProjectSessionRow[];
}

/** Sidebar metadata is independent of loaded pages (and of chat history paging). */
export interface SidebarProject extends Omit<ProjectOverview, "sessions"> {
  /** Non-archived total, including rows not yet requested. */
  total: number;
  archived_total: number;
  /** Includes parked/off-page actors, not inferred from the current page. */
  running?: boolean;
}
export interface SidebarEntry {
  root: string;
  project: string;
  current: boolean;
  row: ProjectSessionRow;
}
export interface SidebarSummary {
  generation: string;
  projects: SidebarProject[];
  /** All global pins, followed by the six newest other non-archived rows. */
  recents: SidebarEntry[];
  active: SidebarEntry | null;
}
export interface SidebarPage {
  generation: string;
  root: string;
  archived: boolean;
  rows: ProjectSessionRow[];
  next_cursor: string | null;
}

/** Persisted message — mirrors `agent_llm::types::ChatMessage` on the wire.
 * `content` serializes as a plain string (None → ""), `tool_calls` uses the
 * OpenAI wire shape `{id, type:"function", function:{name, arguments}}`. */
export interface ChatMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  tool_calls?: {
    id: string;
    type: "function";
    function: { name: string; arguments: string };
  }[];
  tool_call_id?: string;
  /** Replay-only flag persisted on failed tool results — never sent to
   * the provider (the adapter strips it). */
  is_error?: boolean;
  /** Set on `system` entries the live stream showed the user — `system`
   * renders as a plain line, `error` with a `⚠` prefix. `compacted`
   * carries the JSON card payload (`before/after/removed/note`), and
   * `compacted_memory` is the model-facing summary note (hidden), and so is
   * `host` — a host notice (a background job finishing) whose event the UI
   * shows through its own channel (`husk:bg_jobs`).
   * `steer` is a mid-turn user correction: drawn as a user bubble inside the
   * running turn, never as a turn of its own.
   * Absent means internal context (system prompt, hook injections) —
   * stays hidden. */
  notice?: "system" | "error" | "hidden" | "compacted_memory" | "compacted" | "plan" | "steer" | "host";
  /** Creation time (epoch ms) — drives the `—— time ——` turn divider. */
  ts?: number | null;
  /** Reasoning trace of this assistant round — replays as the same 思考过程
   * block the live stream drew. Display-only: never sent to a provider.
   * Absent on rounds without a trace and on sessions written before this
   * was persisted. */
  reasoning?: string | null;
  /** Wall-clock duration (ms) of this message's activity — a tool result
   * carries the call's dispatch span; an assistant row carries its reasoning
   * span. Lets a replayed view re-render the `· 20s` labels the live stream
   * drew. Absent on pre-existing snapshots. */
  duration_ms?: number | null;
}

/** Kernel UI-event queue counters (`agent_kernel::channels::UiStatsSnapshot`)
 * aggregated over every live session. `dropped` counts *text* deltas the
 * queue had to discard — control events (state machine, approvals, final
 * messages) are never dropped. */
export interface UiStats {
  sent: number;
  coalesced: number;
  dropped: number;
  depth_peak: number;
}

/** One thing the user may do instead of a plain yes/no — a grant the host
 *  offers when a tool refused a path (associate another directory). `id` is
 *  opaque here: it goes back out in `ToolDecision` and the host interprets it. */
export interface ApprovalAction {
  id: string;
  label: string;
  description?: string;
}

/** The same list where a stream item keeps it. */
export type ApprovalActions = ApprovalAction[];
