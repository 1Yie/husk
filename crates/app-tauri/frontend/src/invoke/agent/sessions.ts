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
  active_permission_mode?: string;
  config_path?: string;
  models: ModelItem[];
}

export function getModelInfo() {
  return invoke<ModelInfo>("agent_session", { op: "model_info" });
}

export interface DefaultPrefs {
  permission_mode?: string;
  thinking_level?: string;
}

/** The stored default preferences — what NEWLY created sessions spawn with.
 * Distinct from `model_info`'s `active_*` fields, which report the live
 * session's actual mode/level. */
export function getDefaultPrefs() {
  return invoke<DefaultPrefs>("agent_session", { op: "get_default_prefs" });
}

/** Update the default preferences. Only affects sessions created AFTER this
 * call — already-started sessions keep their current mode/level. */
export function setDefaultPrefs(prefs: DefaultPrefs) {
  return invoke<{ success: boolean }>("agent_session", {
    op: "set_default_prefs",
    payload: prefs,
  });
}

export function openConfigFile() {
  return invoke<{ path: string }>("agent_session", { op: "open_config" });
}

export interface AppConfigData {
  path?: string;
  raw: string;
  config: Record<string, any>;
  permission_mode?: string;
  thinking_level?: string;
}

export interface SandboxInfo {
  backend: string;
  tier: string;
  is_fallback: boolean;
}

export function openSettingsWindow() {
  return invoke<{ success: boolean }>("agent_session", { op: "open_settings_window" });
}

export function getAppConfig() {
  return invoke<AppConfigData>("agent_session", { op: "get_app_config" });
}

export function saveAppConfig(payload: any) {
  return invoke<{ success: boolean }>("agent_session", { op: "save_app_config", payload });
}

export function getSandboxInfo() {
  return invoke<SandboxInfo>("agent_session", { op: "get_sandbox_info" });
}

/** A workspace file entry for the `@` mention picker. */
export interface FileItem {
  path: string;
  name: string;
}

/** A workspace skill (`.agents/skills/<name>/SKILL.md`) for the `/$` picker. */
export interface SkillItem {
  name: string;
  description: string;
  path: string;
}

/** Fuzzy file index for the `@` picker — `query` is a lowercase substring. */
export function listFiles(query?: string) {
  return invoke<FileItem[]>("agent_session", {
    op: "list_files",
    payload: { query: query ?? "" },
  });
}

/** Workspace skills — `.agents/skills/` + aliases, for `/` and `$` pickers. */
export function listSkills() {
  return invoke<SkillItem[]>("agent_session", { op: "list_skills" });
}
