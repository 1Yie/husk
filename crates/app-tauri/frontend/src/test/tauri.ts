import { vi } from "vitest";
import type { AgentEventEnvelope } from "@/types";
import type { ModelInfo } from "@/lib/agent-ipc/sessions";

type Args = Record<string, unknown> | undefined;
type Handler = (args: Args) => unknown | Promise<unknown>;
type EventHandler = (event: { event: string; id: number; payload: unknown }) => void;
const handlers = new Map<string, Handler>();
const listeners = new Map<string, Set<EventHandler>>();
const focusHandlers = new Set<(e: { payload: boolean }) => void>();
const key = (command: string, operation?: string) => operation ? `${command}:${operation}` : command;

export const modelInfo = (): ModelInfo => ({
  active_provider: "test", active_model: "model-a", active_thinking_level: "low",
  active_permission_mode: "default", active_agent_mode: "build",
  models: [
    { provider: "test", model: "model-a", name: "Model A", reasoning: true,
      available_levels: ["off", "low", "high", "max"],
      thinking_level_map: { low: "low", high: "high", max: "max" }, context_window: 128000 },
    { provider: "test", model: "model-b", name: "Model B", reasoning: true,
      available_levels: ["off", "medium", "xhigh"],
      thinking_level_map: { medium: "medium", xhigh: "xhigh" }, context_window: 256000 },
  ],
});

export const tauri = {
  invoke: vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    const handler = handlers.get(key(command, typeof args?.op === "string" ? args.op : undefined))
      ?? handlers.get(command);
    if (!handler) throw new Error(`Unhandled Tauri command in test: ${command} ${JSON.stringify(args)}`);
    return handler(args);
  }),
  listen: vi.fn(async (event: string, handler: EventHandler) => {
    let group = listeners.get(event);
    if (!group) listeners.set(event, group = new Set());
    group.add(handler);
    return () => group.delete(handler);
  }),
  window: {
    startDragging: vi.fn(async () => {}), toggleMaximize: vi.fn(async () => {}),
    minimize: vi.fn(async () => {}), close: vi.fn(async () => {}),
    isMaximized: vi.fn(async () => false), onResized: vi.fn(async () => () => {}),
    onFocusChanged: vi.fn(async (handler: (e: { payload: boolean }) => void) => {
      focusHandlers.add(handler);
      return () => focusHandlers.delete(handler);
    }),
  },
};

/** Deliver the shell's native focus report — what `onFocusChanged` hears. */
export function emitWindowFocus(focused: boolean) {
  for (const handler of focusHandlers) handler({ payload: focused });
}

export function mockCommand(command: string, handler: Handler, operation?: string) {
  handlers.set(key(command, operation), handler);
}
export function emitTauriEvent(event: string, payload: unknown) {
  for (const handler of listeners.get(event) ?? []) handler({ event, id: 1, payload });
}
export function emitAgentEvent(envelope: AgentEventEnvelope) {
  emitTauriEvent("agent://event", envelope);
}
export function listenerCount(event = "agent://event") {
  return listeners.get(event)?.size ?? 0;
}
export function resetTauri() {
  listeners.clear();
  handlers.clear();
  focusHandlers.clear();
  mockCommand("agent_cmd", () => undefined);
  mockCommand("agent_session", () => modelInfo(), "model_info");
  mockCommand("agent_session", () => ({ branch: "main", dirty: 0 }), "git_info");
  mockCommand("agent_session", () => [], "list_files");
  mockCommand("agent_session", () => [], "list_skills");
  mockCommand("agent_session", () => [], "plugin_commands");
  mockCommand("agent_session", () => ({ desktop_notifications: true }), "get_default_prefs");
  mockCommand("agent_session", () => ({ success: true }), "notify");
  mockCommand("agent_session", () => null, "peek_notification_click");
  mockCommand("agent_session", () => null, "ack_notification_click");
  mockCommand("paste_clipboard", () => null);
}

/** Control out-of-order IPC without real timers or network calls. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
