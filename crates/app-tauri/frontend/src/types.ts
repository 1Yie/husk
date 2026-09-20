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
export type UiEvent =
  | { StateChanged: AgentState }
  | { UserPrompt: string }
  | { TextDelta: string }
  | { ReasoningDelta: string }
  | { ToolCallStarted: { name: string; args_preview: string } }
  | {
      ToolCallFinished: {
        name: string;
        ok: boolean;
        content: string;
        ui_type?: string;
      };
    }
  | {
      ApprovalRequested: {
        request_id: number;
        tool_name: string;
        diff: string;
        fuzzy: boolean;
      };
    }
  | { AssistantMessage: string }
  | { SystemMessage: string }
  | { Usage: { prompt_tokens: number; completion_tokens: number; context_window: number } }
  | { Error: string };

/** UI → kernel commands — mirrors `UiCommand`. */
export type UiCommand =
  | { Prompt: { text: string } }
  | { Steer: { text: string } }
  | { ToolDecision: { request_id: number; approved: boolean } }
  | "Cancel"
  | { SetModel: { provider: string; model: string } }
  | { SetPermissionMode: { mode: string } }
  | { SetThinkingLevel: { level: string } }
  | "UndoLastTurn";

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
  /** Every persisted conversation, newest first. */
  sessions: ProjectSessionRow[];
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
   * renders as a plain line, `error` with a `⚠` prefix. Absent means
   * internal context (system prompt, compaction note) — stays hidden. */
  notice?: "system" | "error" | "hidden";
  /** Creation time (epoch ms) — drives the `—— time ——` turn divider. */
  ts?: number | null;
}
