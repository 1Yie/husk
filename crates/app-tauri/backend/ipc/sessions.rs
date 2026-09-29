//! `agent_session` — session-list + switch/create commands for the sidebar.

use base64::Engine as _;
use tauri::State;

use crate::kernel::KernelState;

/// History slice size returned by `open`/`fork`/workspace switches and
/// each `history_page` call — ~5-10 turns worth of stream items, enough
/// for several viewports without mounting the whole session.
const HISTORY_PAGE: usize = 80;

#[tauri::command]
pub async fn agent_session(
    app: tauri::AppHandle,
    state: State<'_, KernelState>,
    op: String,
    id: Option<i64>,
    path: Option<String>,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    // Sync commands run on the main thread — and ops like
    // `set_plugin_enabled`/`reload_mcp` block for up to 8s per plugin on
    // MCP handshakes, freezing the whole UI until they finish. Shunt the
    // entire dispatch onto a blocking thread: the UI stays live while the
    // kernel lock is held, and `Runtime::new().block_on` inside
    // `load_plugins` still works because spawn_blocking threads are not a
    // tokio context.
    let kernel = state.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        agent_session_inner(app, kernel, op, id, path, payload)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn agent_session_inner(
    app: tauri::AppHandle,
    state: std::sync::Arc<std::sync::Mutex<agent_kernel::session_manager::SessionManager>>,
    op: String,
    id: Option<i64>,
    path: Option<String>,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    // Native dialogs (rfd → AppKit `runModal`) run on the MAIN thread via a
    // channel, and never while the kernel mutex is held: the modal loop's
    // nested runloop pumps webview IPC callbacks, and a re-entrant
    // `state.lock()` self-deadlocks the main thread (the 0.1.11 spindump:
    // NSSavePanel runModal → WebKit work → pthread mutex wait).
    if op == "save_download" {
        return save_download(&app, payload);
    }
    if op == "pick_attachments" {
        let kind = payload
            .as_ref()
            .and_then(|p| p.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("any")
            .to_string();
        let picked = dialog_on_main(&app, move || {
            let mut dlg = rfd::FileDialog::new().set_title("添加附件");
            dlg = match kind.as_str() {
                "image" => {
                    dlg.add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"])
                }
                "text" => dlg.add_filter(
                    "文本",
                    &[
                        "txt", "md", "markdown", "json", "yaml", "yml", "toml", "xml", "csv",
                        "log", "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "c", "cc",
                        "cpp", "h", "hpp", "css", "html", "sh", "sql", "ini", "conf", "env",
                    ],
                ),
                _ => dlg,
            };
            dlg.pick_files().unwrap_or_default()
        })
        .unwrap_or_default();
        return Ok(serde_json::json!(picked
            .iter()
            .map(|p: &std::path::PathBuf| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()));
    }
    // The pick itself is the dialog half; only `switch_workspace` and the
    // response reads need the manager — lock after the user answers.
    if op == "pick_workspace" {
        let picked = dialog_on_main(&app, || {
            rfd::FileDialog::new()
                .set_title("Open Workspace Directory")
                .pick_folder()
        })
        .flatten();
        let Some(target) = picked else {
            return Ok(serde_json::json!(null));
        };
        let mut mgr = state.lock().map_err(|e| e.to_string())?;
        mgr.switch_workspace(target).map_err(|e| e.to_string())?;
        let name = mgr
            .workspace_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (history, total, turns, turn_from) =
            mgr.store_history_page(mgr.active_id, None, HISTORY_PAGE);
        return Ok(serde_json::json!({
            "root": mgr.workspace_root.to_string_lossy(),
            "name": name,
            "active": mgr.active_id,
            "history": history,
            "history_total": total,
            "turn_total": turns,
            "turn_offset": turn_from,
            "usage": mgr.store_usage(mgr.active_id),
            "queued_prompts": mgr.store_queued(mgr.active_id),
            "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
                serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
            }).collect::<Vec<_>>(),
        }));
    }
    // Same reason: filesystem work that has nothing to do with the running
    // session must not block behind the kernel lock.
    if op == "remove_mcp" {
        let id = payload
            .as_ref()
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if !valid_slug(&id) {
            return Err("插件 ID 只能是小写字母/数字/-/ _".into());
        }
        let root = {
            let m = state.lock().map_err(|e| e.to_string())?;
            m.workspace_root.clone()
        };
        agent_kernel::session_manager::SessionManager::remove_plugin(&root, &id)?;
        return Ok(serde_json::json!({ "success": true }));
    }
    // Artifact actions for the office dock — open / reveal / save-as on a
    // workspace-relative path the session produced. All three resolve under
    // the CURRENT workspace root (office mode included — the office root IS
    // the active workspace then); `..` and absolute paths are refused.
    if op == "open_path" || op == "reveal_path" {
        let rel = payload
            .as_ref()
            .and_then(|p| p.get("path"))
            .and_then(|v| v.as_str())
            .ok_or(format!("{op} needs payload.path"))?;
        let root = {
            let m = state.lock().map_err(|e| e.to_string())?;
            m.workspace_root.clone()
        };
        let target = resolve_workspace_path(&root, rel)?;
        if op == "reveal_path" {
            // macOS has a real "reveal" — elsewhere the parent dir opens.
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open")
                .args(["-R"])
                .arg(&target)
                .spawn();
            #[cfg(target_os = "linux")]
            {
                let dir = target.parent().unwrap_or(target.as_path()).to_path_buf();
                let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            }
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer")
                .arg("/select,")
                .arg(&target)
                .spawn();
        } else {
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&target).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &target.to_string_lossy()])
                .spawn();
        }
        return Ok(serde_json::json!({ "success": true }));
    }
    // Save-as is a modal dialog → runs through `dialog_on_main`, kernel
    // mutex dropped between the root read and the copy (see the header).
    if op == "save_artifact" {
        let rel = payload
            .as_ref()
            .and_then(|p| p.get("path"))
            .and_then(|v| v.as_str())
            .ok_or("save_artifact needs payload.path")?;
        let root = {
            let m = state.lock().map_err(|e| e.to_string())?;
            m.workspace_root.clone()
        };
        let src = resolve_workspace_path(&root, rel)?;
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "artifact".into());
        let Some(target) = dialog_on_main(&app, move || {
            rfd::FileDialog::new()
                .set_title("另存为")
                .set_file_name(&name)
                .save_file()
        })
        .flatten() else {
            return Ok(serde_json::json!({ "saved": false }));
        };
        std::fs::copy(&src, &target).map_err(|e| format!("save_artifact: {e}"))?;
        return Ok(serde_json::json!({
            "saved": true,
            "path": target.to_string_lossy(),
        }));
    }
    // Existence check for the artifacts dock — entries come from tool args,
    // so a file renamed/deleted afterwards (e.g. `web_download` wrote
    // lemonade.png and bash renamed it to .jpg) would list a ghost row.
    // The panel filters these out; the check is a stat under the same
    // workspace-containment rules as the open/reveal ops.
    if op == "stat_paths" {
        let rels: Vec<String> = payload
            .as_ref()
            .and_then(|p| p.get("paths"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let root = {
            let m = state.lock().map_err(|e| e.to_string())?;
            m.workspace_root.clone()
        };
        let mut out = serde_json::Map::new();
        for rel in rels {
            let exists = resolve_workspace_path(&root, &rel)
                .map(|p| p.is_file())
                .unwrap_or(false);
            out.insert(rel, serde_json::Value::Bool(exists));
        }
        return Ok(serde_json::Value::Object(out));
    }
    //「工作」empty state — the most recent files produced under the office
    // workspace (deliverables + downloaded assets), newest first. Walks the
    // office root directly; no kernel lock needed. Hidden entries
    // (.dashi-ppt-project) and officecli crash logs are noise, not deliverables.
    if op == "recent_artifacts" {
        let mut found: Vec<(String, String, u64)> = Vec::new();
        if let Some(root) = agent_kernel::session_store::office_root() {
            collect_recent_files(&root, &root, 3, &mut found);
        }
        found.sort_by(|a, b| b.2.cmp(&a.2));
        found.truncate(100);
        return Ok(serde_json::Value::Array(
            found
                .into_iter()
                .map(|(path, name, mtime)| {
                    serde_json::json!({ "path": path, "name": name, "mtime": mtime })
                })
                .collect(),
        ));
    }
    let mut mgr = state.lock().map_err(|e| e.to_string())?;
    // Memory pane ops share one store open — `~/.local/share/husk/memory.db`
    // keyed by the active workspace root, same path as the session actor's.
    let open_memory = |mgr: &agent_kernel::session_manager::SessionManager| {
        agent_kernel::session_store::app_data_dir().and_then(|d| {
            agent_context::memory::MemoryStore::open(&d.join("memory.db"), &mgr.workspace_root).ok()
        })
    };
    match op.as_str() {
        "list" => Ok(
            serde_json::json!(mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
            serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
        }).collect::<Vec<_>>()),
        ),
        // Sidebar project tree — every recent workspace with its full
        // conversation list. The active workspace comes first (`current: true`)
        // and carries the live running/active overlay; other projects are
        // store-only snapshots (one workspace has actors at a time).
        "projects" => Ok(serde_json::to_value(mgr.projects_overview()).map_err(|e| e.to_string())?),
        "new" => {
            mgr.new_session();
            Ok(serde_json::json!({"active": mgr.active_id}))
        }
        // `open` also returns the persisted history so the webview can
        // rebuild the stream for a session it has no in-memory view for
        // (first open after launch, or a view that was dropped). The live
        // actor is resumed separately — this is display-only data.
        "open" => {
            let id = id.ok_or("open needs id")?;
            mgr.open_session(id);
            // Paged: only the newest slice mounts — older pages stream in
            // on scroll-up via `history_page`. `history_total` lets the
            // webview compute `loadedStart = total - history.len()`.
            let (history, total, turns, turn_from) = mgr.store_history_page(id, None, HISTORY_PAGE);
            // `usage` is the persisted last-turn meter — seeds the header
            // stats so a reopened session doesn't read 0/… until the next
            // turn's `Usage` event.
            Ok(serde_json::json!({
                "active": mgr.active_id,
                "history": history,
                "history_total": total,
                "turn_total": turns,
                "turn_offset": turn_from,
                "usage": mgr.store_usage(id),
                // The composer's parked queue is session state — a rebuilt
                // view seeds it from here; live updates ride `QueuedPrompts`.
                "queued_prompts": mgr.store_queued(id),
            }))
        }
        // Older-history page: `payload.before` is the exclusive end index
        // (the current loadedStart); returns the slice just above it.
        "history_raw" => {
            // Full persisted history for the raw-JSON viewer — every
            // message exactly as it rides the wire: roles, tool_calls
            // (the model's sends + params), tool results, notices, ts.
            let id = id.ok_or("history_raw needs id")?;
            Ok(serde_json::json!({ "history": mgr.store_history(id).unwrap_or_default() }))
        }
        "history_page" => {
            let id = id.ok_or("history_page needs id")?;
            let before = payload
                .as_ref()
                .and_then(|p| p.get("before"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let (history, total, turns, turn_from) =
                mgr.store_history_page(id, before, HISTORY_PAGE);
            Ok(
                serde_json::json!({ "history": history, "history_total": total, "turn_total": turns, "turn_offset": turn_from }),
            )
        }
        // `delete` mirrors `open` in returning the new active id + its
        // history so the webview can rebuild the stream when the deleted
        // session was the one on screen.
        "delete" => {
            let id = id.ok_or("delete needs id")?;
            mgr.delete_session(id);
            let (history, total, turns, turn_from) =
                mgr.store_history_page(mgr.active_id, None, HISTORY_PAGE);
            let usage = mgr.store_usage(mgr.active_id);
            Ok(
                serde_json::json!({"active": mgr.active_id, "history": history, "history_total": total, "turn_total": turns,
                "turn_offset": turn_from, "usage": usage, "queued_prompts": mgr.store_queued(mgr.active_id)}),
            )
        }
        // `fork` copies the source session's latest snapshot into a new
        // session and activates it — same return shape as `open`.
        "fork" => {
            let id = id.ok_or("fork needs id")?;
            match mgr.fork_session(id) {
                Some(new_id) => {
                    let (history, total, turns, turn_from) =
                        mgr.store_history_page(new_id, None, HISTORY_PAGE);
                    Ok(serde_json::json!({
                            "active": mgr.active_id,
                            "id": new_id,
                            "history": history,
                            "history_total": total,
                            "turn_total": turns,
                    "turn_offset": turn_from,
                            "usage": mgr.store_usage(new_id),
                            "queued_prompts": mgr.store_queued(new_id),
                        }))
                }
                None => Err("session has no history to fork yet".into()),
            }
        }
        // `pin` flips a session's pinned flag in the index — the sidebar
        // re-reads `projects`/`list` after the call, so nothing else needs
        // returning beyond the flag itself.
        "pin" => {
            let id = id.ok_or("pin needs id")?;
            let pinned = payload
                .as_ref()
                .and_then(|p| p.get("pinned"))
                .and_then(|v| v.as_bool())
                .ok_or("pin needs payload.pinned")?;
            let ok = mgr.pin_session(id, pinned).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "pinned": ok }))
        }
        "workspace_info" => {
            let name = if mgr.workspace_active {
                mgr.workspace_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            // 「最近打开」is the fallback for an EMPTY sidebar 项目 list —
            // when the sidebar already lists the recents the grid is a
            // duplicate, and a removed project's store stays on disk so
            // the empty sidebar still has something to reopen.
            let recents = if mgr.recent_workspaces().is_empty() {
                agent_kernel::session_store::known_workspaces()
            } else {
                Vec::new()
            };
            Ok(serde_json::json!({
                "root": if mgr.workspace_active { mgr.workspace_root.to_string_lossy().to_string() } else { String::new() },
                "name": name,
                // The office pseudo-workspace root — the sidebar's「办公」
                // block and the composer compare against it to know when
                // they're rendering office mode.
                "office_root": agent_kernel::session_store::office_root()
                    .map(|p| p.canonicalize().unwrap_or(p).to_string_lossy().to_string())
                    .unwrap_or_default(),
                "recents": recents.iter().map(|w| {
                    serde_json::json!({ "path": w.path.to_string_lossy(), "name": w.name, "last_opened": w.last_opened })
                }).collect::<Vec<_>>()
            }))
        }
        // `removed_active` = the open workspace is gone; show the empty state.
        "remove_workspace" => {
            let p = path.ok_or("remove_workspace needs path")?;
            let was_active = mgr.remove_workspace(&p);
            Ok(serde_json::json!({ "removed_active": was_active }))
        }
        // Header chip: branch + dirty count for the active workspace.
        // Not-a-repo → branch:null (workspace can legitimately be outside git).
        "git_info" => {
            if !mgr.workspace_active {
                return Ok(serde_json::json!({ "branch": serde_json::Value::Null, "dirty": 0 }));
            }
            match agent_context::git::git_snapshot(&mgr.workspace_root) {
                Ok(snap) => Ok(serde_json::json!({
                    "branch": snap.branch,
                    "dirty": snap.dirty_count(),
                })),
                Err(_) => Ok(serde_json::json!({ "branch": serde_json::Value::Null, "dirty": 0 })),
            }
        }
        "switch_workspace" => {
            let p = path.ok_or("switch_workspace needs path")?;
            mgr.switch_workspace(std::path::PathBuf::from(p))
                .map_err(|e| e.to_string())?;
            let name = mgr
                .workspace_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let (history, total, turns, turn_from) =
                mgr.store_history_page(mgr.active_id, None, HISTORY_PAGE);
            Ok(serde_json::json!({
                "root": mgr.workspace_root.to_string_lossy(),
                "name": name,
                "active": mgr.active_id,
                "history": history,
                "history_total": total,
                "turn_total": turns,
                "turn_offset": turn_from,
                "usage": mgr.store_usage(mgr.active_id),
                "queued_prompts": mgr.store_queued(mgr.active_id),
                "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
                    serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
                }).collect::<Vec<_>>(),
            }))
        }
        "model_info" => {
            let info = mgr.model_info();
            Ok(serde_json::to_value(&info).map_err(|e| e.to_string())?)
        }
        // Agent overview — the settings page's read-only display of the
        // agent's configuration surfaces: system-prompt template, live
        // model info, skills, discovered MCP plugins, and the subagent spec.
        // `reload_mcp`/`trust_mcp`/`set_plugin_enabled`: snapshot the shared
        // plugin handles, DROP the manager guard, then run the reload —
        // `load_all` is seconds of MCP handshake I/O per plugin and holding
        // the mutex through it dead-queued every other op behind it.
        "reload_mcp" => {
            let parts = mgr.plugin_reload_parts();
            drop(mgr);
            let plugins = parts
                .as_ref()
                .map(agent_kernel::session_manager::SessionManager::reload_plugins_detached)
                .unwrap_or_default();
            Ok(serde_json::json!({ "plugins": plugins }))
        }
        // `trust_mcp` — record consent for a repo-local plugin, then reload so
        // it loads without an app restart. Its hooks run local commands, so
        // this is the gate that stands between a cloned repo and code exec.
        "trust_mcp" => {
            let id = payload
                .as_ref()
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            if !valid_slug(&id) {
                return Err("插件 ID 只能是小写字母/数字/-/ _".into());
            }
            let parts = mgr.plugin_reload_parts();
            drop(mgr);
            let Some(parts) = parts else {
                return Err("plugin router not installed for this workspace".into());
            };
            Ok(serde_json::json!({
                "plugins": agent_kernel::session_manager::SessionManager::trust_plugin_detached(
                    &parts, &id,
                )?,
            }))
        }
        // `set_plugin_enabled` — persist the toggle (plugin-state.json) and
        // reload: hooks stop firing / tools stop advertising on the next turn.
        "set_plugin_enabled" => {
            let id = payload
                .as_ref()
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let enabled = payload
                .as_ref()
                .and_then(|p| p.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if !valid_slug(&id) {
                return Err("插件 ID 只能是小写字母/数字/-/ _".into());
            }
            let parts = mgr.plugin_reload_parts();
            drop(mgr);
            let Some(parts) = parts else {
                return Err("plugin router not installed for this workspace".into());
            };
            Ok(serde_json::json!({
                "plugins": agent_kernel::session_manager::SessionManager::set_plugin_enabled_detached(
                    &parts, &id, enabled,
                )?,
            }))
        }
        // `usage_stats` — raw per-session token records across workspaces.
        "usage_stats" => Ok(serde_json::json!({ "sessions": mgr.usage_stats() })),
        "agent_overview" => {
            let info = mgr.model_info();
            Ok(serde_json::json!({
                "instructions": agent_kernel::session::SYSTEM_PROMPT_TEMPLATE,
                "model": serde_json::to_value(&info).map_err(|e| e.to_string())?,
                "skills": scan_skills(&mgr.workspace_root),
                "plugins": mgr.plugin_overview(),
                // The `delegate` TOOL is the driver, not an agent — it's
                // never listed. Rows = built-in agents + `*.md` manifests.
                "subagents": agent_kernel::tools::delegate::BUILTIN_SUBAGENTS
                    .iter()
                    .map(|(n, d, p)| serde_json::json!({
                        "name": n,
                        "description": d,
                        "prompt": p,
                        "builtin": true,
                    }))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .chain(
                    agent_kernel::tools::delegate::discover_subagents(&mgr.workspace_root)
                        .into_iter()
                        .map(|a| serde_json::json!({
                            "name": a.name,
                            "description": a.description,
                            "prompt": a.prompt,
                            "path": a.path,
                            "global": a.global,
                            "builtin": false,
                        })),
                )
                .collect::<Vec<_>>(),
            }))
        }
        // ---- editable agent surfaces (settings "智能体" panes) ----
        // `get_instructions` / `set_instructions` — the AGENTS.md pair the
        // session prompt appends verbatim (global under ~/.config/husk,
        // project at the workspace root).
        "get_instructions" => Ok(mgr.instructions()),
        "set_instructions" => {
            let scope = payload
                .as_ref()
                .and_then(|p| p.get("scope"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = payload
                .as_ref()
                .and_then(|p| p.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            mgr.set_instructions(scope, content)?;
            Ok(serde_json::json!({ "success": true }))
        }
        // `create_skill` — scaffold `<name>/SKILL.md` under the workspace
        // `.pi/skills/` (project) or `~/.pi/agent/skills/` (global).
        "create_skill" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let desc = payload
                .as_ref()
                .and_then(|p| p.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body = payload
                .as_ref()
                .and_then(|p| p.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let global = payload
                .as_ref()
                .and_then(|p| p.get("scope"))
                .and_then(|v| v.as_str())
                == Some("global");
            if !valid_slug(&name) {
                return Err("技能名只能是小写字母/数字/-/ _".into());
            }
            let base = if global {
                dirs::home_dir().map(|h| h.join(".pi/agent/skills"))
            } else {
                Some(mgr.workspace_root.join(".pi/skills"))
            }
            .ok_or("no skills dir")?;
            let dir = base.join(&name);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = dir.join("SKILL.md");
            std::fs::write(
                &path,
                format!("---\nname: {name}\ndescription: {desc}\n---\n\n{body}\n"),
            )
            .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true, "path": path.to_string_lossy() }))
        }
        // `read_skill` / `delete_skill` — skills live in pi's dirs: global
        // `~/.pi/agent/skills/<name>/SKILL.md`, workspace
        // `<ws>/.pi/skills/<name>/SKILL.md`. Only those two are writable;
        // anything else the scanner finds (installed packs, other roots) is
        // read-only and has no delete path here.
        "read_skill" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let skill = agent_kernel::skills::scanner::find_skill(&mgr.workspace_root, &name)
                .ok_or_else(|| format!("找不到技能 {name}"))?;
            let text = std::fs::read_to_string(&skill.path).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "name": skill.name,
                "description": skill.description,
                "body": strip_front_matter(&text),
                "scope": if skill.global { "global" } else { "workspace" },
                "writable": skill_is_writable(&mgr.workspace_root, &skill.path),
            }))
        }
        "delete_skill" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let skill = agent_kernel::skills::scanner::find_skill(&mgr.workspace_root, &name)
                .ok_or_else(|| format!("找不到技能 {name}"))?;
            if !skill_is_writable(&mgr.workspace_root, &skill.path) {
                return Err("该技能不属于可写目录，不能删除".into());
            }
            let dir = skill.path.parent().ok_or("技能路径异常")?;
            std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true }))
        }
        // Same shape for subagents: `read_subagent` / `delete_subagent`. A
        // builtin has no file, so deleting one fails with that message rather
        // than silently doing nothing.
        "read_subagent" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let path = subagent_file(&mgr.workspace_root, &name)
                .ok_or_else(|| format!("找不到子代理 {name}"))?;
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let (_, desc, body) = agent_kernel::tools::delegate::split_agent(&text);
            Ok(serde_json::json!({
                "name": name,
                "description": desc.unwrap_or_default(),
                "prompt": body,
                "scope": if path.starts_with(mgr.workspace_root.join(".husk/agents")) { "workspace" } else { "global" },
            }))
        }
        "delete_subagent" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let path = subagent_file(&mgr.workspace_root, &name).ok_or("内置子代理不可删除")?;
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true }))
        }
        // `add_mcp` — write `~/.config/husk/mcp/<id>/manifest.json`
        // (entry {command, args}|{url}); the bridge loads on next session
        // spawn. MCP lives in its own tree, separate from plugins/.
        "add_mcp" => {
            let id = payload
                .as_ref()
                .and_then(|p| p.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let http = payload
                .as_ref()
                .and_then(|p| p.get("transport"))
                .and_then(|v| v.as_str())
                == Some("http");
            let command = payload
                .as_ref()
                .and_then(|p| p.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let url = payload
                .as_ref()
                .and_then(|p| p.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let args: Vec<String> = payload
                .as_ref()
                .and_then(|p| p.get("args"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let headers: serde_json::Map<String, serde_json::Value> = payload
                .as_ref()
                .and_then(|p| p.get("headers"))
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            if !valid_slug(&id) {
                return Err("插件 ID 只能是小写字母/数字/-/ _".into());
            }
            // entry shape picks the transport — `{url}` = streamable HTTP,
            // `{command, args}` = spawned stdio child (mcp.rs §Transport).
            let entry = if http {
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err("url 需要是 http(s) 地址".into());
                }
                let mut e = serde_json::json!({ "url": url });
                if !headers.is_empty() {
                    e["headers"] = serde_json::Value::Object(headers);
                }
                e
            } else {
                if command.is_empty() {
                    return Err("command 不能为空".into());
                }
                serde_json::json!({ "command": command, "args": args })
            };
            let dir = dirs::config_dir()
                .map(|d| d.join("husk/mcp").join(&id))
                .ok_or("no config dir")?;
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            // No `version`: the manifest's is a placeholder that means nothing
            // for a remote server, and the card shows the server's own version
            // (`mcp_probe`). The struct default keeps old files parsing.
            let manifest = serde_json::json!({
                "id": id,
                "name": name,
                "kind": "mcp",
                "entry": entry,
            });
            let path = dir.join("manifest.json");
            std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap())
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true, "path": path.to_string_lossy() }))
        }
        // `create_subagent` — write `<name>.md` under `.husk/agents/` (project)
        // or `~/.config/husk/agents/` (global); `delegate { agent: "<name>" }`
        // resolves it at call time.
        "create_subagent" => {
            let name = payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let desc = payload
                .as_ref()
                .and_then(|p| p.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let prompt = payload
                .as_ref()
                .and_then(|p| p.get("prompt"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let global = payload
                .as_ref()
                .and_then(|p| p.get("scope"))
                .and_then(|v| v.as_str())
                == Some("global");
            if !valid_slug(&name) {
                return Err("子代理名只能是小写字母/数字/-/ _".into());
            }
            if prompt.trim().is_empty() {
                return Err("提示词不能为空".into());
            }
            let base = if global {
                agent_kernel::tools::delegate::global_agent_dir()
            } else {
                Some(agent_kernel::tools::delegate::workspace_agent_dir(
                    &mgr.workspace_root,
                ))
            }
            .ok_or("no agents dir")?;
            std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
            let path = base.join(format!("{name}.md"));
            std::fs::write(
                &path,
                format!("---\nname: {name}\ndescription: {desc}\n---\n\n{prompt}\n"),
            )
            .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true, "path": path.to_string_lossy() }))
        }
        // Default preferences — the settings popup's backing store. These
        // are the values NEW sessions spawn with; reading/writing them never
        // touches a live session (no gate write, no UiCommand, no
        // SystemMessage in the chat stream).
        "get_default_prefs" => {
            let (permission_mode, thinking_level, agent_mode, compact_at) = mgr.default_prefs();
            let (memory_enabled, memory_distill) = mgr.memory_prefs();
            let (active_provider, active_model) = mgr.default_model();
            let lim = agent_kernel::sandbox_prefs::current();
            Ok(serde_json::json!({
                "permission_mode": permission_mode,
                "thinking_level": thinking_level,
                "agent_mode": agent_mode,
                "compact_at": compact_at,
                "active_provider": active_provider,
                "active_model": active_model,
                "memory_enabled": memory_enabled,
                "memory_distill": memory_distill,
                "sandbox_network": lim.network_label(),
                "sandbox_max_memory_mb": lim.max_memory_mb,
                "sandbox_max_processes": lim.max_processes,
            }))
        }
        "set_default_prefs" => {
            let p = payload.ok_or("set_default_prefs needs payload")?;
            let permission_mode = p
                .get("permission_mode")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let thinking_level = p
                .get("thinking_level")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let agent_mode = p
                .get("agent_mode")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let compact_at = p
                .get("compact_at")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32);
            // Sandbox overrides live in the process-wide slot — apply them
            // BEFORE mgr.set_default_prefs so its persist captures the
            // effective values. Any sandbox key in the payload rewrites all
            // three fields from the merged view (UI sends the whole row).
            let touched_sandbox = p.get("sandbox_network").is_some()
                || p.get("sandbox_max_memory_mb").is_some()
                || p.get("sandbox_max_processes").is_some();
            if touched_sandbox {
                let mut lim = agent_kernel::sandbox_prefs::current();
                if let Some(v) = p.get("sandbox_network").and_then(|v| v.as_str()) {
                    lim.network = agent_kernel::sandbox_prefs::network_from_label(v);
                }
                if p.get("sandbox_max_memory_mb").is_some() {
                    lim.max_memory_mb = p.get("sandbox_max_memory_mb").and_then(|v| v.as_u64());
                }
                if p.get("sandbox_max_processes").is_some() {
                    lim.max_processes = p
                        .get("sandbox_max_processes")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                }
                agent_kernel::sandbox_prefs::set(lim);
            }
            mgr.set_default_prefs(permission_mode, thinking_level, agent_mode, compact_at);
            // Default model — `active_provider`+`active_model` arrive as a
            // pair or not at all ("" on either clears the override). Own
            // setter so the save lands in one `default_preferences` write.
            if p.get("active_provider").is_some() || p.get("active_model").is_some() {
                let ap = p
                    .get("active_provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let am = p
                    .get("active_model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                mgr.set_default_model(ap, am);
            }
            // Memory switches ride the same payload — separate setter so the
            // save lands in one `default_preferences` write.
            let mem_en = p.get("memory_enabled").and_then(|v| v.as_bool());
            let mem_di = p.get("memory_distill").and_then(|v| v.as_bool());
            if mem_en.is_some() || mem_di.is_some() {
                mgr.set_memory_prefs(mem_en, mem_di);
                // Hot-apply to every live actor — stored prefs alone only
                // reach the NEXT spawn; actors swap their store/distiller
                // on the command.
                mgr.broadcast_memory_prefs();
            }
            Ok(serde_json::json!({ "success": true }))
        }
        // Memory pane — the three write paths (remember tool, /remember,
        // distill) all land on this store; the pane is its read/delete side.
        "memory_overview" => {
            let (enabled, distill) = mgr.memory_prefs();
            let mut out = serde_json::json!({
                "enabled": enabled,
                "distill": distill,
                "persona": [],
                "facts": [],
                "episodes": 0,
            });
            if let Some(s) = open_memory(&mgr) {
                out["persona"] =
                    serde_json::to_value(s.all_persona().unwrap_or_default()).unwrap_or_default();
                out["facts"] = s
                    .list_facts(200)
                    .unwrap_or_default()
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "id": f.id,
                            "text": f.text,
                            "confidence": f.confidence,
                            "created_at": f.created_at,
                        })
                    })
                    .collect();
                out["episodes"] = serde_json::json!(s.count_episodes().unwrap_or(0));
            } else {
                // Honest failure — an empty array would masquerade as
                // "nothing remembered" when the store simply couldn't open.
                out["error"] = serde_json::json!("memory.db 打不开（锁定或数据目录不可达）");
            }
            Ok(out)
        }
        "memory_set" => {
            let p = payload.clone().ok_or("memory_set needs payload")?;
            let key = p
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("memory_set needs key")?;
            let value = p
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("memory_set needs value")?;
            let s = open_memory(&mgr).ok_or("memory store unavailable")?;
            s.set_persona(key, value)?;
            Ok(serde_json::json!({ "success": true }))
        }
        "memory_forget" => {
            let p = payload.clone().ok_or("memory_forget needs payload")?;
            let key = p
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("memory_forget needs key")?;
            let s = open_memory(&mgr).ok_or("memory store unavailable")?;
            let removed = s.remove_persona(key)?;
            Ok(serde_json::json!({ "success": true, "removed": removed }))
        }
        "memory_remove_fact" => {
            let p = payload.clone().ok_or("memory_remove_fact needs payload")?;
            let fid = p
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or("memory_remove_fact needs id")?;
            let s = open_memory(&mgr).ok_or("memory store unavailable")?;
            let removed = s.remove_fact(fid)?;
            Ok(serde_json::json!({ "success": true, "removed": removed }))
        }
        "memory_clear" => {
            let p = payload.clone().ok_or("memory_clear needs payload")?;
            let table = p.get("table").and_then(|v| v.as_str()).unwrap_or("all");
            let s = open_memory(&mgr).ok_or("memory store unavailable")?;
            let removed = s.clear(table)?;
            Ok(serde_json::json!({ "success": true, "removed": removed }))
        }
        // Appearance — the settings window's theme/accent/font choices,
        // persisted next to `default_preferences.json`. Both windows read
        // this at boot; a write from the settings window lands on the next
        // main-window load.
        "get_appearance" => {
            let s = agent_kernel::load_appearance_settings();
            Ok(serde_json::to_value(&s).map_err(|e| e.to_string())?)
        }
        "set_appearance" => {
            let p = payload.ok_or("set_appearance needs payload")?;
            let cur = agent_kernel::load_appearance_settings();
            let merged = serde_json::to_value(&cur).unwrap_or_else(|_| serde_json::json!({}));
            let mut obj = match merged {
                serde_json::Value::Object(m) => m,
                _ => serde_json::Map::new(),
            };
            // Merge only the keys the caller sent — partial updates keep
            // the rest of the file intact.
            for k in [
                "theme_mode",
                "theme_id",
                "accent",
                "background",
                "foreground",
                "dark_accent",
                "dark_background",
                "dark_foreground",
                "ui_font",
                "code_font",
                "contrast",
                "currency",
            ] {
                if let Some(v) = p.get(k) {
                    obj.insert(k.into(), v.clone());
                }
            }
            let s: agent_kernel::AppearanceSettings =
                serde_json::from_value(serde_json::Value::Object(obj))
                    .map_err(|e| e.to_string())?;
            agent_kernel::save_appearance_settings(&s).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "success": true }))
        }
        "open_url" => {
            // Open an external link in the system browser — used by the
            // settings About pane (repo / issues / releases). http(s) only.
            let url = payload
                .and_then(|p| p.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .ok_or("open_url needs a url")?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("open_url only accepts http(s) urls".into());
            }
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&url).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &url])
                .spawn();
            Ok(serde_json::json!({ "success": true }))
        }
        "open_config" => {
            if let Some(path) = agent_llm::AppConfig::default_path() {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(&path).spawn();
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &path.to_string_lossy()])
                    .spawn();
                Ok(serde_json::json!({ "path": path.to_string_lossy() }))
            } else {
                Err("config path not found".into())
            }
        }
        "get_app_config" => {
            let path = agent_llm::AppConfig::default_path();
            let raw = path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            // NORMALIZED view — `AppConfig::load` folds naked provider
            // maps + camelCase aliases into the canonical shape, so the
            // settings UI and `model_info` always see the same `providers`
            // map regardless of how the file was hand-written.
            let cfg = agent_llm::AppConfig::load(None).unwrap_or_default();
            let config_val = serde_json::to_value(&cfg).unwrap_or_else(|_| serde_json::json!({}));
            Ok(serde_json::json!({
                "path": path.map(|p| p.to_string_lossy().to_string()),
                "raw": raw,
                "config": config_val,
                "permission_mode": mgr.permission_mode,
                "thinking_level": mgr.active_thinking_level,
            }))
        }
        // User-level dev config — `~/.config/husk/settings.toml`. The
        // file sits beside `config.toml` so a broken renderer flag can
        // be reverted by hand even while the GUI is dead.
        "get_dev_config" => {
            let dev = agent_llm::config::DevConfig::load()
                .map_err(|e| format!("load dev config: {e}"))?;
            Ok(serde_json::to_value(&dev).map_err(|e| e.to_string())?)
        }
        "save_dev_config" => {
            let Some(val) = payload else {
                return Err("save_dev_config requires payload".into());
            };
            let dev: agent_llm::config::DevConfig =
                serde_json::from_value(val).map_err(|e| format!("dev config: {e}"))?;
            // Validate enum-ish fields so a typo'd renderer can't silently
            // fall back to the default.
            if let Some(r) = dev.renderer.as_deref() {
                if !matches!(r, "x11" | "wayland") {
                    return Err(format!(
                        "dev.renderer must be \"x11\" or \"wayland\", got {r:?}"
                    ));
                }
            }
            agent_llm::config::DevConfig::save(&dev)
                .map_err(|e| format!("save dev config: {e}"))?;
            Ok(serde_json::json!({ "success": true }))
        }
        "save_app_config" => {
            let Some(path) = agent_llm::AppConfig::default_path() else {
                return Err("config path not found".into());
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let Some(val) = payload else {
                return Err("save_app_config requires payload".into());
            };
            // Normalize BEFORE writing — accept the canonical
            // `{providers:{…}}` shape or a naked provider map, then write
            // the file back in ITS OWN format (TOML stays TOML — never
            // JSON-bytes-in-a-.toml-file).
            let mut cfg =
                serde_json::from_value::<agent_llm::AppConfig>(val.clone()).unwrap_or_default();
            if cfg.providers.is_empty() {
                if let Some(obj) = val.as_object() {
                    for (k, v) in obj {
                        if matches!(
                            k.as_str(),
                            "active_provider"
                                | "activeProvider"
                                | "active_model"
                                | "activeModel"
                                | "fallback_chain"
                                | "fallbackChain"
                                | "providers"
                        ) {
                            continue;
                        }
                        if let Ok(p) =
                            serde_json::from_value::<agent_llm::ProviderConfig>(v.clone())
                        {
                            cfg.providers.insert(k.clone(), p);
                        }
                    }
                }
            }
            if cfg.providers.is_empty() {
                return Err("config has no providers — refusing to write".into());
            }
            let text = if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?
            } else {
                serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?
            };
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
            // Hand the edit to the running session — an actor keeps the provider
            // instance and model parameters it was spawned with, so without this
            // the change only surfaced after a restart.
            let _ = mgr.reload_model_config();
            Ok(serde_json::json!({ "success": true }))
        }
        "get_sandbox_info" => {
            let (backend, loud) = agent_sandbox::detect_backend();
            Ok(serde_json::json!({
                "backend": backend.id(),
                "tier": format!("{:?}", backend.tier()),
                "is_fallback": loud,
            }))
        }
        // `@` mention picker — workspace-relative file list (gitignore-aware,
        // capped) for the composer autocomplete. Returns paths only; the
        // kernel inlines content when the prompt is submitted.
        "list_files" => {
            let q = payload
                .as_ref()
                .and_then(|p| p.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let files = workspace_file_index(&mgr.workspace_root, &q, 400);
            Ok(serde_json::json!(files))
        }
        // `/` + `$` picker — the kernel's `scan_all_skills` (workspace +
        // user-level dirs, Claude-Code/pi-compatible SKILL.md manifests).
        "list_skills" => Ok(serde_json::json!(scan_skills(&mgr.workspace_root))),
        // `/` picker's plugin half — declared `capabilities.commands` from
        // enabled plugins, qualified names included for collisions.
        "plugin_commands" => Ok(serde_json::json!(mgr.plugin_commands())),
        // Read one picked file for the attachment chips. Picked paths may
        // live outside the workspace — the sandbox's rw mounts are the
        // workspace + a per-run tmp dir, so an outside path is invisible
        // to `read`/`bash`. `stage_attachment` copies it into
        // `.husk/attachments/` (inside the rw mount, durable for replay)
        // and the staged path is what reaches the model.
        "read_attachment" => {
            let p = payload
                .as_ref()
                .and_then(|p| p.get("path"))
                .and_then(|v| v.as_str())
                .ok_or("read_attachment needs path")?;
            let source = std::path::PathBuf::from(p);
            let name = source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.to_string());
            let path = stage_attachment(&source, &mgr.workspace_root);
            describe_attachment(&path, name)
        }
        // Clipboard paste — a pasted image/file has no path to hand over
        // (the webview only ever gives us the bytes), so the frontend sends
        // them base64-encoded and we stage them exactly like a picked file.
        // Same chip JSON back, so paste and `+` feed one Attachment list.
        "attach_bytes" => stage_clipboard_payload(payload.as_ref(), &mgr.workspace_root),
        _ => Err(format!("unknown session op: {op}")),
    }
}

