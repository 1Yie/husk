// Agent session ops — `agent_session` invokes (sidebar list/new/open).

import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ProjectOverview, SessionRow } from "@/types";

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
  /** Prompt tokens served from the provider cache (absent on older rows). */
  cached?: number;
}

/** Switch to an existing session — returns the active id plus the
 * persisted history so the caller can rebuild the stream view when it
 * has no in-memory events for that session yet. */
export async function openSession(id: number) {
  const r = await invoke<{ active: number; history: ChatMessage[]; history_total?: number;
  turn_total?: number; turn_offset?: number; usage?: SessionUsage | null;
  queued_prompts?: string[] }>(
    "agent_session",
    { op: "open", id },
  );
  return r;
}

/** Older-history page — `before` is the exclusive end index (the current
 * `loadedStart`); returns the slice just above it plus the full length. */
/** Full persisted history for the raw-JSON viewer — the complete wire
 * record (roles, tool_calls params, tool results, notices, ts). */
export async function historyRaw(id: number) {
  const r = await invoke<{ history: ChatMessage[] }>(
    "agent_session",
    { op: "history_raw", id },
  );
  return r.history;
}

export async function historyPage(id: number, before: number) {
  return invoke<{ history: ChatMessage[]; history_total: number; turn_total?: number; turn_offset?: number }>(
    "agent_session",
    { op: "history_page", id, payload: { before } },
  );
}

/** Duplicate a session's history into a fresh session (which becomes the
 * active one) — returns the new id + the copied history. */
export async function forkSession(id: number) {
  return invoke<{ active: number; id: number; history: ChatMessage[]; history_total?: number;
  turn_total?: number; turn_offset?: number; usage?: SessionUsage | null;
  queued_prompts?: string[] }>(
    "agent_session",
    { op: "fork", id },
  );
}

/** Delete a session entirely. Returns the new active id + its history —
 * the backend auto-switches when the deleted session was on screen. */
export async function deleteSession(id: number) {
  return invoke<{ active: number; history: ChatMessage[]; history_total?: number;
  turn_total?: number; turn_offset?: number; usage?: SessionUsage | null;
  queued_prompts?: string[] }>(
    "agent_session",
    { op: "delete", id },
  );
}

/** Toggle a session's pinned flag — pinned rows float to the top of the
 * sidebar ahead of recency order. */
