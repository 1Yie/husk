// Agent session ops — `agent_session` invokes (sidebar list/new/open).

import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ProjectOverview, SessionRow } from "../../types";

export function listSessions() {
  return invoke<SessionRow[]>("agent_session", { op: "list" });
}

/** Sidebar project tree — every recent workspace with its full conversation
 * list; the active workspace is first and carries the live running flags. */
export function listProjects() {
  return invoke<ProjectOverview[]>("agent_session", { op: "projects" });
}

export async function newSession() {
  const r = await invoke<{ active: number }>("agent_session", { op: "new" });
  return r.active;
}

/** Persisted last-turn usage — rides along on ops that return history so
 * a rebuilt view seeds the header meter instead of showing zeros. */
export interface SessionUsage {
  prompt: number;
  completion: number;
  context_window: number;
}

/** Switch to an existing session — returns the active id plus the
 * persisted history so the caller can rebuild the stream view when it
 * has no in-memory events for that session yet. */
export async function openSession(id: number) {
  const r = await invoke<{ active: number; history: ChatMessage[]; usage?: SessionUsage | null }>(
    "agent_session",
    { op: "open", id },
  );
  return r;
}

/** Duplicate a session's history into a fresh session (which becomes the
 * active one) — returns the new id + the copied history. */
export async function forkSession(id: number) {
  return invoke<{ active: number; id: number; history: ChatMessage[]; usage?: SessionUsage | null }>(
    "agent_session",
    { op: "fork", id },
  );
}

/** Delete a session entirely. Returns the new active id + its history —
 * the backend auto-switches when the deleted session was on screen. */
export async function deleteSession(id: number) {
  return invoke<{ active: number; history: ChatMessage[]; usage?: SessionUsage | null }>(
    "agent_session",
    { op: "delete", id },
  );
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
  usage?: SessionUsage | null;
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
  context_window?: number;
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

/** Git branch + dirty count for the active workspace — `branch` is null
 * when the workspace isn't inside a repository. */
export interface GitInfo {
  branch: string | null;
  dirty: number;
}

export function getGitInfo() {
  return invoke<GitInfo>("agent_session", { op: "git_info" });
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

/** One attached file for the composer chips — `kind` decides how the
 * prompt payload inlines it: `text` carries `content` (32KB-capped),
 * `image` rides the wire as a real image part when the model declares
 * vision, `binary` stays a path reference. `path` is the staged copy
 * inside `.husk/attachments/` — sandbox-visible; `data_url` is the
 * image's inline preview (chips + user bubble). */
export interface Attachment {
  path: string;
  name: string;
  kind: "text" | "image" | "binary";
  content?: string;
  truncated?: boolean;
  data_url?: string;
}

/** Native multi-select file picker for the `+` attach button —
 * `kind` presets the filter list. */
export function pickAttachments(kind: "image" | "text" | "any") {
  return invoke<string[]>("agent_session", {
    op: "pick_attachments",
    payload: { kind },
  });
}

export function readAttachment(path: string) {
  return invoke<Attachment>("agent_session", {
    op: "read_attachment",
    payload: { path },
  });
}