/// Flat workspace file index for the `@` picker — wraps
/// `WorkspaceScanner::build_file_index` (shared with the iced shell) and
/// applies the optional lowercase substring query + cap on top.
/// Directories are skipped — the picker completes files, not folders.
fn workspace_file_index(root: &std::path::Path, query: &str, cap: usize) -> Vec<serde_json::Value> {
    let index = agent_context::WorkspaceScanner::build_file_index(root, 0).unwrap_or_default();
    index
        .into_iter()
        .filter(|p| query.is_empty() || p.to_lowercase().contains(query))
        .take(cap)
        .map(|p| {
            serde_json::json!({
                "path": p,
                "name": p.rsplit('/').next().unwrap_or(&p),
            })
        })
        .collect()
}

/// A discovered skill — `agent_kernel::skills::scanner::scan_all_skills` does the
/// real scan (`.agents/skills`, `.agent/skills`, `.skills`, `.claude/skills`,
/// `.pi/skills` in the workspace, plus the user-level dirs under `$HOME`).
/// `path` is workspace-relative, or `~/…` for user-level skills.
fn scan_skills(root: &std::path::Path) -> Vec<serde_json::Value> {
    agent_kernel::skills::scanner::scan_all_skills(root)
        .into_iter()
        .map(|s| {
            let path = if let Ok(rel) = s.path.strip_prefix(root) {
                rel.to_string_lossy().replace('\\', "/")
            } else if let Some(home) = dirs::home_dir() {
                s.path
                    .strip_prefix(&home)
                    .map(|rel| format!("~/{}", rel.to_string_lossy().replace('\\', "/")))
                    .unwrap_or_else(|_| s.path.to_string_lossy().into_owned())
            } else {
                s.path.to_string_lossy().into_owned()
            };
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "path": path,
                "global": s.global,
                // Only the two pi roots the app writes are editable; installed
                // packs and other discovered roots are read-only here.
                "writable": skill_is_writable(root, &s.path),
            })
        })
        .collect()
}

