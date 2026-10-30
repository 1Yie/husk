//! Session-scoped activity operations, executed exclusively by the host owner.
use std::{collections::BTreeMap, path::Path};

use agent_paths::AppPaths;
use agent_store::SessionStore;
use husk_host::session_manager::SessionManager;
use serde_json::{Value, json};

use super::SessionKey;

#[derive(Debug, Clone)]
pub enum ActivityCommand {
    WorkspaceLinks {
        key: SessionKey,
    },
    LinkWorkspace {
        key: SessionKey,
        path: String,
        access: String,
        persist: bool,
    },
    UnlinkWorkspace {
        key: SessionKey,
        path: String,
    },
    SetWorkspaceAccess {
        key: SessionKey,
        path: String,
        access: String,
    },
    OfficeArtifacts {
        root: String,
    },
    Refresh {
        key: SessionKey,
    },
    JobControl {
        key: SessionKey,
        job: String,
        action: String,
    },
    Logs {
        key: SessionKey,
        job: String,
    },
    ChildHistory {
        key: SessionKey,
        child_id: u64,
    },
    SteerChild {
        key: SessionKey,
        child_id: u64,
        text: String,
    },
    ArtifactPreview {
        key: SessionKey,
        path: String,
    },
}

const WRITERS: &[&str] = &[
    "office_create",
    "office_set",
    "office_add",
    "office_remove",
    "office_move",
    "office_batch",
    "office_merge",
    "office_import",
    "office_save",
    "web_download",
];

fn store(
    manager: &SessionManager,
    key: &SessionKey,
) -> Result<std::sync::Arc<SessionStore>, String> {
    // Open an existing store only: a stale request must not create a workspace.
    let _ = manager;
    SessionStore::open_existing(Path::new(&key.root))
        .map(std::sync::Arc::new)
        .ok_or("会话存储不可用".into())
}

