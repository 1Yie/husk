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
        "list" => Ok(serde_json::json!(mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
            serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
        }).collect::<Vec<_>>())),
        // Sidebar project tree — every recent workspace with its full
        // conversation list. The active workspace comes first (`current: true`)
        // and carries the live running/active overlay; other projects are
        // store-only snapshots (one workspace has actors at a time).
        "projects" => Ok(serde_json::to_value(mgr.projects_overview()).map_err(|e| e.to_string())?),
        "new" => { mgr.new_session(); Ok(serde_json::json!({"active": mgr.active_id})) }
        // `open` also returns the persisted history so the webview can
        // rebuild the stream for a session it has no in-memory view for
        // (first open after launch, or a view that was dropped). The live
        // actor is resumed separately — this is display-only data.
        "open" => {
            let id = id.ok_or("open needs id")?;
            mgr.open_session(id);
            let history = mgr.store_history(id).unwrap_or_default();
            // `usage` is the persisted last-turn meter — seeds the header
            // stats so a reopened session doesn't read 0/… until the next
            // turn's `Usage` event.
            Ok(serde_json::json!({
                "active": mgr.active_id,
                "history": history,
                "usage": mgr.store_usage(id),
            }))
        }
        // `delete` mirrors `open` in returning the new active id + its
        // history so the webview can rebuild the stream when the deleted
        // session was the one on screen.
        "delete" => {
            let id = id.ok_or("delete needs id")?;
            mgr.delete_session(id);
            let history = mgr.store_history(mgr.active_id).unwrap_or_default();
            let usage = mgr.store_usage(mgr.active_id);
            Ok(serde_json::json!({"active": mgr.active_id, "history": history, "usage": usage}))
        }
        // `fork` copies the source session's latest snapshot into a new
        // session and activates it — same return shape as `open`.
        "fork" => {
            let id = id.ok_or("fork needs id")?;
            match mgr.fork_session(id) {
                Some(new_id) => {
                    let history = mgr.store_history(new_id).unwrap_or_default();
                    Ok(serde_json::json!({
                        "active": mgr.active_id,
                        "id": new_id,
                        "history": history,
                        "usage": mgr.store_usage(new_id),
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
            let name = mgr.workspace_root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            Ok(serde_json::json!({
                "root": mgr.workspace_root.to_string_lossy(),
                "name": name,
                "recents": mgr.recent_workspaces().iter().map(|w| {
                    serde_json::json!({ "path": w.path.to_string_lossy(), "name": w.name, "last_opened": w.last_opened })
                }).collect::<Vec<_>>()
            }))
        }
        // Header chip: branch + dirty count for the active workspace.
        // Not-a-repo → branch:null (workspace can legitimately be outside git).
        "git_info" => {
            match agent_context::git::git_snapshot(&mgr.workspace_root) {
                Ok(snap) => Ok(serde_json::json!({
                    "branch": snap.branch,
                    "dirty": snap.dirty_count(),
                })),
                Err(_) => Ok(serde_json::json!({ "branch": serde_json::Value::Null, "dirty": 0 })),
            }
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
                    "usage": mgr.store_usage(mgr.active_id),
                    "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
                        serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
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
                "usage": mgr.store_usage(mgr.active_id),
                "sessions": mgr.sidebar_rows().iter().map(|(id,t,p,a,r,pn)| {
                    serde_json::json!({"id":id,"title":t,"preview":p,"active":a,"running":r,"pinned":pn})
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
            for k in ["theme_mode","theme_id","accent","background","foreground","dark_accent","dark_background","dark_foreground","ui_font","code_font","contrast"] {
                if let Some(v) = p.get(k) {
                    obj.insert(k.into(), v.clone());
                }
            }
            let s: agent_kernel::AppearanceSettings =
                serde_json::from_value(serde_json::Value::Object(obj)).map_err(|e| e.to_string())?;
            agent_kernel::save_appearance_settings(&s).map_err(|e| e.to_string())?;
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
        // `+` attach button — native file picker, then `read_attachment`
        // per path. `kind` presets the filter list; picked paths come back
        // absolute and may live outside the workspace (unlike `@` mentions).
        "pick_attachments" => {
            let kind = payload
                .as_ref()
                .and_then(|p| p.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            let mut dlg = rfd::FileDialog::new().set_title("添加附件");
            dlg = match kind {
                "image" => dlg.add_filter(
                    "图片",
                    &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"],
                ),
                "text" => dlg.add_filter(
                    "文本",
                    &[
                        "txt", "md", "markdown", "json", "yaml", "yml", "toml",
                        "xml", "csv", "log", "rs", "ts", "tsx", "js", "jsx",
                        "py", "go", "java", "c", "cc", "cpp", "h", "hpp",
                        "css", "html", "sh", "sql", "ini", "conf", "env",
                    ],
                ),
                _ => dlg,
            };
            let picked = dlg.pick_files().unwrap_or_default();
            Ok(serde_json::json!(
                picked
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            ))
        }
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
            let staged = path.to_string_lossy().to_string();
            if let Some(img) = agent_llm::types::ImageRef::for_path(path.clone()) {
                return Ok(serde_json::json!({
                    "path": staged, "name": name, "kind": "image",
                    "data_url": img.data_url(),
                }));
            }
            const MAX_BYTES: usize = 32 * 1024;
            match std::fs::read(&path) {
                Ok(bytes) if bytes.contains(&0) => Ok(serde_json::json!({
                    "path": staged, "name": name, "kind": "binary",
                })),
                Ok(bytes) => {
                    let truncated = bytes.len() > MAX_BYTES;
                    let capped = if truncated { &bytes[..MAX_BYTES] } else { &bytes[..] };
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
    let dir = workspace_root.join(".husk").join("attachments");
    let gitignore = workspace_root.join(".husk").join(".gitignore");
    if std::fs::create_dir_all(&dir).is_ok() && !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "*\n");
    }
    // Content-hash name — re-picking the same file is idempotent, and two
    // different files sharing a filename never collide.
    let Ok(bytes) = std::fs::read(source) else {
        return source.to_path_buf();
    };
    let hash = xxhash_rust::xxh3::xxh3_64(&bytes);
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let ext = source
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let staged = dir.join(format!("{stem}-{hash:016x}{ext}"));
    if !staged.exists() && std::fs::write(&staged, &bytes).is_err() {
        return source.to_path_buf();
    }
    staged
}