/// Stage a user-picked attachment into `<workspace>/.husk/attachments/` —
/// the sandbox's rw mounts are the workspace + a per-run tmp dir, so a
/// path outside the workspace is invisible to `read`/`bash`. The staged
/// copy lands inside the mount AND survives restart (session replay and
/// later turns can still resolve it). In-workspace files pass through
/// unchanged. The dir carries its own `.gitignore` so uploads never
/// pollute `git status`.
fn stage_attachment(
    source: &std::path::Path,
    workspace_root: &std::path::Path,
) -> std::path::PathBuf {
    if source.starts_with(workspace_root) {
        return source.to_path_buf();
    }
    let Ok(bytes) = std::fs::read(source) else {
        return source.to_path_buf();
    };
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    stage_bytes(&bytes, &name, workspace_root)
}

/// Stage raw bytes (a picked file's contents, or a clipboard paste) under
/// `<workspace>/.husk/attachments/` and return the staged path.
///
/// Content-hash name — re-pasting the same image is idempotent, and two
/// different files sharing a filename never collide.
fn stage_bytes(bytes: &[u8], name: &str, workspace_root: &std::path::Path) -> std::path::PathBuf {
    let dir = workspace_root.join(".husk").join("attachments");
    let gitignore = workspace_root.join(".husk").join(".gitignore");
    if std::fs::create_dir_all(&dir).is_ok() && !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "*\n");
    }
    let hash = xxhash_rust::xxh3::xxh3_64(bytes);
    let as_path = std::path::Path::new(name);
    let stem = as_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let ext = as_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let staged = dir.join(format!("{stem}-{hash:016x}{ext}"));
    if !staged.exists() && std::fs::write(&staged, bytes).is_err() {
        // Unwritable workspace — hand back nothing rather than a path that
        // does not exist (`describe_attachment` then reports the failure).
        return staged;
    }
    staged
}