export async function pinSession(id: number, pinned: boolean) {
  return invoke<{ pinned: boolean }>("agent_session", {
    op: "pin",
    id,
    payload: { pinned },
  });
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
  history_total?: number;
  turn_total?: number; turn_offset?: number;
  usage?: SessionUsage | null;
  queued_prompts?: string[];
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

/** Drop a project from recents; running turns die with it. */
export function removeWorkspace(path: string) {
  return invoke<{ removed_active: boolean }>("agent_session", { op: "remove_workspace", path });
}

export interface ModelItem {
  provider: string;
  model: string;
  name?: string;
  reasoning?: boolean;
  thinking_level_map?: Record<string, string | null>;
  available_levels?: string[];
  context_window?: number;
  /** $/1M tokens — from the model's config `cost` block. Absent when the
   * model carries no pricing; the header cost chip hides itself then. */
  cost?: {
    input?: number;
    output?: number;
    cache_read?: number;
    cache_write?: number;
  };
}

/** Broadcast after any config write (model list, prefs, instructions) so
 * mounted views re-fetch — without it a settings save only surfaced on
 * remount (the stale model-picker bug). */
export const CONFIG_CHANGED_EVENT = "husk:config-changed";

export function notifyConfigChanged() {
  window.dispatchEvent(new CustomEvent(CONFIG_CHANGED_EVENT));
}

export interface ModelInfo {
  active_provider: string;
  active_model: string;
  active_thinking_level?: string;
  active_permission_mode?: string;
  /** `build` | `plan` | `goal` — the composer's agent-mode picker. */
  active_agent_mode?: string;
  config_path?: string;
  models: ModelItem[];
}

export function getModelInfo() {
  return invoke<ModelInfo>("agent_session", { op: "model_info" });
}

/** A discovered MCP/plugin manifest (listing only — the server is never
 * spawned for the settings display). */
export interface PluginItem {
  id: string;
  name: string;
  /** Manifest version — "" for servers added through the UI (a remote server
   *  has no such version; the live one is `serverVersion`). */
  version: string;
  kind: string;
  entry: {
    command?: string;
    args?: string[];
    url?: string;
    /** Present for HTTP servers added with custom headers. */
    headers?: Record<string, string>;
  };
  /** Tool names from the connection the app loaded at BOOT — reading this does
   *  not reconnect. Empty when the plugin is not loaded. */
  tools: string[];
  commands: number;
  sandboxed: boolean;
  dir: string;
  /** Registered at boot. `false` means it failed to load (`error` explains),
   *  or the app started before the manifest existed. */
  connected: boolean;
  serverVersion?: string;
  error?: string;
}

/** One subagent type the kernel can spawn. */
export interface SubagentItem {
  name: string;
  description: string;
  prompt: string;
  timeout_secs?: number;
  max_tool_rounds?: number;
  max_depth?: number;
  /** Builtin kernel delegate vs a `*.md` manifest under the agent dirs. */
  builtin: boolean;
  /** Manifest path — custom agents only. */
  path?: string;
  global?: boolean;
}

/** The agent's configuration surfaces for the settings "智能体" pane —
 * read-only: system-prompt template, live model info, skills, MCP
 * plugins, and subagent specs. */
export interface AgentOverview {
  instructions: string;
  model: ModelInfo;
  skills: SkillItem[];
  plugins: PluginItem[];
  subagents: SubagentItem[];
}

export function getAgentOverview() {
  return invoke<AgentOverview>("agent_session", { op: "agent_overview" });
}

/** One instructions file (AGENTS.md) the session prompt appends verbatim. */
export interface InstructionsFile {
  path: string | null;
  content: string;
}
export interface InstructionsSet {
  global: InstructionsFile;
  workspace: InstructionsFile;
}
export function getInstructions() {
  return invoke<InstructionsSet>("agent_session", { op: "get_instructions" });
}
export function setInstructions(scope: "global" | "workspace", content: string) {
  return invoke<{ success: boolean }>("agent_session", {
    op: "set_instructions",
    payload: { scope, content },
  });
}

export function createSkill(p: {
  name: string;
  description: string;
  scope: "workspace" | "global";
  body: string;
}) {
  return invoke<{ success: boolean; path: string }>("agent_session", {
    op: "create_skill",
    payload: p,
  });
}

/** One session's last recorded usage — the raw material for the 统计 pane.
 *  Bucketing by day is the frontend's job: only it knows the reader's timezone. */
export interface UsageRecord {
  session: number;
  title: string;
  workspace: string;
  /** Unix seconds of the session's last activity. */
  ts: number;
  prompt: number;
  completion: number;
  cached: number;
  context_window: number;
  model?: string | null;
  provider?: string | null;
}

export function getUsageStats() {
  return invoke<{ sessions: UsageRecord[] }>("agent_session", { op: "usage_stats" });
}

export function addMcp(p: {
  id: string;
  name: string;
  transport: "stdio" | "http";
  command?: string;
  args?: string[];
  url?: string;
  headers?: Record<string, string>;
}) {
  return invoke<{ success: boolean; path: string }>("agent_session", {
    op: "add_mcp",
    payload: p,
  });
}

/** Live answer from `mcp_probe` — the server's own name/version + tool names. */
export interface McpProbe {
  ok: boolean;
  serverVersion?: string;
  tools?: string[];
  error?: string;
}

/** Connect one MCP server on demand and report what it actually serves. The
 *  stored manifest says nothing useful (version "1.0.0", no tool list). */
export function probeMcp(id: string) {
  return invoke<McpProbe>("mcp_probe", { id });
}

/** Drop a plugin directory — the settings card's delete action. */
/** Re-read the plugin dirs and swap the live router. Sessions share the handle
 *  the engine reads per request, so a server added in settings reaches the agent
 *  on the next turn — no restart. */
export function reloadMcp() {
  return invoke<{ plugins: unknown[] }>("agent_session", { op: "reload_mcp" });
}

export function removeMcp(id: string) {
  return invoke<{ success: boolean }>("agent_session", { op: "remove_mcp", payload: { id } });
}

export function createSubagent(p: {
  name: string;
  description: string;
  scope: "workspace" | "global";
  prompt: string;
}) {
  return invoke<{ success: boolean; path: string }>("agent_session", {
    op: "create_subagent",
    payload: p,
  });
}

export interface DefaultPrefs {
  permission_mode?: string;
  thinking_level?: string;
  agent_mode?: string;
  /** Fraction of the context window that triggers compaction — 0.7/0.8/0.9. */
  compact_at?: number;
  /** Sandbox network override — "auto" (per-command audit) | "allow" | "deny". */
  sandbox_network?: "auto" | "allow" | "deny";
  /** Per-command memory cap (MB) — null/absent → the audit plan's 2048. */
  sandbox_max_memory_mb?: number | null;
  /** Fork-bomb process cap — null/absent → the audit plan's 256. */
  sandbox_max_processes?: number | null;
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
  }).then((r) => {
    // Prefs shape the composer's mode/level pickers too — same refresh
    // trigger as a full config save.
    notifyConfigChanged();
    return r;
  });
}

