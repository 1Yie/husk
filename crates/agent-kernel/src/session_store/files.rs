//! Independent JSONL journals. Locks are held only for one file transaction,
//! never for an application's lifetime; unrelated sessions never share a lock.

use super::*;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

pub struct SessionDatabase {
    pub(super) root: PathBuf,
    _temporary: Option<tempfile::TempDir>,
}

impl SessionDatabase {
    pub fn open(path: &Path) -> std::io::Result<Arc<Self>> {
        std::fs::create_dir_all(path)?;
        Ok(Arc::new(Self {
            root: path.to_path_buf(),
            _temporary: None,
        }))
    }
    pub(super) fn memory() -> std::io::Result<Arc<Self>> {
        let dir = tempfile::tempdir()?;
        Ok(Arc::new(Self {
            root: dir.path().to_path_buf(),
            _temporary: Some(dir),
        }))
    }
}

pub(super) fn atomic_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io_err("missing parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(tmp.as_file_mut(), value)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn locked<T>(path: &Path, f: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    std::fs::create_dir_all(path.parent().ok_or_else(|| io_err("missing lock parent"))?)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    // Busy writers block briefly, with an explicit upper bound so a wedged
    // writer cannot indefinitely freeze an IPC operation.
    let start = std::time::Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    && start.elapsed() < std::time::Duration::from_secs(5) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(e) => return Err(e),
        }
    }
    let result = f();
    FileExt::unlock(&file)?;
    result
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> std::io::Result<Option<T>> {
    match File::open(path) {
        Ok(file) => serde_json::from_reader(file).map(Some).map_err(io_err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Location {
    offset: u64,
    hash: u64,
    turn: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct JournalIndex {
    bytes: u64,
    messages: Vec<Location>,
    meta: Option<SessionMeta>,
    has_history: bool,
    deleted: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    History { from: usize },
    Message { message: ChatMessage },
    Metadata { meta: SessionMeta },
    Deleted,
    Commit { has_history: bool },
}

fn is_turn(message: &ChatMessage) -> bool {
    message.role == agent_llm::Role::User
        && message.notice != Some(agent_llm::types::NoticeKind::Hidden)
}
fn journal_index(journal: &Path, index: &Path) -> std::io::Result<JournalIndex> {
    let file = match File::open(journal) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(JournalIndex::default()),
        Err(e) => return Err(e),
    };
    let length = file.metadata()?.len();
    let cached: Option<JournalIndex> = read_json(index).unwrap_or(None);
    let mut committed = cached.filter(|c| c.bytes <= length).unwrap_or_default();
    let mut pending = committed.clone();
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(committed.bytes))?;
    let mut offset = committed.bytes;
    let mut line = Vec::new();
    loop {
        line.clear();
        let len = reader.read_until(b'\n', &mut line)?;
        if len == 0 || !line.ends_with(b"\n") {
            break;
        }
        let record: Record = serde_json::from_slice(&line)
            .map_err(|e| io_err(format!("{} at byte {offset}: {e}", journal.display())))?;
        match record {
            Record::History { from } => {
                if from > pending.messages.len() {
                    return Err(io_err("history journal has invalid sequence"));
                }
                pending.messages.truncate(from);
            }
            Record::Message { message } => pending.messages.push(Location {
                offset,
                hash: xxhash_rust::xxh3::xxh3_64(&serde_json::to_vec(&message)?),
                turn: is_turn(&message),
            }),
            Record::Metadata { meta } => pending.meta = Some(meta),
            Record::Deleted => {
                pending.deleted = true;
                pending.messages.clear();
                pending.meta = None;
            }
            Record::Commit { has_history } => {
                pending.bytes = offset + len as u64;
                pending.has_history = has_history;
                committed = pending.clone();
            }
        }
        offset += len as u64;
    }
    Ok(committed)
}

struct JournalWriter {
    file: File,
    index: JournalIndex,
    index_path: PathBuf,
}
impl JournalWriter {
    fn record(&mut self, record: Record) -> std::io::Result<()> {
        let offset = self.file.stream_position()?;
        if let Record::Message { message } = &record {
            self.index.messages.push(Location {
                offset,
                hash: xxhash_rust::xxh3::xxh3_64(&serde_json::to_vec(message)?),
                turn: is_turn(message),
            });
        }
        serde_json::to_writer(&mut self.file, &record)?;
        self.file.write_all(b"\n")?;
        Ok(())
    }
    fn history(&mut self, history: &[ChatMessage]) -> std::io::Result<()> {
        let mut first = 0;
        while first < history.len() && first < self.index.messages.len() {
            if xxhash_rust::xxh3::xxh3_64(&serde_json::to_vec(&history[first])?)
                != self.index.messages[first].hash
            {
                break;
            }
            first += 1;
        }
        if first != history.len() || first != self.index.messages.len() {
            self.record(Record::History { from: first })?;
            self.index.messages.truncate(first);
            for message in &history[first..] {
                self.record(Record::Message {
                    message: message.clone(),
                })?;
            }
        }
        self.index.has_history = true;
        Ok(())
    }
    fn meta(&mut self, meta: SessionMeta) -> std::io::Result<()> {
        self.record(Record::Metadata { meta: meta.clone() })?;
        self.index.meta = Some(meta);
        Ok(())
    }
    fn commit(mut self) -> std::io::Result<()> {
        self.record(Record::Commit {
            has_history: self.index.has_history,
        })?;
        self.file.sync_all()?;
        self.index.bytes = self.file.stream_position()?;
        atomic_json(&self.index_path, &self.index)
    }
}

impl SessionStore {
    fn directory(&self) -> PathBuf {
        self.db.root.join(&self.ws)
    }
    fn journal(&self, id: i64) -> PathBuf {
        self.directory().join(format!("{id}.jsonl"))
    }
    fn index(&self, id: i64) -> PathBuf {
        self.directory().join(format!("{id}.index.json"))
    }
    fn read_index(&self, id: i64) -> std::io::Result<JournalIndex> {
        journal_index(&self.journal(id), &self.index(id))
    }
    fn write<T>(
        &self,
        id: i64,
        f: impl FnOnce(&mut JournalWriter) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        locked(&self.directory().join(format!("{id}.lock")), || {
            let index = self.read_index(id)?;
            if index.deleted {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "session was deleted",
                ));
            }
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(self.journal(id))?;
            file.set_len(index.bytes)?;
            file.seek(SeekFrom::Start(index.bytes))?;
            let mut writer = JournalWriter {
                file,
                index,
                index_path: self.index(id),
            };
            let result = f(&mut writer)?;
            writer.commit()?;
            Ok(result)
        })
    }
    pub fn open(root: &Path) -> std::io::Result<Self> {
        Self::open_with(root, try_global_db()?)
    }
    pub(crate) fn open_or_transient(root: &Path) -> std::io::Result<Self> {
        let db = {
            let mut shared = shared_db().lock().map_err(io_err)?;
            if shared.db.is_none() {
                match open_persistent_db() {
                    Ok(db) => shared.db = Some(db),
                    Err(error) => {
                        let warning = format!(
                            "会话文件暂不可用: {error}；当前会话只保存到临时目录，退出后不会保留。"
                        );
                        eprintln!("[session storage] {warning}");
                        shared.warning = Some(warning);
                        shared.db = Some(SessionDatabase::memory()?);
                    }
                }
            }
            shared.db.as_ref().unwrap().clone()
        };
        Self::open_with(root, db)
    }
    pub fn open_with(root: &Path, db: Arc<SessionDatabase>) -> std::io::Result<Self> {
        let store = Self {
            ws: workspace_key(root),
            db,
        };
        locked(&store.directory().join("workspace.lock"), || {
            let path = store.directory().join("workspace.json");
            let mut row: WorkspaceRow = read_json(&path)?.unwrap_or_default();
            if row.root.is_none() {
                row.root = Some(root.to_string_lossy().into());
                atomic_json(&path, &row)?;
            }
            Ok(())
        })?;
        Ok(store)
    }
    pub fn open_at(root: &Path, path: &Path) -> std::io::Result<Self> {
        Self::open_with(root, open_db(path)?)
    }
    pub fn open_existing(root: &Path) -> Option<Self> {
        let store = Self {
            ws: workspace_key(root),
            db: global_db()?,
        };
        store.workspace_row().ok()??;
        Some(store)
    }
    pub fn all_workspaces() -> Vec<Self> {
        global_db().map(Self::all_on).unwrap_or_default()
    }
    pub fn all_workspaces_at(path: &Path) -> Vec<Self> {
        open_db(path).map(Self::all_on).unwrap_or_default()
    }
    fn all_on(db: Arc<SessionDatabase>) -> Vec<Self> {
        std::fs::read_dir(&db.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("workspace.json").is_file())
            .map(|e| Self {
                ws: e.file_name().to_string_lossy().into(),
                db: db.clone(),
            })
            .collect()
    }
    pub fn dir_name(&self) -> Option<String> {
        Some(self.ws.clone())
    }
    pub(super) fn workspace_row(&self) -> std::io::Result<Option<WorkspaceRow>> {
        read_json(&self.directory().join("workspace.json"))
    }
    pub fn recorded_root(&self) -> Option<PathBuf> {
        self.workspace_row()
            .ok()
            .flatten()
            .and_then(|r| r.root)
            .map(PathBuf::from)
    }
    pub fn record_root(&self, root: &Path) {
        let _ = self.workspace_update(|row| {
            if row.root.is_none() {
                row.root = Some(root.to_string_lossy().into_owned());
            }
        });
    }
    pub fn recover_root_from_history(&self) -> Option<PathBuf> {
        for meta in self.list().iter().take(4) {
            for msg in self.load_history_page(meta.id, Some(2), 2).ok()?.0 {
                if let Some(rest) = msg
                    .content
                    .as_deref()
                    .and_then(|c| c.split("Workspace root: ").nth(1))
                {
                    let p = rest.split(['"', '\n']).next()?.trim();
                    if !p.is_empty() {
                        return Some(p.into());
                    }
                }
            }
        }
        None
    }
    fn state_path(&self, id: i64, name: &str) -> PathBuf {
        let name = name
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        self.directory().join(format!("{id}.state")).join(name)
    }
    pub fn read_state(&self, id: i64, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        if self.is_deleted(id) {
            return Ok(None);
        }
        read_json(&self.state_path(id, name))
    }
    pub fn write_state(&self, id: i64, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.write(id, |_| atomic_json(&self.state_path(id, name), &bytes))
    }
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut metas: Vec<_> = std::fs::read_dir(self.directory())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".jsonl")?
                    .parse::<i64>()
                    .ok()
            })
            .filter_map(|id| self.read_index(id).ok().filter(|i| !i.deleted)?.meta)
            .collect();
        metas.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        metas
    }
    pub fn set_pinned(&self, id: i64, pinned: bool) -> std::io::Result<bool> {
        self.write(id, |w| {
            let Some(mut meta) = w.index.meta.clone() else {
                return Ok(false);
            };
            meta.pinned = pinned;
            w.meta(meta)?;
            Ok(true)
        })
    }
    pub fn last_active(&self) -> Option<i64> {
        self.workspace_row()
            .ok()
            .flatten()
            .and_then(|r| r.last_active)
    }
    fn workspace_update<T>(&self, f: impl FnOnce(&mut WorkspaceRow) -> T) -> std::io::Result<T> {
        locked(&self.directory().join("workspace.lock"), || {
            let mut row = self.workspace_row()?.unwrap_or_default();
            let value = f(&mut row);
            atomic_json(&self.directory().join("workspace.json"), &row)?;
            Ok(value)
        })
    }
    pub fn set_last_active(&self, id: i64) -> std::io::Result<()> {
        self.workspace_update(|r| r.last_active = Some(id))
    }
    pub fn ever_used(&self) -> bool {
        self.workspace_row()
            .ok()
            .flatten()
            .is_some_and(|r| r.next_id > 0)
    }
    pub fn load_history(&self, id: i64) -> Option<Vec<ChatMessage>> {
        let index = self.read_index(id).ok()?;
        if index.deleted || !index.has_history {
            return None;
        }
        self.messages(id, &index.messages).ok()
    }

    pub fn has_history(&self, id: i64) -> std::io::Result<bool> {
        let index = self.read_index(id)?;
        Ok(index.has_history && !index.deleted)
    }
    fn messages(&self, id: i64, locations: &[Location]) -> std::io::Result<Vec<ChatMessage>> {
        let mut reader = BufReader::new(File::open(self.journal(id))?);
        let mut messages = Vec::with_capacity(locations.len());
        let mut line = Vec::new();
        for location in locations {
            reader.seek(SeekFrom::Start(location.offset))?;
            line.clear();
            reader.read_until(b'\n', &mut line)?;
            match serde_json::from_slice::<Record>(&line)? {
                Record::Message { message } => messages.push(message),
                _ => return Err(io_err("invalid message offset")),
            }
        }
        Ok(messages)
    }
    pub fn load_history_page(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
    ) -> std::io::Result<(Vec<ChatMessage>, usize, usize, usize)> {
        let index = self.read_index(id)?;
        if index.deleted {
            return Ok((vec![], 0, 0, 0));
        }
        let total = index.messages.len();
        let end = before.unwrap_or(total).min(total);
        let start = end.saturating_sub(count);
        let turns = index.messages.iter().filter(|m| m.turn).count();
        let offset = index.messages[..start].iter().filter(|m| m.turn).count();
        let page = if start == end {
            vec![]
        } else {
            self.messages(id, &index.messages[start..end])?
        };
        Ok((page, total, turns, offset))
    }
    pub fn snapshot(&self, id: i64, history: &[ChatMessage]) -> std::io::Result<()> {
        self.write(id, |w| w.history(history))
    }
    pub fn upsert_meta(&self, meta: SessionMeta) -> std::io::Result<()> {
        self.write(meta.id, |w| w.meta(meta))
    }
    pub fn commit_turn(
        &self,
        history: &[ChatMessage],
        mut meta: SessionMeta,
    ) -> std::io::Result<()> {
        self.write(meta.id, |w| {
            if let Some(old) = &w.index.meta {
                meta.pinned = old.pinned;
                meta.queued_prompts = old.queued_prompts.clone();
                if meta.usage.is_none() {
                    meta.usage = old.usage;
                }
            }
            w.history(history)?;
            w.meta(meta)
        })
    }

    pub fn history_revision_of(history: &[ChatMessage]) -> u64 {
        if history.is_empty() {
            return 0;
        }
        let hashes: Vec<u8> = history
            .iter()
            .flat_map(|m| {
                xxhash_rust::xxh3::xxh3_64(&serde_json::to_vec(m).unwrap_or_default()).to_le_bytes()
            })
            .collect();
        xxhash_rust::xxh3::xxh3_64(&hashes)
    }

    pub fn commit_turn_checked(
        &self,
        history: &[ChatMessage],
        mut meta: SessionMeta,
        expected: u64,
    ) -> std::io::Result<u64> {
        self.write(meta.id, |w| {
            let actual = if w.index.messages.is_empty() {
                0
            } else {
                let hashes: Vec<u8> = w
                    .index
                    .messages
                    .iter()
                    .flat_map(|m| m.hash.to_le_bytes())
                    .collect();
                xxhash_rust::xxh3::xxh3_64(&hashes)
            };
            if actual != expected {
                return Err(io_err(
                    "会话已被另一个窗口更新，请重新打开此会话；当前结果未覆盖其他窗口的记录",
                ));
            }
            if let Some(old) = &w.index.meta {
                meta.pinned = old.pinned;
                meta.queued_prompts = old.queued_prompts.clone();
                if meta.usage.is_none() {
                    meta.usage = old.usage;
                }
            }
            w.history(history)?;
            w.meta(meta)?;
            Ok(Self::history_revision_of(history))
        })
    }
    pub fn update_settings(&self, meta: SessionMeta) -> std::io::Result<()> {
        self.write(meta.id, |w| {
            let mut current = w.index.meta.clone().unwrap_or_else(|| meta.clone());
            current.model = meta.model;
            current.provider = meta.provider;
            current.permission_mode = meta.permission_mode;
            current.agent_mode = meta.agent_mode;
            current.thinking_level = meta.thinking_level;
            current.updated_at = meta.updated_at;
            w.meta(current)
        })
    }
    pub fn update_queue(&self, id: i64, items: Vec<String>) -> std::io::Result<()> {
        self.write(id, |w| {
            let mut meta = w.index.meta.clone().unwrap_or_else(|| empty_meta(id));
            meta.queued_prompts = items;
            w.meta(meta)
        })
    }
    pub fn next_id(&self) -> i64 {
        self.workspace_update(|row| {
            let max = std::fs::read_dir(self.directory())
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".jsonl")?
                        .parse::<i64>()
                        .ok()
                })
                .max()
                .unwrap_or(0);
            row.next_id = row.next_id.max(max) + 1;
            row.next_id
        })
        .expect("allocate session id")
    }
    pub fn remove(&self, id: i64) -> std::io::Result<()> {
        if !self.is_deleted(id) {
            self.write(id, |w| {
                w.record(Record::Deleted)?;
                w.index.deleted = true;
                w.index.messages.clear();
                w.index.meta = None;
                w.index.has_history = false;
                Ok(())
            })?;
        }
        self.workspace_update(|r| {
            if r.last_active == Some(id) {
                r.last_active = None
            }
        })?;
        let state = self.directory().join(format!("{id}.state"));
        if state.exists() {
            std::fs::remove_dir_all(state)?;
        }
        Ok(())
    }
    pub fn is_deleted(&self, id: i64) -> bool {
        self.read_index(id).map(|i| i.deleted).unwrap_or(true)
    }
    pub fn delete_workspace(&self) -> std::io::Result<()> {
        for meta in self.list() {
            self.remove(meta.id)?;
        }
        let path = self.directory().join("workspace.json");
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
    pub fn load_prefs(&self) -> WorkspacePrefs {
        read_json(&self.directory().join("prefs.json"))
            .ok()
            .flatten()
            .unwrap_or_default()
    }
    pub fn save_prefs(&self, prefs: &WorkspacePrefs) -> std::io::Result<()> {
        locked(&self.directory().join("prefs.lock"), || {
            atomic_json(&self.directory().join("prefs.json"), prefs)
        })
    }
}