/// Stage a PNG read from the *system* clipboard (see `paste_clipboard`) and
/// describe it as a chip — the Rust-side twin of the `attach_bytes` op, so a
/// Ctrl+V image and a pasted blob land in the same list.
pub(crate) fn stage_clipboard_image(
    png: &[u8],
    workspace_root: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let path = stage_bytes(png, "clipboard.png", workspace_root);
    describe_attachment(&path, "clipboard.png".into())
}

/// RGBA8 → PNG. Needed because the clipboard hands over raw pixels while every
/// provider (and `ImageRef`) speaks a real image format.
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png encode: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("png encode: {e}"))?;
    }
    Ok(out)
}

/// The chip payload for one staged file — the single shape both
/// `read_attachment` (picked path) and `attach_bytes` (clipboard paste)
/// return, so the frontend keeps one `Attachment` list.
/// `attach_bytes` body — split out of the command match so the wire contract
/// (`name` / `mime` / base64 `data`) stays unit-testable without Tauri state.
fn stage_clipboard_payload(
    payload: Option<&serde_json::Value>,
    workspace_root: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let p = payload.ok_or("attach_bytes needs a payload")?;
    let mut name = p
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("clipboard")
        .to_string();
    // A nameless clipboard blob (WebKit hands over `image/png` with an empty
    // name in some paste paths) must still land with an extension — the image
    // detection downstream is extension-based, and a bare `clipboard` file
    // would be treated as opaque binary instead of an image.
    if std::path::Path::new(&name).extension().is_none() {
        if let Some(ext) = p.get("mime").and_then(|v| v.as_str()).and_then(mime_ext) {
            name = format!("{name}.{ext}");
        }
    }
    let data = p
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or("attach_bytes needs data")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("attach_bytes: bad base64: {e}"))?;
    if bytes.is_empty() {
        return Err("attach_bytes: empty payload".into());
    }
    const MAX_PASTE_BYTES: usize = 24 * 1024 * 1024;
    if bytes.len() > MAX_PASTE_BYTES {
        return Err(format!(
            "粘贴内容过大（{} MB，上限 {} MB）",
            bytes.len() / (1024 * 1024),
            MAX_PASTE_BYTES / (1024 * 1024)
        ));
    }
    let path = stage_bytes(&bytes, &name, workspace_root);
    describe_attachment(&path, name)
}

