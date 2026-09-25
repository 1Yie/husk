// Agent commands — `UiCommand` invokes (UI → kernel).
//
// Every function maps one semantic operation to one `UiCommand` variant.
// Callers pass plain arguments; the enum wiring stays inside.

import { invoke } from "@tauri-apps/api/core";
import type { UiCommand } from "@/types";

function send(cmd: UiCommand) {
  return invoke("agent_cmd", { cmd });
}

/** A fresh user prompt — starts a turn. */
export const sendPrompt = (text: string) => send({ Prompt: { text } });

/** Mid-turn steering text — injected into the running turn. */
export const steer = (text: string) => send({ Steer: { text } });

/** Replace the session's parked queue — every list op (enqueue, edit,
 * remove, reorder) resolves to this one write; the kernel echoes the new
 * list back via `QueuedPrompts`. */
export const setQueued = (items: string[]) => send({ SetQueued: { items } });

/** Park one follow-up prompt — the kernel appends and, if the session is
 * already idle, drains it immediately (closes the "turn ended between the
 * user's keystroke and this command" race). */
export const enqueue = (text: string) => send({ Enqueue: { text } });

/** Resolve a pending approval card. */
export const decideTool = (requestId: number, approved: boolean) =>
  send({ ToolDecision: { request_id: requestId, approved } });

/** Whitelist a tool for the rest of this session — pairs with
 *  `decideTool` on the computer-use approval modal: "本会话允许"
 *  writes the gate AND resolves the pending card. Session-scoped, so
 *  quitting the app revokes it; nothing is persisted. */
export const approveSessionTool = (toolName: string) =>
  send({ ApproveSessionTool: { tool_name: toolName } });

/** Drop a tool from the session whitelist — the overlay's "停止" button.
 *  Session-scoped like the grant; the NEXT `computer` call falls back
 *  to Ask (which re-opens the approval modal). */
export const revokeSessionTool = (toolName: string) =>
  send({ RevokeSessionTool: { tool_name: toolName } });

/** Answer a pending `ask_question` card — option label or free text. */
export const answerQuestion = (requestId: number, answer: string) =>
  send({ AnswerQuestion: { request_id: requestId, answer } });

/** Abort the in-flight turn. */
export const cancelTurn = () => send("Cancel");

/** Hot-swap provider/model for the next turn. */
export const setModel = (provider: string, model: string) =>
  send({ SetModel: { provider, model } });

/** Switch the kernel permission mode (default / acceptEdits / auto /
 * dontAsk / bypassPermissions) — applies to the next tool dispatch. */
export const setPermissionMode = (mode: string) =>
  send({ SetPermissionMode: { mode } });

/** Switch the agent mode (`build` | `plan` | `goal`) — swaps the session's
 * tool registry immediately (plan drops write tools; goal adds the
 * goal_complete contract). */
export const setAgentMode = (mode: string) =>
  send({ SetAgentMode: { mode } });

/** Set thinking / reasoning effort level (e.g. "off", "low", "medium", "high", "max"). */
export const setThinkingLevel = (level: string) =>
  send({ SetThinkingLevel: { level } });

/** Regenerate the last turn — the kernel rewinds history to the last
 * user prompt and re-runs it (slash intercept + hooks included). */
export const retryTurn = () => send("Retry");

/** Kernel UI-event queue counters — one `get_ui_stats` read for the status
 * chip. Cheap: atomics on the Rust side, no locking of session actors. */
export function getUiStats() {
  return invoke<import("@/types").UiStats>("get_ui_stats");
}

/** Stage the *system* clipboard's image as an attachment — the same chip shape
 * as a picked file. `null` when the clipboard holds no image, which is not an
 * error: the caller then leaves the browser's own text paste alone.
 *
 * The webview's `paste` event is not a dependable image carrier on WebKitGTK
 * (a `<textarea>` paste can deliver no `clipboardData`), so this reads the
 * clipboard from Rust instead. */
export function pasteClipboardImage() {
  return invoke<import("@/lib/agent-ipc/sessions").Attachment | null>("paste_clipboard", { kind: "image" });
}

/** The clipboard's text, for the composer menu's 粘贴 item. `null` when empty. */
export function pasteClipboardText() {
  return invoke<{ kind: "text"; text: string } | null>("paste_clipboard", { kind: "text" });
}
