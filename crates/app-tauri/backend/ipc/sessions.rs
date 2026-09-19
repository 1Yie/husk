//! `agent_session` — session-list + switch/create commands for the sidebar.

use tauri::{Manager, State};

use crate::kernel::KernelState;

#[tauri::command]
pub fn agent_session(
    app: tauri::AppHandle,
    state: State<'_, KernelState>,
    op: String,
    id: Option<i64>,
    path: Option<String>,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    match op.as_str() {
        "list" => Ok(serde_json::json!(mgr.sidebar_rows().iter().map(|(id,t,p,a,r)| {
            serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r})
        }).collect::<Vec<_>>())),
        "new" => { mgr.new_session(); Ok(serde_json::json!({"active": mgr.active_id})) }
        // `open` also returns the persisted history so the webview can
        // rebuild the stream for a session it has no in-memory view for
        // (first open after launch, or a view that was dropped). The live
        // actor is resumed separately — this is display-only data.
        "open" => {
            let id = id.ok_or("open needs id")?;
            mgr.open_session(id);
            let history = mgr.store_history(id).unwrap_or_default();
            Ok(serde_json::json!({"active": mgr.active_id, "history": history}))
        }
        // `delete` mirrors `open` in returning the new active id + its
        // history so the webview can rebuild the stream when the deleted
        // session was the one on screen.
        "delete" => {
            let id = id.ok_or("delete needs id")?;
            mgr.delete_session(id);
            let history = mgr.store_history(mgr.active_id).unwrap_or_default();
            Ok(serde_json::json!({"active": mgr.active_id, "history": history}))
        }
        // `fork` copies the source session's latest snapshot into a new
        // session and activates it — same return shape as `open`.
        "fork" => {
            let id = id.ok_or("fork needs id")?;
            match mgr.fork_session(id) {
                Some(new_id) => {
                    let history = mgr.store_history(new_id).unwrap_or_default();
                    Ok(serde_json::json!({"active": mgr.active_id, "id": new_id, "history": history}))
                }
                None => Err("session has no history to fork yet".into()),
            }
        }
        "workspace_info" => {
            let name = mgr.workspace_root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            Ok(serde_json::json!({
                "root": mgr.workspace_root.to_string_lossy(),
                "name": name,
                "recents": mgr.recent_workspaces().iter().map(|w| {
                    serde_json::json!({ "path": w.path.to_string_lossy(), "name": w.name, "last_opened": w.last_opened })
                }).collect::<Vec<_>>()
            }))
        }
        "pick_workspace" => {
            let picked = rfd::FileDialog::new().set_title("Open Workspace Directory").pick_folder();
            if let Some(target) = picked {
                mgr.switch_workspace(target).map_err(|e| e.to_string())?;
                let name = mgr.workspace_root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let history = mgr.store_history(mgr.active_id).unwrap_or_default();
                Ok(serde_json::json!({
                    "root": mgr.workspace_root.to_string_lossy(),
                    "name": name,
                    "active": mgr.active_id,
                    "history": history,
                    "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r)| {
                        serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r})
                    }).collect::<Vec<_>>(),
                }))
            } else {
                Ok(serde_json::json!(null))
            }
        }
        "switch_workspace" => {
            let p = path.ok_or("switch_workspace needs path")?;
            mgr.switch_workspace(std::path::PathBuf::from(p)).map_err(|e| e.to_string())?;
            let name = mgr.workspace_root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let history = mgr.store_history(mgr.active_id).unwrap_or_default();
            Ok(serde_json::json!({
                "root": mgr.workspace_root.to_string_lossy(),
                "name": name,
                "active": mgr.active_id,
                "history": history,
                "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r)| {
                    serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r})
                }).collect::<Vec<_>>(),
            }))
        }
        "model_info" => {
            let info = mgr.model_info();
            Ok(serde_json::to_value(&info).map_err(|e| e.to_string())?)
        }
        // Default preferences — the settings popup's backing store. These
        // are the values NEW sessions spawn with; reading/writing them never
        // touches a live session (no gate write, no UiCommand, no
        // SystemMessage in the chat stream).
        "get_default_prefs" => {
            let (permission_mode, thinking_level) = mgr.default_prefs();
            Ok(serde_json::json!({
                "permission_mode": permission_mode,
                "thinking_level": thinking_level,
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
            mgr.set_default_prefs(permission_mode, thinking_level);
            Ok(serde_json::json!({ "success": true }))
        }
        "open_config" => {
            if let Some(path) = agent_llm::AppConfig::default_path() {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(&path).spawn();
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).spawn();
                Ok(serde_json::json!({ "path": path.to_string_lossy() }))
            } else {
                Err("config path not found".into())
            }
        }
        "open_settings_window" => {
            if let Some(win) = app.get_webview_window("settings") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
                let _ = win.eval("window.location.reload()");
            } else {
                tauri::WebviewWindowBuilder::new(
                    &app,
                    "settings",
                    tauri::WebviewUrl::App("index.html?window=settings".into()),
                )
                .title("设置")
                .inner_size(980.0, 680.0)
                .min_inner_size(800.0, 540.0)
                .resizable(true)
                .maximizable(false)
                .decorations(false)
                .center()
                .build()
                .map_err(|e| e.to_string())?;
            }
            Ok(serde_json::json!({ "success": true }))
        }
        "get_app_config" => {
            let path = agent_llm::AppConfig::default_path();
            let raw = path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            let config_val = if !raw.trim().is_empty() {
                serde_json::from_str::<serde_json::Value>(&raw).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            Ok(serde_json::json!({
                "path": path.map(|p| p.to_string_lossy().to_string()),
                "raw": raw,
                "config": config_val,
                "permission_mode": mgr.permission_mode,
                "thinking_level": mgr.active_thinking_level,
            }))
        }
        "save_app_config" => {
            let Some(path) = agent_llm::AppConfig::default_path() else {
                return Err("config path not found".into());
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Some(val) = payload {
                let formatted = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
                std::fs::write(&path, formatted).map_err(|e| e.to_string())?;
                let _ = mgr.model_info();
                Ok(serde_json::json!({ "success": true }))
            } else {
                Err("save_app_config requires payload".into())
            }
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
        "list_skills" => {
            Ok(serde_json::json!(scan_skills(&mgr.workspace_root)))
        }
        _ => Err(format!("unknown session op: {op}")),
    }
}

/// Flat workspace file index for the `@` picker — wraps
/// `WorkspaceScanner::build_file_index` (shared with the iced shell) and
/// applies the optional lowercase substring query + cap on top.
/// Directories are skipped — the picker completes files, not folders.
fn workspace_file_index(
    root: &std::path::Path,
    query: &str,
    cap: usize,
) -> Vec<serde_json::Value> {
    let index = agent_context::WorkspaceScanner::build_file_index(root, 0)
        .unwrap_or_default();
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

/// A discovered skill — `agent_kernel::commands::scan_all_skills` does the
/// real scan (`.agents/skills`, `.agent/skills`, `.skills`, `.claude/skills`,
/// `.pi/skills` in the workspace, plus the user-level dirs under `$HOME`).
/// `path` is workspace-relative, or `~/…` for user-level skills.
fn scan_skills(root: &std::path::Path) -> Vec<serde_json::Value> {
    agent_kernel::commands::scan_all_skills(root)
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
            })
        })
        .collect()
}
