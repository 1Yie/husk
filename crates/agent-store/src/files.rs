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

/// Atomic replace without `fsync`. Only the live-turn recovery snapshot uses
/// this: losing the newest draft on a power cut is acceptable, while a process
/// crash already leaves the written pages in the kernel cache. Canonical
/// journal/index writes still go through `atomic_json`.
fn atomic_json_checkpoint(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io_err("missing parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(tmp.as_file_mut(), value)?;
    tmp.as_file_mut().flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
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

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
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
    /// Monotonic count of committed *history* writes. Metadata-only commits do
    /// not bump it, so a live checkpoint survives a pin/queue edit. It is
    /// replayed from the journal's `Commit` records, so it survives losing the
    /// index cache, and it never decreases — which is what makes recovery
    /// snapshots ABA-safe: a snapshot can never become valid again after a
    /// later commit merely because canonical history reverted to an old value.
    #[serde(default)]
    history_generation: u64,
}
/// A running turn's newest history, stored outside the journal so frequent
/// checkpoints replace one small file instead of appending full drafts.
/// `base_generation` + `base_revision` identify the exact canonical journal
/// state it was checkpointed against; once a commit advances either, the
/// snapshot no longer matches and must be ignored.
#[derive(Clone, Serialize, Deserialize)]
struct RecoverySnapshot {
    base_revision: u64,
    #[serde(default)]
    base_generation: u64,
    history: Vec<ChatMessage>,
    /// Indices into `history` that exist only for crash recovery: the
    /// "interrupted" notice and the unknown-outcome placeholders for calls
    /// that never produced a result. `load_history` keeps them (the resumed
    /// engine needs the paired results), while `history_page` hides them so a
    /// live turn's trace does not claim it was interrupted. Sorted ascending.
    #[serde(default)]
    recovery_rows: Vec<usize>,
    meta: SessionMeta,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    History {
        from: usize,
    },
    Message {
        message: ChatMessage,
    },
    /// Replace one already-committed message in place. The system prompt at
    /// index 0 is re-rendered every turn (memory/skills/mode), so rewriting
    /// the whole journal just because it changed made the file grow
    /// O(turns × history). The superseded record stays in the file; the index
    /// points at this one.
    Replace {
        index: usize,
        message: ChatMessage,
    },
    Metadata {
        meta: SessionMeta,
    },
    Deleted,
    Commit {
        has_history: bool,
        #[serde(default)]
        history_generation: u64,
    },
}

fn is_turn(message: &ChatMessage) -> bool {
    message.role == agent_llm_core::Role::User
        && message.notice != Some(agent_llm_core::types::NoticeKind::Hidden)
}

/// Content hash for the journal index — the same construction as
/// `SessionStore::history_revision_of`, one message at a time.
fn message_hash(message: &ChatMessage) -> std::io::Result<u64> {
    Ok(xxhash_rust::xxh3::xxh3_64(&serde_json::to_vec(message)?))
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
                hash: message_hash(&message)?,
                turn: is_turn(&message),
            }),
            Record::Replace { index, message } => {
                let Some(location) = pending.messages.get_mut(index) else {
                    return Err(io_err("replace journal has an invalid index"));
                };
                location.offset = offset;
                location.hash = message_hash(&message)?;
                location.turn = is_turn(&message);
            }
            Record::Metadata { meta } => pending.meta = Some(meta),
            Record::Deleted => {
                pending.deleted = true;
                pending.messages.clear();
                pending.meta = None;
            }
            Record::Commit {
                has_history,
                history_generation,
            } => {
                pending.bytes = offset + len as u64;
                pending.has_history = has_history;
                pending.history_generation = history_generation;
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
    recovery_path: PathBuf,
    clear_recovery: bool,
}
impl JournalWriter {
    fn record(&mut self, record: Record) -> std::io::Result<()> {
        let offset = self.file.stream_position()?;
        if let Record::Message { message } = &record {
            self.index.messages.push(Location {
                offset,
                hash: message_hash(message)?,
                turn: is_turn(message),
            });
        }
        serde_json::to_writer(&mut self.file, &record)?;
        self.file.write_all(b"\n")?;
        Ok(())
    }
    /// Point an existing index entry at a new `Replace` record.
    fn replace(&mut self, index: usize, message: &ChatMessage) -> std::io::Result<()> {
        if index >= self.index.messages.len() {
            return Err(io_err("replace index out of range"));
        }
        let offset = self.file.stream_position()?;
        self.record(Record::Replace {
            index,
            message: message.clone(),
        })?;
        let location = &mut self.index.messages[index];
        location.offset = offset;
        location.hash = message_hash(message)?;
        location.turn = is_turn(message);
        Ok(())
    }
    fn history(&mut self, history: &[ChatMessage]) -> std::io::Result<()> {
        let mut first = 0;
        while first < history.len() && first < self.index.messages.len() {
            if message_hash(&history[first])? != self.index.messages[first].hash {
                break;
            }
            first += 1;
        }
        // The leading system prompt is rebuilt every turn (memory, skills,
        // mode). Replacing that one entry and appending the turn's new tail
        // keeps the journal linear in new turns instead of appending the whole
        // history each time.
        if first == 0 && !history.is_empty() && !self.index.messages.is_empty() {
            let old_len = self.index.messages.len();
            let common = history.len().min(old_len);
            let mut tail_matches = true;
            for index in 1..common {
                if message_hash(&history[index])? != self.index.messages[index].hash {
                    tail_matches = false;
                    break;
                }
            }
            if tail_matches && history.len() >= old_len {
                self.replace(0, &history[0])?;
                for message in &history[old_len..] {
                    self.record(Record::Message {
                        message: message.clone(),
                    })?;
                }
                self.index.has_history = true;
                // Every history write advances the epoch so an earlier
                // recovery snapshot can never match again. Metadata-only
                // writes don't reach here and keep the epoch.
                self.index.history_generation = self.index.history_generation.saturating_add(1);
                return Ok(());
            }
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
        // Any history write — even a no-op rewrite of identical content —
        // advances the epoch so an earlier recovery snapshot can never match
        // again. Metadata-only writes don't reach here and keep the epoch.
        self.index.history_generation = self.index.history_generation.saturating_add(1);
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
            history_generation: self.index.history_generation,
        })?;
        self.file.sync_all()?;
        self.index.bytes = self.file.stream_position()?;
        atomic_json(&self.index_path, &self.index)?;
        // Recovery lives outside the journal. Once an authoritative history
        // write lands it must not outlive it; the index is written first, so a
        // crash (or a failed unlink) before this removal leaves a snapshot
        // whose base generation no longer matches canonical — reads ignore it.
        // The caller holds the session lock, so this never races another
        // writer.
        if self.clear_recovery {
            // The authoritative commit already succeeded. Its new generation
            // invalidates any leftover snapshot, so cleanup is best effort;
            // reporting failure here would leave the caller's CAS revision
            // behind the history that was actually committed.
            let _ = remove_file_if_exists(&self.recovery_path);
        }
        Ok(())
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
    fn recovery_path(&self, id: i64) -> PathBuf {
        self.directory().join(format!("{id}.recovery.json"))
    }
    fn lock(&self, id: i64) -> PathBuf {
        self.directory().join(format!("{id}.lock"))
    }
    /// A corrupt or torn snapshot degrades to the journal: the snapshot is a
    /// cache, the journal is the source of truth.
    fn read_recovery(&self, id: i64) -> std::io::Result<Option<RecoverySnapshot>> {
        match File::open(self.recovery_path(id)) {
            Ok(file) => match serde_json::from_reader(file) {
                Ok(snapshot) => Ok(Some(snapshot)),
                Err(_) => Ok(None),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// Revision of a canonical journal from its cached per-message hashes —
    /// the same construction as `history_revision_of`, without loading any
    /// message bodies. This is what keeps paging O(page), not O(history).
    fn index_revision(index: &JournalIndex) -> u64 {
        if index.messages.is_empty() {
            return 0;
        }
        let hashes: Vec<u8> = index
            .messages
            .iter()
            .flat_map(|m| m.hash.to_le_bytes())
            .collect();
        xxhash_rust::xxh3::xxh3_64(&hashes)
    }
    /// The recovery snapshot is valid only when it still describes the exact
    /// canonical journal state it was checkpointed against. Both the monotonic
    /// epoch and the content revision must match: the revision rejects an
    /// unrelated commit, the epoch rejects an ABA revert to an old canonical
    /// history.
    fn recovery_matches(snapshot: &RecoverySnapshot, index: &JournalIndex) -> bool {
        snapshot.base_generation == index.history_generation
            && snapshot.base_revision == Self::index_revision(index)
    }
    /// The effective revision: a valid draft's history, else canonical.
    fn effective_revision(snapshot: Option<&RecoverySnapshot>, index: &JournalIndex) -> u64 {
        match snapshot {
            Some(snapshot) if Self::recovery_matches(snapshot, index) => {
                Self::history_revision_of(&snapshot.history)
            }
            _ => Self::index_revision(index),
        }
    }
    /// Recovery snapshot and journal index for a lock-free read. Order is
    /// deliberate: the snapshot is read *before* the index, so a commit that
    /// lands between the two reads produces a newer index generation and the
    /// older snapshot is rejected. Reading index-then-snapshot could instead
    /// cache a snapshot that a concurrent commit already superseded.
    fn recovery_and_index(
        &self,
        id: i64,
    ) -> std::io::Result<(Option<RecoverySnapshot>, JournalIndex)> {
        let snapshot = self.read_recovery(id)?;
        let index = self.read_index(id)?;
        let snapshot = snapshot.filter(|s| Self::recovery_matches(s, &index));
        Ok((snapshot, index))
    }
    fn clear_recovery(&self, id: i64) -> std::io::Result<()> {
        let path = self.recovery_path(id);
        locked(&self.lock(id), || remove_file_if_exists(&path))
    }
    fn read_index(&self, id: i64) -> std::io::Result<JournalIndex> {
        journal_index(&self.journal(id), &self.index(id))
    }
    /// Metadata-only transactions keep any live recovery draft; history
    /// transactions clear it after their commit (see `JournalWriter::commit`).
    fn write<T>(
        &self,
        id: i64,
        f: impl FnOnce(&mut JournalWriter) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        self.write_with(id, false, f)
    }
    fn write_with<T>(
        &self,
        id: i64,
        clear_recovery: bool,
        f: impl FnOnce(&mut JournalWriter) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        locked(&self.lock(id), || {
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
                recovery_path: self.recovery_path(id),
                clear_recovery,
            };
            let result = f(&mut writer)?;
            writer.commit()?;
            Ok(result)
        })
    }
    pub fn open(root: &Path) -> std::io::Result<Self> {
        Self::open_with(root, try_global_db()?)
    }
    pub fn open_or_transient(root: &Path) -> std::io::Result<Self> {
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
    /// Every id with persistent state on disk — journal or recovery snapshot.
    /// A turn that has only checkpointed has no `.jsonl`, so id discovery and
    /// allocation must consult both (and dedup an id present in each).
    fn session_ids(&self) -> Vec<i64> {
        std::fs::read_dir(self.directory())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                name.to_str()?
                    .strip_suffix(".jsonl")
                    .or_else(|| name.to_str()?.strip_suffix(".recovery.json"))?
                    .parse::<i64>()
                    .ok()
            })
            .collect()
    }
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut seen = std::collections::HashSet::new();
        let mut metas: Vec<_> = self
            .session_ids()
            .into_iter()
            .filter(|id| seen.insert(*id))
            .filter_map(|id| self.meta(id))
            .collect();
        metas.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        metas
    }
    /// One session's meta, resolved exactly like its `list()` row — without
    /// scanning every session in the workspace.
    pub fn meta(&self, id: i64) -> Option<SessionMeta> {
        let (snapshot, index) = self.recovery_and_index(id).ok()?;
        if index.deleted {
            return None;
        }
        match snapshot {
            Some(snapshot) => {
                let mut meta = snapshot.meta;
                if let Some(old) = &index.meta {
                    preserve_canonical(&mut meta, old);
                }
                Some(meta)
            }
            _ => index.meta,
        }
    }
    pub fn set_pinned(&self, id: i64, pinned: bool) -> std::io::Result<bool> {
        self.edit_meta(id, |m| m.pinned = pinned)
    }
    pub fn set_pinned_global(&self, id: i64, pinned: bool) -> std::io::Result<bool> {
        self.edit_meta(id, |m| m.pinned_global = pinned)
    }
    /// Archiving also drops both pins — a pin on a hidden row means nothing,
    /// and restoring shouldn't resurrect it at the top of a list.
    pub fn set_archived(&self, id: i64, archived: bool) -> std::io::Result<bool> {
        self.edit_meta(id, |m| {
            m.archived = archived;
            if archived {
                m.pinned = false;
                m.pinned_global = false;
            }
        })
    }
    /// User title. Blank clears the override: the next turn re-derives the
    /// title from the first prompt.
    pub fn rename(&self, id: i64, title: &str) -> std::io::Result<bool> {
        let title = title.trim().to_string();
        self.edit_meta(id, |m| {
            m.renamed = !title.is_empty();
            if !title.is_empty() {
                m.title = title;
            }
        })
    }
    /// Read-modify-write one session's meta; `false` when it has none yet.
    fn edit_meta(&self, id: i64, f: impl FnOnce(&mut SessionMeta)) -> std::io::Result<bool> {
        self.write(id, |w| {
            // A live turn that has only checkpointed (no journal metadata yet)
            // still needs user edits to land: seed from its recovery meta.
            let mut meta = match w.index.meta.clone() {
                Some(meta) => meta,
                None => match self.read_recovery(id)? {
                    Some(snapshot) => snapshot.meta,
                    None => return Ok(false),
                },
            };
            f(&mut meta);
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
        let (snapshot, index) = self.recovery_and_index(id).ok()?;
        if index.deleted {
            return None;
        }
        // A valid recovery draft is the live turn's newest history; the
        // journal is only authoritative once a commit clears the snapshot.
        if let Some(snapshot) = snapshot {
            return Some(snapshot.history);
        }
        if !index.has_history {
            return None;
        }
        self.messages(id, &index.messages).ok()
    }

    pub fn has_history(&self, id: i64) -> std::io::Result<bool> {
        let (snapshot, index) = self.recovery_and_index(id)?;
        if index.deleted {
            return Ok(false);
        }
        if snapshot.is_some() {
            return Ok(true);
        }
        Ok(index.has_history)
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
                Record::Message { message } | Record::Replace { message, .. } => {
                    messages.push(message)
                }
                _ => return Err(io_err("invalid message offset")),
            }
        }
        Ok(messages)
    }
    /// Slice the *effective* history (valid recovery draft, else the journal)
    /// and report the exact revision of the snapshot the slice came from.
    /// Canonical reads use the cached index offsets and hashes, so the full
    /// history is never loaded just to derive the revision.
    fn history_page(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
    ) -> std::io::Result<HistoryPageSnapshot> {
        let (snapshot, index) = self.recovery_and_index(id)?;
        if index.deleted {
            return Ok(HistoryPageSnapshot {
                revision: format!("{:016x}", 0u64),
                ..HistoryPageSnapshot::default()
            });
        }
        if let Some(snapshot) = snapshot {
            // The recovery draft carries synthetic rows (the interrupted
            // notice and unknown-outcome tool results) that only the resumed
            // engine should see. A live trace must not claim the running turn
            // was interrupted, so slice the *visible* history instead. The
            // revision still describes the full draft, matching the checkpoint
            // revision the writer validates against.
            let revision = Self::history_revision_of(&snapshot.history);
            let visible: Vec<&ChatMessage> = if snapshot.recovery_rows.is_empty() {
                snapshot.history.iter().collect()
            } else {
                snapshot
                    .history
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| snapshot.recovery_rows.binary_search(index).is_err())
                    .map(|(_, message)| message)
                    .collect()
            };
            let total = visible.len();
            let end = before.unwrap_or(total).min(total);
            let start = end.saturating_sub(count);
            let turns = visible.iter().filter(|m| is_turn(m)).count();
            let turn_offset = visible[..start].iter().filter(|m| is_turn(m)).count();
            return Ok(HistoryPageSnapshot {
                history: visible[start..end]
                    .iter()
                    .map(|message| (*message).clone())
                    .collect(),
                total,
                turns,
                turn_offset,
                revision: format!("{revision:016x}"),
            });
        }
        let revision = Self::index_revision(&index);
        let total = index.messages.len();
        let end = before.unwrap_or(total).min(total);
        let start = end.saturating_sub(count);
        let turns = index.messages.iter().filter(|m| m.turn).count();
        let turn_offset = index.messages[..start].iter().filter(|m| m.turn).count();
        let page = if start == end {
            vec![]
        } else {
            self.messages(id, &index.messages[start..end])?
        };
        Ok(HistoryPageSnapshot {
            history: page,
            total,
            turns,
            turn_offset,
            revision: format!("{revision:016x}"),
        })
    }
    pub fn load_history_page(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
    ) -> std::io::Result<(Vec<ChatMessage>, usize, usize, usize)> {
        let page = self.history_page(id, before, count)?;
        Ok((page.history, page.total, page.turns, page.turn_offset))
    }
    /// Checked paging: `expected_revision` must match the snapshot the page is
    /// sliced from, so a caller paging a stale view gets a refresh error
    /// instead of a silently inconsistent splice.
    pub fn load_history_page_checked(
        &self,
        id: i64,
        before: Option<usize>,
        count: usize,
        expected_revision: Option<&str>,
    ) -> std::io::Result<HistoryPageSnapshot> {
        let page = self.history_page(id, before, count)?;
        if let Some(expected) = expected_revision {
            if page.revision != expected {
                return Err(std::io::Error::other(StaleHistoryPage));
            }
        }
        Ok(page)
    }
    pub fn snapshot(&self, id: i64, history: &[ChatMessage]) -> std::io::Result<()> {
        self.write_with(id, true, |w| w.history(history))
    }
    pub fn upsert_meta(&self, meta: SessionMeta) -> std::io::Result<()> {
        self.write(meta.id, |w| w.meta(meta))
    }
    pub fn commit_turn(
        &self,
        history: &[ChatMessage],
        mut meta: SessionMeta,
    ) -> std::io::Result<()> {
        let id = meta.id;
        self.write_with(id, true, |w| {
            if let Some(old) = &w.index.meta {
                preserve_canonical(&mut meta, old);
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
        let id = meta.id;
        self.write_with(id, true, |w| {
            // Lock held: read the draft and validate it against the index we
            // are about to write from. A checkpoint may have advanced the
            // effective history past the last canonical commit; validate
            // against whichever is current.
            let snapshot = self.read_recovery(id)?;
            let effective = Self::effective_revision(snapshot.as_ref(), &w.index);
            if effective != expected {
                return Err(std::io::Error::other(StaleRevision));
            }
            if let Some(old) = &w.index.meta {
                preserve_canonical(&mut meta, old);
            }
            w.history(history)?;
            w.meta(meta)?;
            Ok(Self::history_revision_of(history))
        })
    }

    /// Persist a running turn's newest recovery point without touching the
    /// journal. The snapshot replaces the previous one atomically, so a turn
    /// that checkpoints once a second keeps the journal O(commits), not
    /// O(checkpoints). `expected` is validated against the *effective*
    /// revision — the live snapshot when it is still valid, the canonical
    /// journal otherwise — and the returned hash is the new effective one.
    pub fn checkpoint_turn_checked(
        &self,
        history: &[ChatMessage],
        meta: SessionMeta,
        expected: u64,
    ) -> std::io::Result<u64> {
        self.checkpoint_turn_checked_with_rows(history, meta, expected, &[])
    }

    /// `recovery_rows` are indices of synthetic rows that exist only for
    /// crash recovery (interrupted notice / unknown tool results). They stay
    /// in `load_history` but are hidden from `history_page`.
    pub fn checkpoint_turn_checked_with_rows(
        &self,
        history: &[ChatMessage],
        mut meta: SessionMeta,
        expected: u64,
        recovery_rows: &[usize],
    ) -> std::io::Result<u64> {
        let id = meta.id;
        let revision = Self::history_revision_of(history);
        locked(&self.lock(id), || {
            let index = self.read_index(id)?;
            if index.deleted {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "session was deleted",
                ));
            }
            let snapshot = self.read_recovery(id)?;
            let effective = Self::effective_revision(snapshot.as_ref(), &index);
            if effective != expected {
                return Err(std::io::Error::other(StaleRevision));
            }
            if let Some(old) = &index.meta {
                preserve_canonical(&mut meta, old);
            }
            // Draft writes intentionally skip fsync: a process crash keeps
            // the page cache, and a power loss only costs this recovery point
            // (the canonical journal is still synced on commit).
            atomic_json_checkpoint(
                &self.recovery_path(id),
                &RecoverySnapshot {
                    base_revision: Self::index_revision(&index),
                    base_generation: index.history_generation,
                    history: history.to_vec(),
                    recovery_rows: recovery_rows.to_vec(),
                    meta,
                },
            )?;
            Ok(revision)
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
            let mut meta = match w.index.meta.clone() {
                Some(meta) => meta,
                None => match self.read_recovery(id)? {
                    Some(snapshot) => snapshot.meta,
                    None => empty_meta(id),
                },
            };
            meta.queued_prompts = items;
            w.meta(meta)
        })
    }
    pub fn next_id(&self) -> i64 {
        self.workspace_update(|row| {
            let max = self.session_ids().into_iter().max().unwrap_or(0);
            row.next_id = row.next_id.max(max) + 1;
            row.next_id
        })
        .expect("allocate session id")
    }
    pub fn remove(&self, id: i64) -> std::io::Result<()> {
        if !self.is_deleted(id) {
            self.write_with(id, true, |w| {
                w.record(Record::Deleted)?;
                w.index.deleted = true;
                w.index.messages.clear();
                w.index.meta = None;
                w.index.has_history = false;
                Ok(())
            })?;
        } else {
            // An earlier delete may have crashed after the tombstone but
            // before dropping the draft; a tombstone always invalidates a
            // snapshot, so drop it under the same lock.
            self.clear_recovery(id)?;
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

/// Fields only the user sets (pins, archive, a custom title) — an actor's
/// turn-boundary write carries stale or default values for them, so the
/// stored ones win.
fn keep_user_fields(meta: &mut SessionMeta, old: &SessionMeta) {
    meta.pinned = old.pinned;
    meta.pinned_global = old.pinned_global;
    meta.archived = old.archived;
    if old.renamed {
        meta.renamed = true;
        meta.title = old.title.clone();
    }
}

/// Canonical-only facts an actor's checkpoint or turn write must not clobber:
/// user-owned flags/title, the parked queue, and the last recorded usage.
/// Applied both when a draft is written and when a valid draft is read back,
/// so edits that land mid-turn survive the next checkpoint.
fn preserve_canonical(meta: &mut SessionMeta, old: &SessionMeta) {
    keep_user_fields(meta, old);
    meta.queued_prompts = old.queued_prompts.clone();
    if meta.usage.is_none() {
        meta.usage = old.usage;
    }
}

fn empty_meta(id: i64) -> SessionMeta {
    SessionMeta {
        id,
        title: "新会话".into(),
        preview: String::new(),
        updated_at: 0,
        ..Default::default()
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
    fn meta_reads_one_session_like_its_list_row() {
        let (_dir, store) = store();
        let mut meta = empty_meta(1);
        meta.runs = Some(3);
        meta.tasks = Some(2);
        store.upsert_meta(meta).unwrap();
        store.upsert_meta(empty_meta(2)).unwrap();
        store.set_pinned(1, true).unwrap();
        let one = store.meta(1).unwrap();
        assert_eq!(one.id, 1);
        assert!(one.pinned);
        assert_eq!((one.runs, one.tasks), (Some(3), Some(2)));
        assert_eq!(store.meta(2).unwrap().runs, None);
        assert!(store.meta(9).is_none());
        store.remove(2).unwrap();
        assert!(store.meta(2).is_none());
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
        let error = store
            .commit_turn_checked(&[ChatMessage::user("stale window")], meta, revision)
            .unwrap_err();
        assert!(crate::is_stale_revision(&error));
        assert!(!crate::is_stale_revision(&std::io::Error::other(
            "disk full"
        )));
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

    #[test]
    fn recovery_cleanup_failure_does_not_turn_a_successful_commit_into_an_error() {
        let (_dir, store) = store();
        let initial = vec![ChatMessage::user("initial")];
        let revision = store
            .commit_turn_checked(&initial, empty_meta(1), 0)
            .unwrap();
        // A directory cannot be unlinked as a file. It models cleanup failure
        // after the authoritative journal and its index already landed.
        std::fs::create_dir(store.recovery_path(1)).unwrap();
        let final_history = vec![ChatMessage::user("completed")];
        let revision = store
            .commit_turn_checked(&final_history, empty_meta(1), revision)
            .unwrap();
        assert_eq!(store.load_history(1).unwrap(), final_history);
        let next = vec![ChatMessage::user("next turn")];
        store
            .commit_turn_checked(&next, empty_meta(1), revision)
            .unwrap();
        assert_eq!(store.load_history(1).unwrap(), next);
    }

    #[test]
    fn repeated_checkpoints_do_not_grow_the_journal_and_a_fresh_store_sees_the_latest() {
        let (dir, store) = store();
        let meta = empty_meta(1);
        let mut history = vec![ChatMessage::user("draft")];
        let mut revision = store
            .checkpoint_turn_checked(&history, meta.clone(), 0)
            .unwrap();
        assert!(!store.journal(1).exists());
        for i in 0..150 {
            history.push(ChatMessage::assistant(format!("draft {i}")));
            revision = store
                .checkpoint_turn_checked(&history, meta.clone(), revision)
                .unwrap();
        }
        // 150 checkpoints never touched the journal; only the snapshot grew.
        assert!(!store.journal(1).exists());
        assert!(store.recovery_path(1).exists());

        let other = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        let loaded = other.load_history(1).expect("snapshot history");
        assert_eq!(loaded.len(), history.len());
        assert_eq!(loaded.last().unwrap().content.as_deref(), Some("draft 149"));
        assert!(other.has_history(1).unwrap());
        assert!(other.list().iter().any(|m| m.id == 1));
        assert_eq!(other.next_id(), 2);
    }

    #[test]
    fn authoritative_commit_clears_recovery_and_the_interruption_draft() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let draft = vec![
            ChatMessage::user("q"),
            ChatMessage::notice("上次任务因会话意外中断而停止"),
        ];
        let draft_revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), 0)
            .unwrap();
        assert!(store.recovery_path(1).exists());

        let committed = vec![
            ChatMessage::user("q"),
            ChatMessage::assistant("final answer"),
        ];
        store
            .commit_turn_checked(&committed, meta, draft_revision)
            .unwrap();

        assert!(!store.recovery_path(1).exists());
        let loaded = store.load_history(1).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.last().unwrap().content.as_deref(),
            Some("final answer")
        );
        assert!(loaded.iter().all(|m| m.notice.is_none()));
    }

    #[test]
    fn a_changed_system_prompt_replaces_one_record_instead_of_rewriting_the_journal() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let mut history = vec![ChatMessage::system("system prompt v1")];
        history.extend((0..80).map(|i| ChatMessage::user(format!("message {i}"))));
        store.commit_turn(&history, meta.clone()).unwrap();
        let before = std::fs::metadata(store.journal(1)).unwrap().len();

        let mut changed = history.clone();
        changed[0] = ChatMessage::system("system prompt v2");
        // A real turn also appends its new messages.
        changed.push(ChatMessage::assistant("new answer"));
        store.commit_turn(&changed, meta).unwrap();
        let after = std::fs::metadata(store.journal(1)).unwrap().len();
        assert!(
            after - before < 1024,
            "a system-prompt update appended {} bytes",
            after - before
        );

        let loaded = store.load_history(1).unwrap();
        assert_eq!(loaded[0].content.as_deref(), Some("system prompt v2"));
        assert_eq!(loaded.len(), changed.len());

        // The index cache is an optimization: replaying the journal from
        // scratch must apply the replacement too.
        std::fs::remove_file(store.index(1)).unwrap();
        let replayed = store.load_history(1).unwrap();
        assert_eq!(replayed[0].content.as_deref(), Some("system prompt v2"));
        assert_eq!(replayed.len(), changed.len());
    }

    #[test]
    fn recovery_only_rows_are_hidden_from_history_pages_but_kept_for_resume() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let mut history = vec![ChatMessage::system("sys"), ChatMessage::user("q")];
        let hidden = history.len();
        history.push(ChatMessage::notice("interrupted"));
        let revision = store
            .checkpoint_turn_checked_with_rows(&history, meta.clone(), 0, &[hidden])
            .unwrap();

        // Recovery readers keep the synthetic row for the resumed engine.
        assert_eq!(store.load_history(1).unwrap().len(), 3);
        // Display readers hide it and page within the visible history.
        let (page, total, turns, _) = store.load_history_page(1, None, 10).unwrap();
        assert_eq!(total, 2);
        assert_eq!(turns, 1);
        assert!(page
            .iter()
            .all(|m| m.content.as_deref() != Some("interrupted")));
        let (older, _, _, _) = store.load_history_page(1, Some(1), 10).unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].content.as_deref(), Some("sys"));

        // Once the turn commits, the notice is real history and is shown.
        store.commit_turn_checked(&history, meta, revision).unwrap();
        let (page, total, _, _) = store.load_history_page(1, None, 10).unwrap();
        assert_eq!(total, 3);
        assert!(page
            .iter()
            .any(|m| m.content.as_deref() == Some("interrupted")));
    }

    #[test]
    fn competing_commit_invalidates_the_snapshot_and_rejects_a_stale_writer() {
        let (dir, store) = store();
        let other = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        let meta = empty_meta(1);
        let draft = vec![ChatMessage::user("q"), ChatMessage::assistant("draft")];
        let draft_revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), 0)
            .unwrap();

        let committed = vec![
            ChatMessage::user("q"),
            ChatMessage::assistant("other window"),
        ];
        other
            .commit_turn_checked(&committed, meta.clone(), draft_revision)
            .unwrap();
        assert!(!store.recovery_path(1).exists());

        let error = store
            .checkpoint_turn_checked(&draft, meta, draft_revision)
            .unwrap_err();
        assert!(crate::is_stale_revision(&error));
        assert_eq!(
            store
                .load_history(1)
                .unwrap()
                .last()
                .unwrap()
                .content
                .as_deref(),
            Some("other window")
        );
    }

    #[test]
    fn a_deleted_session_cannot_be_resurrected_by_a_recovery_snapshot() {
        let (dir, store) = store();
        let meta = empty_meta(1);
        let draft = vec![ChatMessage::user("q"), ChatMessage::assistant("draft")];
        let revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), 0)
            .unwrap();
        assert!(store.recovery_path(1).exists());

        store.remove(1).unwrap();
        assert!(!store.recovery_path(1).exists());
        assert!(store.load_history(1).is_none());
        assert!(!store.has_history(1).unwrap());
        assert!(store.list().is_empty());
        assert!(store
            .load_history_page_checked(1, None, 10, None)
            .unwrap()
            .history
            .is_empty());
        assert!(store
            .checkpoint_turn_checked(&draft, meta, revision)
            .is_err());

        let other = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        assert!(other.load_history(1).is_none());
        assert!(other.list().is_empty());
    }

    #[test]
    fn recovery_only_session_is_listed_and_allocates_the_next_id() {
        let (_dir, store) = store();
        store
            .checkpoint_turn_checked(&[ChatMessage::user("draft")], empty_meta(7), 0)
            .unwrap();
        assert!(!store.journal(7).exists());
        assert!(store.recovery_path(7).exists());
        let ids: Vec<i64> = store.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![7]);
        assert_eq!(store.list()[0].title, "新会话");
        assert_eq!(store.load_history(7).unwrap().len(), 1);
        assert_eq!(store.next_id(), 8);
    }

    #[test]
    fn checkpoint_preserves_canonical_user_fields_queue_and_usage() {
        let (_dir, store) = store();
        let mut meta = empty_meta(1);
        meta.updated_at = 10;
        store.upsert_meta(meta.clone()).unwrap();
        store.set_pinned(1, true).unwrap();
        store.set_pinned_global(1, true).unwrap();
        store.rename(1, "我的标题").unwrap();
        store.update_queue(1, vec!["queued".into()]).unwrap();
        let mut with_usage = store.list()[0].clone();
        with_usage.usage = Some(crate::SessionUsage {
            prompt: 5,
            completion: 6,
            context_window: 100,
            cached: 2,
            cache_write: 3,
            breakdown: Some(crate::ContextBreakdown {
                system: 7,
                tool_results: 9,
                ..Default::default()
            }),
        });
        store.upsert_meta(with_usage).unwrap();

        // The checkpoint carries a stale/default actor meta; canonical wins.
        let draft = vec![ChatMessage::user("q"), ChatMessage::assistant("draft")];
        let revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), 0)
            .unwrap();
        let current = &store.list()[0];
        assert!(current.pinned && current.pinned_global);
        assert_eq!(current.title, "我的标题");
        assert_eq!(current.queued_prompts, ["queued"]);
        assert_eq!(current.usage.map(|u| u.prompt), Some(5));

        // The authoritative commit keeps them too.
        let committed = vec![ChatMessage::user("q"), ChatMessage::assistant("final")];
        store
            .commit_turn_checked(&committed, meta, revision)
            .unwrap();
        let current = &store.list()[0];
        assert!(current.pinned && current.pinned_global);
        assert_eq!(current.title, "我的标题");
        assert_eq!(current.queued_prompts, ["queued"]);
        assert_eq!(current.usage.map(|u| u.prompt), Some(5));
        let breakdown = current.usage.and_then(|u| u.breakdown).unwrap();
        assert_eq!((breakdown.system, breakdown.tool_results), (7, 9));
    }

    #[test]
    fn usage_rows_without_a_breakdown_still_load() {
        let usage: crate::SessionUsage = serde_json::from_str(
            r#"{"prompt":1,"completion":2,"context_window":3,"cached":0,"cache_write":0}"#,
        )
        .unwrap();
        assert_eq!(usage.breakdown, None);
    }

    #[test]
    fn metadata_only_writes_keep_a_live_checkpoint_valid() {
        let (_dir, store) = store();
        let draft = vec![ChatMessage::user("q"), ChatMessage::assistant("draft")];
        let revision = store
            .checkpoint_turn_checked(&draft, empty_meta(1), 0)
            .unwrap();

        store.set_pinned(1, true).unwrap();
        store.update_queue(1, vec!["q2".into()]).unwrap();

        // The metadata epoch did not move, so the draft is still effective.
        assert_eq!(store.load_history(1).unwrap(), draft);
        let current = &store.list()[0];
        assert!(current.pinned);
        assert_eq!(current.queued_prompts, ["q2"]);
        store
            .checkpoint_turn_checked(&draft, empty_meta(1), revision)
            .unwrap();
    }

    #[test]
    fn checked_paging_detects_a_same_length_history_rewrite() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let first = vec![ChatMessage::user("q"), ChatMessage::assistant("one")];
        let first_revision = store.commit_turn_checked(&first, meta.clone(), 0).unwrap();
        let first_hex = format!("{first_revision:016x}");
        let page = store
            .load_history_page_checked(1, None, 10, Some(&first_hex))
            .unwrap();
        assert_eq!(page.revision, first_hex);
        assert_eq!(page.total, 2);
        assert_eq!(page.turns, 1);
        assert_eq!(page.history.len(), 2);

        // Same length, different content => different revision.
        let second = vec![ChatMessage::user("q"), ChatMessage::assistant("two")];
        let second_revision = store
            .commit_turn_checked(&second, meta, first_revision)
            .unwrap();
        let error = store
            .load_history_page_checked(1, None, 10, Some(&first_hex))
            .unwrap_err();
        assert!(crate::is_stale_page(&error));
        let second_hex = format!("{second_revision:016x}");
        let page = store
            .load_history_page_checked(1, None, 10, Some(&second_hex))
            .unwrap();
        assert_eq!(page.revision, second_hex);
        assert_eq!(page.history.last().unwrap().content.as_deref(), Some("two"));
    }

    #[test]
    fn identical_history_commit_clears_recovery_and_does_not_resurrect_metadata() {
        let (_dir, store) = store();
        let committed = vec![ChatMessage::user("q"), ChatMessage::assistant("a")];
        let revision = store
            .commit_turn_checked(&committed, empty_meta(1), 0)
            .unwrap();

        let mut stale = empty_meta(1);
        stale.title = "STALE DRAFT".into();
        let checkpoint_revision = store
            .checkpoint_turn_checked(&committed, stale, revision)
            .unwrap();
        assert_eq!(checkpoint_revision, revision);
        assert!(store.recovery_path(1).exists());
        assert_eq!(store.list()[0].title, "STALE DRAFT");

        // Identical history: the revision still matches, so only the history
        // epoch and the cleanup keep the stale draft from resurfacing.
        store
            .commit_turn_checked(&committed, empty_meta(1), revision)
            .unwrap();
        assert!(!store.recovery_path(1).exists());
        assert_eq!(store.list()[0].title, "新会话");
    }

    #[test]
    fn checkpoint_validates_against_the_effective_snapshot_revision_not_canonical() {
        let (_dir, store) = store();
        let meta = empty_meta(1);
        let base = vec![ChatMessage::user("q")];
        let base_revision = store.commit_turn_checked(&base, meta.clone(), 0).unwrap();

        let draft = vec![ChatMessage::user("q"), ChatMessage::assistant("draft")];
        let draft_revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), base_revision)
            .unwrap();
        assert_ne!(draft_revision, base_revision);
        // Reusing the canonical revision (the snapshot's base) is no longer
        // the effective revision, so the same-base reuse is rejected.
        let error = store
            .checkpoint_turn_checked(&draft, meta.clone(), base_revision)
            .unwrap_err();
        assert!(crate::is_stale_revision(&error));
        // The effective revision still works and replaces the snapshot.
        store
            .checkpoint_turn_checked(&draft, meta, draft_revision)
            .unwrap();
    }

    #[test]
    fn stale_snapshot_cannot_reappear_when_canonical_history_reverts() {
        let (dir, store) = store();
        let meta = empty_meta(1);
        let original = vec![ChatMessage::user("q"), ChatMessage::assistant("original")];
        let original_revision = store
            .commit_turn_checked(&original, meta.clone(), 0)
            .unwrap();

        let draft = vec![
            ChatMessage::user("q"),
            ChatMessage::assistant("recovered draft"),
        ];
        let draft_revision = store
            .checkpoint_turn_checked(&draft, meta.clone(), original_revision)
            .unwrap();
        let stale_snapshot = std::fs::read(store.recovery_path(1)).unwrap();

        // Another window commits a different history, then reverts canonical
        // to `original` — the very revision the snapshot was based on (ABA).
        let intermediate = vec![
            ChatMessage::user("q"),
            ChatMessage::assistant("intermediate"),
        ];
        let intermediate_revision = store
            .commit_turn_checked(&intermediate, meta.clone(), draft_revision)
            .unwrap();
        assert!(!store.recovery_path(1).exists());
        store
            .commit_turn_checked(&original, meta.clone(), intermediate_revision)
            .unwrap();
        assert!(!store.recovery_path(1).exists());

        // Simulate a crash where the unlink never happened: the old snapshot
        // is back and its base revision now equals canonical again.
        std::fs::write(store.recovery_path(1), &stale_snapshot).unwrap();

        let loaded = store.load_history(1).unwrap();
        assert_eq!(loaded, original);
        let page = store.load_history_page_checked(1, None, 10, None).unwrap();
        assert_eq!(page.history, original);
        assert_eq!(page.revision, format!("{original_revision:016x}"));

        // A cold handle rebuilds the epoch from the journal and agrees.
        let other = SessionStore::open_at(dir.path(), &dir.path().join("conversations")).unwrap();
        assert_eq!(other.load_history(1).unwrap(), original);
    }

    #[test]
    fn corrupt_recovery_snapshot_falls_back_to_the_journal() {
        let (_dir, store) = store();
        let history = vec![ChatMessage::user("q"), ChatMessage::assistant("a")];
        let revision = store
            .commit_turn_checked(&history, empty_meta(1), 0)
            .unwrap();
        store
            .checkpoint_turn_checked(&history, empty_meta(1), revision)
            .unwrap();
        std::fs::write(store.recovery_path(1), b"{not json").unwrap();
        assert_eq!(store.load_history(1).unwrap(), history);
        assert!(store.has_history(1).unwrap());
    }
}
