//! `agent_session` — session-list + switch/create commands for the sidebar.

use tauri::State;

use crate::kernel::KernelState;

#[tauri::command]
pub fn agent_session(
    state: State<'_, KernelState>,
    op: String,
    id: Option<i64>,
    path: Option<String>,
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
        _ => Err(format!("unknown session op: {op}")),
    }
}
