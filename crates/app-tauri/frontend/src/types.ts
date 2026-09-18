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
  | { Usage: { prompt_tokens: number; completion_tokens: number } }
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

/** The `agent://event` envelope — session id + the event payload. */
export interface AgentEventEnvelope {
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
}
