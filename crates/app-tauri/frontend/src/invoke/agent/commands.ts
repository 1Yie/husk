// Agent commands — `UiCommand` invokes (UI → kernel).
//
// Every function maps one semantic operation to one `UiCommand` variant.
// Callers pass plain arguments; the enum wiring stays inside.

import { invoke } from "@tauri-apps/api/core";
import type { UiCommand } from "../../types";

function send(cmd: UiCommand) {
  return invoke("agent_cmd", { cmd });
}

/** A fresh user prompt — starts a turn. */
export const sendPrompt = (text: string) => send({ Prompt: { text } });

/** Mid-turn steering text — injected into the running turn. */
export const steer = (text: string) => send({ Steer: { text } });

/** Resolve a pending approval card. */
export const decideTool = (requestId: number, approved: boolean) =>
  send({ ToolDecision: { request_id: requestId, approved } });

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

/** Rewind the last-turn hunk set. */
export const undoLastTurn = () => send("UndoLastTurn");