fn empty_meta(id: i64) -> SessionMeta {
    SessionMeta {
        id,
        title: "新会话".into(),
        preview: String::new(),
        updated_at: 0,
        pinned: false,
        usage: None,
        model: None,
        provider: None,
        permission_mode: None,
        agent_mode: None,
        thinking_level: None,
        queued_prompts: vec![],
    }
}

pub(super) fn get<T: serde::de::DeserializeOwned>(db: &SessionDatabase, key: &str) -> Option<T> {
    read_json(&db.root.join("globals").join(format!("{key}.json")))
        .ok()
        .flatten()
}
pub(super) fn put(db: &SessionDatabase, key: &str, value: &impl Serialize) -> std::io::Result<()> {
    locked(&db.root.join("globals").join(format!("{key}.lock")), || {
        atomic_json(&db.root.join("globals").join(format!("{key}.json")), value)
    })
}
pub(super) fn update_global<T: serde::de::DeserializeOwned + Serialize + Default>(
    db: &SessionDatabase,
    key: &str,
    f: impl FnOnce(&mut T),
) -> std::io::Result<()> {
    locked(&db.root.join("globals").join(format!("{key}.lock")), || {
        let path = db.root.join("globals").join(format!("{key}.json"));
        let mut value = read_json(&path)?.unwrap_or_default();
        f(&mut value);
        atomic_json(&path, &value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        (dir, store)
    }

    #[test]
    fn append_preserves_prior_bytes_and_reads_selected_payloads_only() {
        let (_dir, store) = store();
        let mut history: Vec<_> = (0..100)
            .flat_map(|i| {
                [
                    ChatMessage::user(format!("q{i}")),
                    ChatMessage::assistant(format!("a{i}")),
                ]
            })
            .collect();
        store.snapshot(1, &history).unwrap();
        let prefix = std::fs::read(store.journal(1)).unwrap();
        history.push(ChatMessage::user("next"));
        store.snapshot(1, &history).unwrap();
        let current = std::fs::read(store.journal(1)).unwrap();
        assert!(current.starts_with(&prefix));
        let (page, total, turns, offset) = store.load_history_page(1, None, 10).unwrap();
        assert_eq!((page.len(), total, turns, offset), (10, 201, 101, 96));
        assert_eq!(page.last().unwrap().content.as_deref(), Some("next"));
        let index = store.read_index(1).unwrap();
        let first = index.messages[0].offset;
        let mut file = OpenOptions::new()
            .write(true)
            .open(store.journal(1))
            .unwrap();
        file.seek(SeekFrom::Start(first)).unwrap();
        file.write_all(b"!invalid!").unwrap();
        assert_eq!(store.load_history_page(1, None, 10).unwrap().0.len(), 10);
        assert!(store.load_history(1).is_none());
    }

    #[test]
    fn lost_index_is_rebuilt_and_uncommitted_or_torn_tail_is_ignored() {
        let (_dir, store) = store();
        store.snapshot(1, &[ChatMessage::user("durable")]).unwrap();
        let committed = std::fs::read(store.journal(1)).unwrap();
        std::fs::remove_file(store.index(1)).unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(store.journal(1))
            .unwrap();
        serde_json::to_writer(&mut file, &Record::History { from: 0 }).unwrap();
        file.write_all(b"\n").unwrap();
        serde_json::to_writer(
            &mut file,
            &Record::Message {
                message: ChatMessage::user("uncommitted"),
            },
        )
        .unwrap();
        file.write_all(b"\n{\"type\":").unwrap();
        drop(file);
        assert_eq!(
            store.load_history(1).unwrap()[0].content.as_deref(),
            Some("durable")
        );
        store
            .snapshot(
                1,
                &[
                    ChatMessage::user("durable"),
                    ChatMessage::assistant("committed"),
                ],
            )
            .unwrap();
        let bytes = std::fs::read(store.journal(1)).unwrap();
        assert!(bytes.starts_with(&committed));
        assert_eq!(store.load_history(1).unwrap().len(), 2);
    }

    #[test]
    fn stale_index_replays_a_completed_transaction_after_crash() {
        let (_dir, store) = store();
        store.snapshot(1, &[ChatMessage::user("a")]).unwrap();
        let old_index = std::fs::read(store.index(1)).unwrap();
        store
            .snapshot(1, &[ChatMessage::user("a"), ChatMessage::assistant("b")])
            .unwrap();
        std::fs::write(store.index(1), old_index).unwrap();
        assert_eq!(store.load_history(1).unwrap().len(), 2);
    }

    #[test]
    fn history_rewind_and_clear_leave_only_current_view_in_index() {
        let (_dir, store) = store();
        store
            .snapshot(1, &[ChatMessage::user("a"), ChatMessage::assistant("b")])
            .unwrap();
        store
            .snapshot(1, &[ChatMessage::notice("cleared")])
            .unwrap();
        assert_eq!(store.load_history(1).unwrap().len(), 1);
        assert_eq!(
            store.load_history(1).unwrap()[0].content.as_deref(),
            Some("cleared")
        );
    }

    #[test]
    fn fields_updates_and_tombstones_survive_parallel_repository_handles() {
        let (dir, store) = store();
        let other = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        let meta = empty_meta(1);
        store.upsert_meta(meta.clone()).unwrap();
        other.set_pinned(1, true).unwrap();
        other.update_queue(1, vec!["queue".into()]).unwrap();
        store
            .commit_turn(&[ChatMessage::assistant("reply")], meta.clone())
            .unwrap();
        let current = &other.list()[0];
        assert!(current.pinned);
        assert_eq!(current.queued_prompts, ["queue"]);
        other.remove(1).unwrap();
        assert!(store.snapshot(1, &[]).is_err());
        assert!(store.update_settings(meta).is_err());
        assert!(store.write_state(1, "todo", b"late").is_err());
        assert!(store.list().is_empty());
    }

    #[test]
    fn same_and_different_sessions_support_concurrent_writes() {
        let (dir, store) = store();
        let path = dir.path().join("conversations");
        let root = dir.path().to_path_buf();
        let a = std::thread::spawn(move || {
            let store = SessionStore::open_at(&root, &path).unwrap();
            (0..40).map(|_| store.next_id()).collect::<Vec<_>>()
        });
        let ids: Vec<_> = (0..40).map(|_| store.next_id()).collect();
        let mut ids = ids;
        ids.extend(a.join().unwrap());
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 80);
        let db = store.db.clone();
        let ws = store.ws.clone();
        let other = SessionStore { db, ws };
        let a = std::thread::spawn(move || {
            for _ in 0..40 {
                other.update_queue(1, vec!["queue".into()]).unwrap();
            }
        });
        for _ in 0..40 {
            store.set_pinned(1, true).unwrap();
        }
        a.join().unwrap();
        assert_eq!(store.list()[0].queued_prompts, ["queue"]);
    }

    #[test]
    fn stale_window_cannot_overwrite_another_windows_history() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let first = vec![ChatMessage::user("first")];
        let revision = store.commit_turn_checked(&first, meta.clone(), 0).unwrap();
        let mut second = first.clone();
        second.push(ChatMessage::assistant("new answer"));
        store
            .commit_turn_checked(&second, meta.clone(), revision)
            .unwrap();
        assert!(store
            .commit_turn_checked(&[ChatMessage::user("stale window")], meta, revision)
            .is_err());
        assert_eq!(
            store
                .load_history(1)
                .unwrap()
                .last()
                .unwrap()
                .content
                .as_deref(),
            Some("new answer")
        );
    }
}