fn artifacts(manager: &SessionManager, key: &SessionKey) -> Result<Vec<Value>, String> {
    let store = store(manager, key)?;
    let rows = store
        .tool_rows(key.id, WRITERS)
        .map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for row in rows.into_iter().filter(|r| !r.is_error) {
        let Ok(args) = serde_json::from_str::<Value>(&row.args) else {
            continue;
        };
        let Some(path) = args
            .get("file")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("file_path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let path = Path::new(path);
        let relative = if path.is_absolute() {
            let Ok(relative) = path.strip_prefix(&key.root) else {
                continue;
            };
            relative
        } else {
            path
        };
        if !valid_relative(relative) {
            continue;
        }
        let path = relative.to_string_lossy().into_owned();
        files.insert(
            path.clone(),
            json!({"path":path,"tool":row.tool,"index":row.index}),
        );
    }
    Ok(files.into_values().collect())
}

fn valid_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn children(
    manager: &SessionManager,
    paths: &AppPaths,
    key: &SessionKey,
) -> Result<Vec<Value>, String> {
    let dir = husk_host::subagents::transcript_dir(&paths.workspace_dir(Path::new(&key.root)));
    let mut children = BTreeMap::<u64, Value>::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                let Some(id) = transcript_child(&name.to_string_lossy(), key.id) else {
                    continue;
                };
                children.insert(
                    id,
                    json!({
                        "child_id":id, "label":format!("子代理 {id}"),
                        "task":"", "state":"saved", "waiting":null,
                    }),
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    for child in manager.active_children(&key.root, key.id) {
        children.insert(
            child.child_id,
            json!({
                "child_id":child.child_id, "label":child.label, "task":child.task,
                "waiting":child.waiting, "state":"running",
            }),
        );
    }
    Ok(children.into_values().collect())
}

fn transcript_child(name: &str, session: i64) -> Option<u64> {
    let name = match name.split_once('-') {
        Some((stamp, rest)) if stamp.len() == 15 && stamp.bytes().all(|b| b.is_ascii_digit()) => {
            rest
        }
        _ => name,
    };
    name.strip_prefix(&format!("s{session}-c"))?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

fn snapshot(manager: &SessionManager, paths: &AppPaths, key: &SessionKey) -> Value {
    // A failure in one source must not hide the other two sources.
    let (artifacts, artifact_error) = match artifacts(manager, key) {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let (children, child_error) = match children(manager, paths, key) {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    json!({
        "kind":"refresh", "artifacts":artifacts, "artifact_error":artifact_error,
        "jobs":manager.bg_jobs_for_display(&key.root,key.id),
        "children":children,"child_error":child_error,
    })
}

pub(super) fn handle(
    manager: &mut SessionManager,
    paths: &AppPaths,
    command: ActivityCommand,
) -> Result<(SessionKey, Value), String> {
    match command {
        ActivityCommand::WorkspaceLinks { key } => {
            linked_session(manager, &key)?;
            Ok((key.clone(), workspace_links(manager, &key)))
        }
        ActivityCommand::LinkWorkspace {
            key,
            path,
            access,
            persist,
        } => {
            linked_session(manager, &key)?;
            link_access(&access)?;
            manager.link_root(&key.root, key.id, Path::new(&path), &access, persist)?;
            Ok((key.clone(), workspace_links(manager, &key)))
        }
        ActivityCommand::UnlinkWorkspace { key, path } => {
            linked_session(manager, &key)?;
            manager.unlink_root(&key.root, key.id, Path::new(&path))?;
            Ok((key.clone(), workspace_links(manager, &key)))
        }
        ActivityCommand::SetWorkspaceAccess { key, path, access } => {
            linked_session(manager, &key)?;
            link_access(&access)?;
            manager.set_link_access(&key.root, key.id, Path::new(&path), &access)?;
            Ok((key.clone(), workspace_links(manager, &key)))
        }
        ActivityCommand::OfficeArtifacts { root } => {
            if !husk_host::office::is_office_root(Path::new(&root)) {
                return Err("产物列表仅支持办公工作区".into());
            }
            let (artifacts, truncated) =
                crate::features::workspace::recent_artifacts(Path::new(&root))?;
            Ok((
                SessionKey { root, id: 0 },
                json!({
                    "kind":"office_artifacts","artifacts":artifacts,"truncated":truncated
                }),
            ))
        }
        ActivityCommand::Refresh { key } => {
            let value = snapshot(manager, paths, &key);
            Ok((key, value))
        }
        ActivityCommand::JobControl { key, job, action } => {
            if !matches!(action.as_str(), "pause" | "resume" | "kill" | "remove") {
                return Err("未知后台任务操作".into());
            }
            if action == "remove"
                && let Some(jobs) = manager.remove_bg_record(&key.root, key.id, &job)
            {
                return Ok((key, json!({"kind":"jobs","jobs":jobs})));
            }
            let reg = manager.bg_registry(&key.root, key.id).ok_or("会话未运行")?;
            match action.as_str() {
                "pause" => reg.set_paused(&job, true)?,
                "resume" => reg.set_paused(&job, false)?,
                "remove" => reg.remove(&job)?,
                "kill" => {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?
                        .block_on(reg.kill(&job))?;
                }
                _ => unreachable!(),
            }
            let jobs = manager.bg_jobs_for_display(&key.root, key.id);
            Ok((key, json!({"kind":"jobs","jobs":jobs})))
        }
        ActivityCommand::Logs { key, job } => {
            let saved = manager.bg_jobs_for_display(&key.root, key.id);
            if let Some(row) = saved.as_array().and_then(|rows| {
                rows.iter().find(|row| {
                    row["id"].as_str() == Some(job.as_str())
                        && row["restored"].as_bool() == Some(true)
                })
            }) {
                let text = row["log_tail"].as_str().ok_or("这条保存的记录没有日志")?;
                return Ok((key, json!({"kind":"logs","job":job,"text":text})));
            }
            let reg = manager
                .bg_registry(&key.root, key.id)
                .ok_or("会话未运行，无法读取实时日志")?;
            let text = reg.tail(&job, 200)?;
            Ok((key, json!({"kind":"logs","job":job,"text":text})))
        }
        ActivityCommand::ChildHistory { key, child_id } => {
            let dir =
                husk_host::subagents::transcript_dir(&paths.workspace_dir(Path::new(&key.root)));
            let messages = match husk_host::subagents::find_transcript(&dir, key.id, child_id) {
                Some(path) => husk_host::subagents::load_transcript(&path)?,
                None => Vec::new(),
            };
            Ok((
                key,
                json!({"kind":"child_history","child_id":child_id,"messages":messages}),
            ))
        }
        ActivityCommand::SteerChild {
            key,
            child_id,
            text,
        } => {
            if text.trim().is_empty() {
                return Err("指导内容不能为空".into());
            }
            // steer_active_child is deliberately active-session-only. Never
            // route a stale dock's action to a child belonging to another session.
            if manager.active_id != key.id || manager.workspace_root != Path::new(&key.root) {
                return Err("请先打开这个子代理所属的会话".into());
            }
            manager.steer_active_child(child_id, text)?;
            Ok((key, json!({"kind":"steered","child_id":child_id})))
        }
        ActivityCommand::ArtifactPreview { key, path } => {
            // Membership is rechecked from durable successful tool results,
            // not trusted from a UI-provided arbitrary path.
            if !artifacts(manager, &key)?
                .iter()
                .any(|row| row["path"].as_str() == Some(&path))
            {
                return Err("这个路径不是当前会话记录的产物".into());
            }
            let text = scoped_text(Path::new(&key.root), &path)?;
            Ok((
                key,
                json!({"kind":"artifact_preview","path":path,"text":text}),
            ))
        }
    }
}

fn linked_session(manager: &SessionManager, key: &SessionKey) -> Result<(), String> {
    let root = Path::new(&key.root);
    if !root.is_absolute() || !root.is_dir() || key.id <= 0 {
        return Err("关联目录所属的工作区或会话不存在".into());
    }
    if store(manager, key)?.meta(key.id).is_none() {
        return Err("关联目录所属的会话不存在".into());
    }
    Ok(())
}

fn link_access(access: &str) -> Result<(), String> {
    if matches!(access, "read" | "readwrite") {
        Ok(())
    } else {
        Err("未知目录权限".into())
    }
}

fn workspace_links(manager: &SessionManager, key: &SessionKey) -> Value {
    let (workspace, session) = manager.link_layers(&key.root, key.id);
    let roots: Vec<Value> = manager
        .effective_links(&key.root, key.id)
        .iter()
        .map(|root| {
            let mut scopes = Vec::new();
            if workspace.iter().any(|entry| entry.path == root.path) {
                scopes.push("workspace");
            }
            if session.iter().any(|entry| entry.path == root.path) {
                scopes.push("session");
            }
            json!({
                "path": root.path.to_string_lossy(),
                "access": root.access.as_str(),
                "scopes": scopes,
            })
        })
        .collect();
    let workspace: Vec<Value> = workspace
        .iter()
        .map(|root| husk_host::link_policy::root_json(root, "workspace"))
        .collect();
    let session: Vec<Value> = session
        .iter()
        .map(|root| husk_host::link_policy::root_json(root, "session"))
        .collect();
    json!({
        "kind": "workspace_links",
        "workspace": workspace,
        "session": session,
        "roots": roots,
    })
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn scoped_text(root: &Path, relative: &str) -> Result<String, String> {
    use rustix::fs::{Mode, OFlags, openat};
    use std::{fs::File, io::Read, path::Component};
    if !root.is_absolute() || !valid_relative(Path::new(relative)) {
        return Err("无效的工作区相对路径".into());
    }
    let open = |parent: &File, name: &Path, directory: bool| -> Result<File, String> {
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if directory {
            flags |= OFlags::DIRECTORY
        }
        openat(parent, name, flags, Mode::empty())
            .map(File::from)
            .map_err(|e| e.to_string())
    };
    let mut file = File::open("/").map_err(|e| e.to_string())?;
    for part in root.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => file = open(&file, Path::new(name), true)?,
            _ => return Err("无效工作区路径".into()),
        }
    }
    let parts = Path::new(relative).components().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        file = open(&file, Path::new(part.as_os_str()), index + 1 != parts.len())?;
    }
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("只允许预览普通文件".into());
    }
    if meta.len() > 2 * 1024 * 1024 {
        return Err("文件超过 2 MiB，不能在 Dock 中预览".into());
    }
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("文件超过预览上限".into());
    }
    if bytes.contains(&0) {
        return Err("二进制产物不能作为文本预览".into());
    }
    String::from_utf8(bytes).map_err(|_| "此产物不是 UTF-8 文本，不能作为文本预览".into())
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn scoped_text(_: &Path, _: &str) -> Result<String, String> {
    Err("当前平台暂不支持安全的产物预览".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_identity_is_session_scoped() {
        assert_eq!(transcript_child("s12-c34.jsonl", 12), Some(34));
        assert_eq!(transcript_child("s112-c34.jsonl", 12), None);
        assert_eq!(transcript_child("c34.jsonl", 12), None);
        assert_eq!(
            transcript_child("000000000000001-s12-c34.jsonl", 12),
            Some(34)
        );
        assert_eq!(transcript_child("000000000000001-s112-c34.jsonl", 12), None);
        assert_eq!(transcript_child("s12-c../34.jsonl", 12), None);
    }
    #[test]
    fn reject_unscoped_paths() {
        for path in ["", "/tmp/a", "../a", "dir/../../a"] {
            assert!(!valid_relative(Path::new(path)));
        }
        assert!(valid_relative(Path::new("docs/report.md")));
    }
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn preview_rejects_links_and_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.txt"), "actual output").unwrap();
        assert_eq!(
            scoped_text(dir.path(), "report.txt").unwrap(),
            "actual output"
        );
        std::os::unix::fs::symlink("report.txt", dir.path().join("link")).unwrap();
        assert!(scoped_text(dir.path(), "link").is_err());
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "not in workspace").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked-dir")).unwrap();
        assert!(scoped_text(dir.path(), "linked-dir/secret").is_err());
        std::fs::write(dir.path().join("binary"), [0, 1, 2]).unwrap();
        assert!(scoped_text(dir.path(), "binary").is_err());
        assert!(scoped_text(dir.path(), "../report.txt").is_err());
    }
}