/// Base64 bytes → an `rfd` save dialog → `std::fs::write`. The webview's own
/// `<a download>` never reaches the filesystem (Tauri wires no handler); `data` is
/// the envelope `attach_bytes` takes.
/// Run a blocking `rfd` dialog on the MAIN thread and wait on a channel for
/// the answer. AppKit panels are main-thread-only, and — per the header of
/// `agent_session_inner` — must never run while the kernel mutex is held.
/// Dispatch failure or a closed app both read as "cancelled" (None).
fn dialog_on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .ok()?;
    rx.recv().ok()
}

/// Workspace-relative path → absolute, canonicalized and contained —
/// the artifact actions (`open_path` / `reveal_path` / `save_artifact`)
/// accept `path` strings the model wrote as tool args, so absolute paths,
/// `..` escapes and symlink escapes are all refused up front.
fn resolve_workspace_path(root: &std::path::Path, rel: &str) -> Result<std::path::PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err("empty path".into());
    }
    let p = std::path::Path::new(rel);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("path escapes workspace".into());
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canon = root
        .join(p)
        .canonicalize()
        .map_err(|e| format!("{rel}: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path escapes workspace".into());
    }
    Ok(canon)
}

fn save_download(
    app: &tauri::AppHandle,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let p = payload.ok_or("save_download needs a payload")?;
    let name = p
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("download");
    let data = p
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or("save_download needs payload.data")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("save_download: bad base64: {e}"))?;
    if bytes.is_empty() {
        return Err("save_download: empty payload".into());
    }
    // The extension comes from the caller; the dialog decides the real path.
    // AppKit panels are main-thread-only → hop over via the channel helper.
    let name = name.to_string();
    let Some(target) = dialog_on_main(app, move || {
        rfd::FileDialog::new()
            .set_title("保存文件")
            .set_file_name(&name)
            .save_file()
    })
    .flatten() else {
        // Cancelled — not an error, nothing written.
        return Ok(serde_json::json!({ "saved": false }));
    };
    std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "saved": true,
        "path": target.to_string_lossy(),
        "bytes": bytes.len(),
    }))
}