/** Open an external link in the system browser (xdg-open / open / start). */
export function openUrl(url: string) {
  return invoke<{ success: boolean }>("agent_session", {
    op: "open_url",
    payload: { url },
  });
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

export function getAppConfig() {
  return invoke<AppConfigData>("agent_session", { op: "get_app_config" });
}

export function saveAppConfig(payload: any) {
  return invoke<{ success: boolean }>("agent_session", { op: "save_app_config", payload })
    .then((r) => {
      notifyConfigChanged();
      return r;
    });
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

/** Appearance settings — persisted at the app-data root as
 * `appearance.json`. The frontend applies it at boot (dark class + CSS
 * vars) so the shell paints with the user's choices from the first frame. */
export interface AppearanceConfig {
  theme_mode: "system" | "light" | "dark";
  theme_id?: string | null;
  accent: string;
  background: string;
  foreground: string;
  /** Dark-mode overrides — applied instead of the base values while the
   *  effective mode is dark; absent → the `.dark` palette defaults win. */
  dark_accent?: string | null;
  dark_background?: string | null;
  dark_foreground?: string | null;
  ui_font: string;
  code_font: string;
  contrast: number;
  /** Cost display currency — "usd" | "cny". Absent in older files → usd. */
  currency?: "usd" | "cny";
}

export function getAppearance() {
  return invoke<AppearanceConfig>("agent_session", { op: "get_appearance" });
}

/** Partial update — only the keys sent are merged into the stored file. */
export function setAppearance(patch: Partial<AppearanceConfig>) {
  return invoke<{ success: boolean }>("agent_session", {
    op: "set_appearance",
    payload: patch,
  });
}

/** A workspace file entry for the `@` mention picker. */
export interface FileItem {
  path: string;
  name: string;
}

/** A workspace skill (`.agents/skills/<name>/SKILL.md`) for the `/$` picker. */
export interface SkillDetail {
  name: string;
  description: string;
  body: string;
  scope: "workspace" | "global";
  writable: boolean;
}

/** `SKILL.md` body + scope for the edit dialog. */
export function readSkill(name: string) {
  return invoke<SkillDetail>("agent_session", { op: "read_skill", payload: { name } });
}

export function deleteSkill(name: string) {
  return invoke<{ success: boolean }>("agent_session", { op: "delete_skill", payload: { name } });
}

export interface SubagentDetail {
  name: string;
  description: string;
  prompt: string;
  scope: "workspace" | "global";
}

/** `*.md` body + scope for the edit dialog. */
export function readSubagent(name: string) {
  return invoke<SubagentDetail>("agent_session", { op: "read_subagent", payload: { name } });
}

export function deleteSubagent(name: string) {
  return invoke<{ success: boolean }>("agent_session", {
    op: "delete_subagent",
    payload: { name },
  });
}

export interface SkillItem {
  name: string;
  description: string;
  path: string;
  /** Lives in a global skills dir rather than the workspace. */
  global: boolean;
  /** False for skills outside the two writable pi roots (installed packs). */
  writable: boolean;
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

/** Stage clipboard bytes as an attachment. A pasted image has no path to
 * hand over, so the webview sends the payload (`data` = base64, no data:
 * prefix) and the backend writes it into `.husk/attachments/` exactly like
 * a picked file — same `Attachment` shape back, so both flows feed one chip
 * list. `mime` supplies the extension when the pasted blob has no name. */
export function attachBytes(name: string, mime: string, data: string) {
  return invoke<Attachment>("agent_session", {
    op: "attach_bytes",
    payload: { name, mime, data },
  });
}

/** Write a frontend-made export (diagram SVG/PNG, table CSV, image bytes) to a
 *  path the user picks. `data` is base64 — the same envelope `attachBytes`
 *  sends. Exports streamdown builds from a blob reach the filesystem only
 *  through here: the webview's own `<a download>` has no handler under Tauri
 *  (see `lib/download-bridge.ts`). */
export function saveDownload(name: string, data: string) {
  return invoke<{ saved: boolean; path?: string; bytes?: number }>("agent_session", {
    op: "save_download",
    payload: { name, data },
  });
}
