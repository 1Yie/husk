//! Read old databases through a verified, private snapshot, never by acquiring
//! their live application lock. Source files are not changed or renamed.

use super::*;
use std::io::Write;

pub(super) fn import(
    db: &Arc<SessionDatabase>,
    parent: &Path,
    source: Option<&Path>,
) -> std::io::Result<()> {
    files::locked(&db.root.join("migration.lock"), || {
        let marker = db.root.join("migration.json");
        if marker.exists() {
            return Ok(());
        }
        let mut workspaces = 0;
        let mut sessions = 0;
        if let Some(source) = source.filter(|p| p.is_file()) {
            let snapshot = stable_snapshot(source)?;
            // Recovery may modify only this private copy. The user's live
            // database remains owned by its original process.
            let legacy = redb::Database::open(snapshot.path()).map_err(io_err)?;
            let read = legacy.begin_read().map_err(io_err)?;
            fn entries(
                read: &redb::ReadTransaction,
                table: TableDefinition<&str, &[u8]>,
            ) -> std::io::Result<Vec<(String, Vec<u8>)>> {
                let t = match read.open_table(table) {
                    Ok(t) => t,
                    Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
                    Err(e) => return Err(io_err(e)),
                };
                t.iter()
                    .map_err(io_err)?
                    .map(|e| {
                        e.map(|(k, v)| (k.value().to_string(), v.value().to_vec()))
                            .map_err(io_err)
                    })
                    .collect()
            }
            let rows = entries(&read, WORKSPACES)?;
            let metadata = entries(&read, METAS)?;
            let histories = entries(&read, HISTORY)?;
            let states = entries(&read, STATE)?;
            let prefs = entries(&read, PREFS)?;
            let globals = entries(&read, GLOBALS)?;
            // Validate all structured input before starting the import.
            for (_, bytes) in &rows {
                let _: WorkspaceRow = serde_json::from_slice(bytes)?;
            }
            for (key, bytes) in &metadata {
                let (_, id) = split(key)?;
                let meta: SessionMeta = serde_json::from_slice(bytes)?;
                if meta.id != id {
                    return Err(io_err("legacy session key/id mismatch"));
                }
            }
            for (_, bytes) in &histories {
                let _: Vec<ChatMessage> = serde_json::from_slice(bytes)?;
            }
            for (_, bytes) in &prefs {
                let _: WorkspacePrefs = serde_json::from_slice(bytes)?;
            }
            for (_, bytes) in &globals {
                let _: serde_json::Value = serde_json::from_slice(bytes)?;
            }
            for (ws, bytes) in &rows {
                files::atomic_json(
                    &db.root.join(ws).join("workspace.json"),
                    &serde_json::from_slice::<WorkspaceRow>(bytes)?,
                )?;
            }
            for (key, bytes) in &metadata {
                let (ws, _) = split(key)?;
                let store = bind(db, &ws)?;
                store.upsert_meta(serde_json::from_slice(bytes)?)?;
            }
            for (key, bytes) in &histories {
                let (ws, id) = split(key)?;
                let store = bind(db, &ws)?;
                let history: Vec<ChatMessage> = serde_json::from_slice(bytes)?;
                store.snapshot(id, &history)?;
                if serde_json::to_vec(
                    &store
                        .load_history(id)
                        .ok_or_else(|| io_err("missing imported history"))?,
                )? != serde_json::to_vec(&history)?
                {
                    return Err(io_err("history import verification failed"));
                }
            }
            for (key, bytes) in &states {
                let (session, name) = key
                    .rsplit_once('/')
                    .ok_or_else(|| io_err("invalid legacy state key"))?;
                let (ws, id) = split(session)?;
                let store = bind(db, &ws)?;
                store.write_state(id, name, bytes)?;
                if store.read_state(id, name)?.as_deref() != Some(bytes) {
                    return Err(io_err("state import verification failed"));
                }
            }
            for (ws, bytes) in &prefs {
                bind(db, ws)?.save_prefs(&serde_json::from_slice::<WorkspacePrefs>(bytes)?)?;
            }
            for (key, bytes) in &globals {
                safe_name(key)?;
                files::put(
                    db,
                    key,
                    &serde_json::from_slice::<serde_json::Value>(bytes)?,
                )?;
            }
            workspaces = rows.len();
            sessions = metadata.len();
        } else {
            let legacy = parent.join("sessions");
            if legacy.is_dir() {
                for entry in std::fs::read_dir(legacy)? {
                    let entry = entry?;
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let ws = entry.file_name().to_string_lossy().to_string();
                    let dir = entry.path();
                    let store = bind(db, &ws)?;
                    if dir.join("workspace.json").is_file() {
                        let row: WorkspaceRow = serde_json::from_reader(std::fs::File::open(
                            dir.join("workspace.json"),
                        )?)?;
                        files::atomic_json(&db.root.join(&ws).join("workspace.json"), &row)?;
                    }
                    if dir.join("index.json").is_file() {
                        let index: SessionIndex =
                            serde_json::from_reader(std::fs::File::open(dir.join("index.json"))?)?;
                        if let Some(id) = index.last_active {
                            store.set_last_active(id)?;
                        }
                        sessions += index.sessions.len();
                        for meta in index.sessions {
                            store.upsert_meta(meta)?;
                        }
                    }
                    if dir.join("prefs.json").is_file() {
                        store.save_prefs(&serde_json::from_reader::<_, WorkspacePrefs>(
                            std::fs::File::open(dir.join("prefs.json"))?,
                        )?)?;
                    }
                    for file in std::fs::read_dir(dir)? {
                        let file = file?;
                        let name = file.file_name().to_string_lossy().to_string();
                        if let Some(id) = name
                            .strip_suffix(".jsonl")
                            .and_then(|id| id.parse::<i64>().ok())
                        {
                            let bytes = last_intact_jsonl_line(&file.path())
                                .ok_or_else(|| io_err("invalid JSONL history"))?;
                            store.snapshot(
                                id,
                                &serde_json::from_slice::<Vec<ChatMessage>>(&bytes)?,
                            )?;
                        } else if let Some((id, name)) = name.split_once('.') {
                            if let Ok(id) = id.parse::<i64>() {
                                store.write_state(id, name, &std::fs::read(file.path())?)?;
                            }
                        }
                    }
                    workspaces += 1;
                }
            }
            for (file, key) in [
                ("recent_workspaces.json", "recent_workspaces"),
                ("default_preferences.json", "default_preferences"),
                ("appearance.json", "appearance"),
            ] {
                let file = parent.join(file);
                if file.is_file() {
                    let value: serde_json::Value =
                        serde_json::from_reader(std::fs::File::open(file)?)?;
                    files::put(db, key, &value)?;
                }
            }
        }
        files::atomic_json(
            &marker,
            &serde_json::json!({"format":"jsonl-v1","workspaces":workspaces,"sessions":sessions,"source":source}),
        )
    })
}