/// Extension for a clipboard MIME type — only used when the pasted name
/// carries none, so the staged file is still classifiable (image vs text vs
/// binary) and vision providers accept it.
fn mime_ext(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "image/svg+xml" => Some("svg"),
        "text/plain" => Some("txt"),
        "application/json" => Some("json"),
        _ => None,
    }
}

fn describe_attachment(path: &std::path::Path, name: String) -> Result<serde_json::Value, String> {
    let staged = path.to_string_lossy().to_string();
    if let Some(img) = agent_llm::types::ImageRef::for_path(path.to_path_buf()) {
        return Ok(serde_json::json!({
            "path": staged, "name": name, "kind": "image",
            "data_url": img.data_url(),
        }));
    }
    const MAX_BYTES: usize = 32 * 1024;
    match std::fs::read(path) {
        Ok(bytes) if bytes.contains(&0) => Ok(serde_json::json!({
            "path": staged, "name": name, "kind": "binary",
        })),
        Ok(bytes) => {
            let truncated = bytes.len() > MAX_BYTES;
            let capped = if truncated {
                &bytes[..MAX_BYTES]
            } else {
                &bytes[..]
            };
            Ok(serde_json::json!({
                "path": staged,
                "name": name,
                "kind": "text",
                "content": String::from_utf8_lossy(capped),
                "truncated": truncated,
            }))
        }
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

/// The two skill roots the app writes to (`create_skill`), so anything else
/// the scanner finds is read-only here.
fn skill_is_writable(workspace_root: &std::path::Path, path: &std::path::Path) -> bool {
    let Ok(dir) = path.strip_prefix(path.parent().and_then(|p| p.parent()).unwrap_or(path)) else {
        return false;
    };
    let _ = dir;
    let name_dir = match path.parent() {
        Some(d) => d,
        None => return false,
    };
    let ws = workspace_root.join(".pi/skills");
    let gl = dirs::home_dir().map(|h| h.join(".pi/agent/skills"));
    [Some(ws), gl]
        .into_iter()
        .flatten()
        .any(|base| name_dir.parent() == Some(base.as_path()))
}

/// `<name>.md` in the workspace or global agent dir — `None` for a builtin,
/// which has no file to edit or delete.
fn subagent_file(workspace_root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    if !valid_slug(name) {
        return None;
    }
    let ws = agent_kernel::tools::delegate::workspace_agent_dir(workspace_root)
        .join(format!("{name}.md"));
    if ws.is_file() {
        return Some(ws);
    }
    let gl = agent_kernel::tools::delegate::global_agent_dir()?.join(format!("{name}.md"));
    gl.is_file().then_some(gl)
}

/// Everything after the `---` front-matter block, for the edit dialogs.
fn strip_front_matter(text: &str) -> String {
    let trimmed = text.strip_prefix("---").unwrap_or(text);
    match trimmed.find("\n---") {
        Some(i) => trimmed[i + 4..].trim_start().to_string(),
        None => text.trim_start().to_string(),
    }
}

/// Slug check for user-created ids (skill names, plugin ids, subagent
/// names) — lowercase alnum + `-`/`_` only, so names stay path-safe.
fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Recursive walk for `recent_artifacts` — `(rel path, file name, mtime
/// secs)` per file, depth-capped so a stray deep tree can't stall the UI.
fn collect_recent_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    out: &mut Vec<(String, String, u64)>,
) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hidden entries (`.dashi-ppt-project`) and officecli's own crash
        // dumps are workspace bookkeeping, not produced files.
        if name.starts_with('.') || name.starts_with("hs_err") {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                collect_recent_files(root, &path, depth - 1, out);
            }
            Ok(ft) if ft.is_file() => {
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                let mtime = path
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.push((rel.to_string_lossy().replace('\\', "/"), name, mtime));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 1x1 PNG — enough for `ImageRef` (which is extension-based).
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

    #[test]
    fn staged_bytes_are_idempotent_and_content_addressed() {
        let ws = tempfile::tempdir().unwrap();
        let a = stage_bytes(PNG, "clipboard.png", ws.path());
        let b = stage_bytes(PNG, "clipboard.png", ws.path());
        assert_eq!(a, b, "same bytes + name must reuse one staged file");
        assert!(a.exists());
        assert!(a.starts_with(ws.path().join(".husk/attachments")));

        let other = stage_bytes(b"different", "clipboard.png", ws.path());
        assert_ne!(a, other, "different content must not collide");
        // Uploads never pollute `git status`.
        assert!(ws.path().join(".husk/.gitignore").exists());
    }

    #[test]
    fn describe_classifies_image_text_and_binary() {
        let ws = tempfile::tempdir().unwrap();

        let img = stage_bytes(PNG, "clipboard.png", ws.path());
        let chip = describe_attachment(&img, "clipboard.png".into()).unwrap();
        assert_eq!(chip["kind"], "image");
        assert!(chip["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        let txt = stage_bytes(b"hello\n", "note.md", ws.path());
        let chip = describe_attachment(&txt, "note.md".into()).unwrap();
        assert_eq!(chip["kind"], "text");
        assert_eq!(chip["content"], "hello\n");

        let bin = stage_bytes(b"a\0b", "blob.bin", ws.path());
        let chip = describe_attachment(&bin, "blob.bin".into()).unwrap();
        assert_eq!(chip["kind"], "binary");
    }

    #[test]
    fn nameless_pastes_get_an_extension_from_their_mime() {
        assert_eq!(mime_ext("image/png"), Some("png"));
        assert_eq!(mime_ext("image/jpeg"), Some("jpg"));
        assert_eq!(mime_ext("application/pdf"), None);
    }

    /// Locks the frontend contract: op `attach_bytes` with
    /// `{ name, mime, data }`, and a chip shaped exactly like the picked-file
    /// path so both flows feed one `Attachment` list.
    #[test]
    fn clipboard_payload_round_trips_to_an_image_chip() {
        let ws = tempfile::tempdir().unwrap();
        let payload = serde_json::json!({
            "name": "clipboard",
            "mime": "image/png",
            "data": base64::engine::general_purpose::STANDARD.encode(PNG),
        });
        let chip = stage_clipboard_payload(Some(&payload), ws.path()).unwrap();
        assert_eq!(chip["kind"], "image");
        assert_eq!(chip["name"], "clipboard.png");
        // Staged name is `clipboard-<content-hash>.png`.
        let staged = chip["path"].as_str().unwrap();
        assert!(
            staged.ends_with(".png"),
            "extension from `mime` must reach the file: {staged}"
        );
        assert!(staged.contains("clipboard"), "stem survives: {staged}");
        assert!(chip["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    /// The system-clipboard path hands over raw RGBA; this is the only place
    /// the pixels become an image a provider will accept.
    #[test]
    fn clipboard_pixels_encode_to_a_real_png() {
        let rgba = vec![255u8, 0, 0, 255, 0, 255, 0, 255]; // 2x1: red, green
        let png = encode_png(&rgba, 2, 1).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

        let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(
            &buf[..4],
            &[255, 0, 0, 255],
            "pixels survive the round trip"
        );

        // …and it lands as an image chip, not opaque binary.
        let ws = tempfile::tempdir().unwrap();
        let chip = stage_clipboard_image(&png, ws.path()).unwrap();
        assert_eq!(chip["kind"], "image");
        assert!(chip["data_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn clipboard_payload_rejects_missing_and_corrupt_input() {
        let ws = tempfile::tempdir().unwrap();
        assert!(stage_clipboard_payload(None, ws.path()).is_err());
        let no_data = serde_json::json!({ "name": "x.png" });
        assert!(stage_clipboard_payload(Some(&no_data), ws.path()).is_err());
        let bad_b64 = serde_json::json!({ "name": "x.png", "data": "not base64!!" });
        assert!(stage_clipboard_payload(Some(&bad_b64), ws.path()).is_err());
        let empty = serde_json::json!({ "name": "x.png", "data": "" });
        assert!(stage_clipboard_payload(Some(&empty), ws.path()).is_err());
    }
}

/// Connect one MCP server and report what it actually is (live name/version +
/// tool names, or the reason it failed). See `SessionManager::mcp_probe`.
#[tauri::command]
pub async fn mcp_probe(
    state: State<'_, KernelState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let root = {
        let m = state.0.lock().map_err(|e| e.to_string())?;
        m.workspace_root.clone()
    };
    Ok(agent_kernel::session_manager::SessionManager::mcp_probe(&root, &id).await)
}
