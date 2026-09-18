// Agent session ops — `agent_session` invokes (sidebar list/new/open).

import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, SessionRow } from "../../types";

export function listSessions() {
  return invoke<SessionRow[]>("agent_session", { op: "list" });
}

export async function newSession() {
  const r = await invoke<{ active: number }>("agent_session", { op: "new" });
  return r.active;
}

/** Switch to an existing session — returns the active id plus the
 * persisted history so the caller can rebuild the stream view when it
 * has no in-memory events for that session yet. */
export async function openSession(id: number) {
  const r = await invoke<{ active: number; history: ChatMessage[] }>(
    "agent_session",
    { op: "open", id },
  );
  return r;
}

export interface WorkspaceInfo {
  root: string;
  name: string;
  recents: { path: string; name: string; last_opened: number }[];
}

export interface WorkspaceSwitchResult {
  root: string;
  name: string;
  active: number;
  history: ChatMessage[];
  sessions: SessionRow[];
}

export function getWorkspaceInfo() {
  return invoke<WorkspaceInfo>("agent_session", { op: "workspace_info" });
}

export function pickWorkspace() {
  return invoke<WorkspaceSwitchResult | null>("agent_session", { op: "pick_workspace" });
}

export function switchWorkspace(path: string) {
  return invoke<WorkspaceSwitchResult>("agent_session", { op: "switch_workspace", path });
}

export interface ModelItem {
  provider: string;
  model: string;
  name?: string;
  reasoning?: boolean;
  thinking_level_map?: Record<string, string | null>;
  available_levels?: string[];
}

export interface ModelInfo {
  active_provider: string;
  active_model: string;
  active_thinking_level?: string;
  config_path?: string;
  models: ModelItem[];
}

export function getModelInfo() {
  return invoke<ModelInfo>("agent_session", { op: "model_info" });
}

export function openConfigFile() {
  return invoke<{ path: string }>("agent_session", { op: "open_config" });
}