fn safe_name(name: &str) -> std::io::Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(io_err("invalid legacy namespace"));
    }
    Ok(())
}
fn split(key: &str) -> std::io::Result<(String, i64)> {
    let (ws, id) = key
        .rsplit_once('/')
        .ok_or_else(|| io_err("invalid legacy session key"))?;
    safe_name(ws)?;
    Ok((ws.into(), id.parse().map_err(io_err)?))
}
fn bind(db: &Arc<SessionDatabase>, ws: &str) -> std::io::Result<SessionStore> {
    safe_name(ws)?;
    let store = SessionStore {
        ws: ws.into(),
        db: db.clone(),
    };
    let path = db.root.join(ws).join("workspace.json");
    if !path.exists() {
        files::atomic_json(&path, &WorkspaceRow::default())?;
    }
    Ok(store)
}

fn stable_snapshot(path: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    for _ in 0..4 {
        let before = std::fs::metadata(path)?;
        let bytes = std::fs::read(path)?;
        let after = std::fs::metadata(path)?;
        if before.len() != after.len() || before.modified()? != after.modified()? {
            continue;
        }
        let verify = std::fs::read(path)?;
        if bytes != verify {
            continue;
        }
        let mut snapshot = tempfile::NamedTempFile::new()?;
        snapshot.write_all(&bytes)?;
        snapshot.as_file().sync_all()?;
        return Ok(snapshot);
    }
    Err(io_err(
        "旧会话文件持续变化，无法取得稳定快照；稍后重试即可，无需关闭其他窗口",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_live_locked_redb_without_changing_source_or_owner() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sessions.db");
        let owner = redb::Database::create(&source).unwrap();
        let tx = owner.begin_write().unwrap();
        for def in [METAS, HISTORY, STATE, PREFS, WORKSPACES, GLOBALS] {
            tx.open_table(def).unwrap();
        }
        tx.open_table(WORKSPACES)
            .unwrap()
            .insert("ws", b"{\"root\":\"/project\",\"next_id\":3}".as_slice())
            .unwrap();
        tx.open_table(METAS)
            .unwrap()
            .insert(
                "ws/3",
                b"{\"id\":3,\"title\":\"old\",\"preview\":\"\",\"updated_at\":2}".as_slice(),
            )
            .unwrap();
        let history = serde_json::to_vec(&vec![ChatMessage::user("old message")]).unwrap();
        tx.open_table(HISTORY)
            .unwrap()
            .insert("ws/3", history.as_slice())
            .unwrap();
        tx.commit().unwrap();
        let original = std::fs::read(&source).unwrap();
        let db = SessionDatabase::open(&dir.path().join("conversations")).unwrap();
        import(&db, dir.path(), Some(&source)).unwrap();
        let store = bind(&db, "ws").unwrap();
        assert_eq!(
            store.load_history(3).unwrap()[0].content.as_deref(),
            Some("old message")
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let tx = owner.begin_write().unwrap();
        tx.open_table(STATE)
            .unwrap()
            .insert("still-running", b"ok".as_slice())
            .unwrap();
        tx.commit().unwrap();
        store.update_queue(3, vec!["new queue".into()]).unwrap();
        import(&db, dir.path(), Some(&source)).unwrap();
        assert_eq!(store.list()[0].queued_prompts, ["new queue"]);
    }
}
