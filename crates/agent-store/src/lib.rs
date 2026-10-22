//! File-based session repository: independent JSONL journals and atomic indexes.
//! Each window opens files on demand; only individual writes use short locks.
//! Old redb data is imported from a verified snapshot without locking the live
//! database or stopping another frontend. Original sources remain unchanged.

mod files;
mod migration;
pub use files::SessionDatabase;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use agent_llm_core::types::ChatMessage;
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const DB_FILE: &str = "conversations";

const METAS: TableDefinition<&str, &[u8]> = TableDefinition::new("metas");
const HISTORY: TableDefinition<&str, &[u8]> = TableDefinition::new("history");
const STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("state");
const PREFS: TableDefinition<&str, &[u8]> = TableDefinition::new("prefs");
const WORKSPACES: TableDefinition<&str, &[u8]> = TableDefinition::new("workspaces");
const GLOBALS: TableDefinition<&str, &[u8]> = TableDefinition::new("globals");

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// The journal changed under the caller: another window committed first.
/// Retrying the same write would overwrite that window, so callers must stop.
#[derive(Debug)]
struct StaleRevision;

/// Whether a row opens a user turn: a real prompt, not a hidden recall row
/// (`NoticeKind::Hidden`) and not a mid-turn steer. A steer rides in as a `user`
/// row so the model reads it as one, but it belongs to the turn already running
/// and never opens a new one, so it does not advance the turn count (rail
/// ordinal, `/raw` page numbers, the header card). The kernel decides what a
/// steer is ([`ChatMessage::is_steer`]); this is the one definition the store,
/// the index and the callers that count turns all share.
pub fn counts_as_turn(message: &ChatMessage) -> bool {
    message.role == agent_llm_core::Role::User
        && message.notice != Some(agent_llm_core::types::NoticeKind::Hidden)
        && !message.is_steer()
}

impl std::fmt::Display for StaleRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("会话已被另一个窗口更新，请重新打开此会话；当前结果未覆盖其他窗口的记录")
    }
}

impl std::error::Error for StaleRevision {}

/// True when `error` is the stale-revision conflict from `commit_turn_checked`.
pub fn is_stale_revision(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<StaleRevision>())
}

/// The paged history changed under the caller: another window committed or
/// checkpointed a different revision between this caller's read and its
/// `expected_revision` check. Refreshing the page is the only safe recovery.
#[derive(Debug)]
struct StaleHistoryPage;

impl std::fmt::Display for StaleHistoryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("会话历史已被其他窗口更新，请刷新后重试")
    }
}

impl std::error::Error for StaleHistoryPage {}

/// True when `error` is the stale-history-page conflict from
/// [`SessionStore::load_history_page_checked`].
pub fn is_stale_page(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<StaleHistoryPage>())
}

/// One session's sidebar metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    pub title: String,
    /// Preview of the last agent reply (or "running…" mid-turn).
    pub preview: String,
    /// Unix seconds of the last activity.
    pub updated_at: u64,
    /// Pinned within its workspace — floats to the top of `list()` (and so
    /// of the project's sidebar list) ahead of recency.
    #[serde(default)]
    pub pinned: bool,
    /// Pinned globally — leads the sidebar's cross-project 会话 list.
    /// Independent of `pinned`.
    #[serde(default)]
    pub pinned_global: bool,
    /// Archived — hidden from the regular lists (kept, restorable).
    #[serde(default)]
    pub archived: bool,
    /// `title` was set by the user; the per-turn title derived from the first
    /// prompt must not overwrite it.
    #[serde(default)]
    pub renamed: bool,
    /// Usage of the last completed turn — persisted so reopening a session
    /// seeds the header meter with real numbers before the next `Usage`
    /// event arrives (history alone can't reconstruct token counts).
    #[serde(default)]
    pub usage: Option<SessionUsage>,
    /// Model that produced `usage` — the settings 统计 pane groups spend by
    /// model, which the token counts alone cannot tell apart. Doubles as the
    /// session's persisted model selection: `spawn_actor` restores it so a
    /// reopened session keeps the model it was last run with.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Per-session composer settings — written by the actor the moment a
    /// `SetModel`/`SetPermissionMode`/`SetAgentMode`/`SetThinkingLevel`
    /// command lands, so a session's choices are its own (not a workspace-wide
    /// default that leaks across sessions). `None` = "fall back to the
    /// workspace default at spawn".
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub agent_mode: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    /// Parked follow-up prompts the composer queued mid-turn — session
    /// state like `history`, so a switched-away-and-back or reopened
    /// session still has them. Written on every queue mutation; restored
    /// by `spawn_actor`.
    #[serde(default)]
    pub queued_prompts: Vec<String>,
    /// Model rounds (assistant messages) in the current history. The header
    /// calls this "运行" — one user prompt can contain many of them, unlike
    /// the user-turn count the rail uses.
    /// `None` on snapshots written before counts existed — the UI then falls
    /// back to the loaded rows instead of showing a false 0.
    #[serde(default)]
    pub runs: Option<u32>,
    /// Tool results (task steps) in the current history.
    #[serde(default)]
    pub tasks: Option<u32>,
    /// Token spend of this session's post-turn distillation calls — a side
    /// channel, never part of `usage`. Store-owned: `add_distill_usage`
    /// accumulates it from the background task, and every other writer
    /// leaves it alone (see `Writer::meta`).
    #[serde(default)]
    pub distill: Option<DistillUsage>,
}

/// What the background distiller has spent on this session, across resumes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistillUsage {
    pub calls: u32,
    pub prompt: u32,
    pub completion: u32,
}

/// One checked page of a session's *effective* history — the recovery draft
/// when it is still valid for the canonical journal, otherwise the journal
/// itself. `revision` is the 16-char hex revision of the exact snapshot the
/// page was sliced from, so a caller can detect a same-length rewrite without
/// rereading (or trusting) the whole history.
#[derive(Debug, Clone, Default)]
pub struct HistoryPageSnapshot {
    /// The requested slice, oldest `before`-bounded page of the snapshot.
    pub history: Vec<ChatMessage>,
    /// Number of messages in the whole snapshot the page came from.
    pub total: usize,
    /// Number of non-hidden user turns in the whole snapshot.
    pub turns: usize,
    /// Ordinal of the first turn inside `history` (turns before it).
    pub turn_offset: usize,
    /// `format!("{:016x}", revision)` — hex, never a JS number.
    pub revision: String,
}

/// A session's last-known token usage, persisted inside `SessionMeta`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SessionUsage {
    pub prompt: u32,
    pub completion: u32,
    pub context_window: u32,
    /// Prompt tokens served from the provider cache (cache hits) —
    /// `prompt - cached - cache_write` is the fresh share. `default` keeps
    /// old meta files valid.
    #[serde(default)]
    pub cached: u32,
    /// Prompt tokens written to the provider's cache on this turn (Anthropic's
    /// `cache_creation_input_tokens`). Zero for wires that do not report it.
    #[serde(default)]
    pub cache_write: u32,
    /// Estimated prompt composition of the same request — the title bar's
    /// context card. `None` on rows written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<ContextBreakdown>,
}

/// A request's prompt split by source, in estimated tokens (character
/// heuristic — the UI scales the shares to the reported prompt total).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBreakdown {
    /// System prompt(s): instructions, workspace context, skills, memory.
    pub system: u32,
    /// Tool schemas sent alongside the messages.
    pub tools: u32,
    /// Visible user messages, including attached images.
    pub user: u32,
    /// Hidden recall rows the memory store injects before a user message.
    pub memory: u32,
    /// Assistant reply text.
    pub assistant: u32,
    /// Assistant reasoning replayed on the wire.
    pub reasoning: u32,
    /// Tool-call names and arguments the assistant emitted.
    pub tool_calls: u32,
    /// Tool results, including returned screenshots.
    pub tool_results: u32,
    /// Other hidden rows (steers, notices, compaction summaries).
    pub other: u32,
}

/// The legacy on-disk session index (`sessions/<ws>/index.json`) — kept as
/// the migration's deserialization shape only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    /// The session the user last had open — restored on next launch
    /// instead of defaulting to the most recently updated one.
    #[serde(default)]
    pub last_active: Option<i64>,
    pub sessions: Vec<SessionMeta>,
}

/// Per-workspace composer preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspacePrefs {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Per-workspace agent mode (`build` | `plan` | `goal`).
    #[serde(default)]
    pub agent_mode: Option<String>,
    /// Directories outside this workspace the user linked for good ("关联并记住").
    /// Every session of this workspace starts with them; another workspace
    /// never sees them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_roots: Vec<LinkedRootPref>,
}

/// One linked directory as stored. `access` is `"read"` or `"readwrite"`;
/// anything else reads back as read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedRootPref {
    pub path: std::path::PathBuf,
    #[serde(default = "default_link_access")]
    pub access: String,
}

fn default_link_access() -> String {
    "read".into()
}

/// Per-workspace row in the `workspaces` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkspaceRow {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    last_active: Option<i64>,
    /// Monotonic session-id counter — survives deletes. Without it
    /// `next_id` would reuse the id of a just-deleted session, and
    /// `root:id`-keyed frontend state could resurface as a "restored"
    /// session.
    #[serde(default)]
    next_id: i64,
}

/// Per-workspace session store — one row namespace (`<ws>/…`) inside the
/// shared `sessions.db`.
pub struct SessionStore {
    ws: String,
    db: Arc<SessionDatabase>,
}

/// Stable per-workspace key — xxh3 of the canonical root, unless the host
/// supplied a named namespace for this root.
pub fn workspace_key(root: &Path) -> String {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Some(key) = app_data_dir().and_then(|dir| {
        let config = DATA_DIR.read().ok()?;
        config.as_ref()?.namespaces.iter().find_map(|(path, key)| {
            let path = dir.join(path);
            let path = path.canonicalize().unwrap_or(path);
            (canon == path).then(|| key.clone())
        })
    }) {
        return key;
    }
    let h = xxhash_rust::xxh3::xxh3_64(canon.to_string_lossy().as_bytes());
    format!("{h:016x}")
}

struct DataDir {
    path: PathBuf,
    namespaces: Vec<(PathBuf, String)>,
}

/// The host's data directory.
///
/// The session database — and the global preferences that live beside it — are
/// process-wide singletons, so this cannot be a per-call argument: the host
/// sets it once at startup ([`init_data_dir`], called from
/// `SessionManager::spawn_at`). Directory resolution and any rename migration
/// belong to the product, which knows its own name.
static DATA_DIR: std::sync::RwLock<Option<DataDir>> = std::sync::RwLock::new(None);

/// Point the store at the host's data directory. Idempotent per directory; a
/// later call replaces the value so a manager booted at another root sees its
/// own. The database itself is opened lazily on first use.
pub fn init_data_dir(dir: PathBuf) {
    init_data_dir_with_namespaces(dir, &[]);
}

/// Set the host's data directory and stable keys for roots relative to it.
/// Named namespaces preserve product-specific storage layouts without the
/// repository knowing those products or interpreting their workspace modes.
pub fn init_data_dir_with_namespaces(dir: PathBuf, namespaces: &[(&str, &str)]) {
    if let Ok(mut w) = DATA_DIR.write() {
        *w = Some(DataDir {
            path: dir,
            namespaces: namespaces
                .iter()
                .map(|(path, key)| (PathBuf::from(*path), (*key).to_string()))
                .collect(),
        });
    }
}

/// The data directory the host supplied, if any.
///
/// A test binary gets one throwaway directory for the whole process instead:
/// the store is a process-wide singleton, so parallel tests must agree on the
/// directory or they would clobber each other's value.
pub fn app_data_dir() -> Option<PathBuf> {
    if is_test_binary() {
        return Some(std::env::temp_dir().join(format!("agent-test-{}-data", std::process::id())));
    }
    DATA_DIR
        .read()
        .ok()
        .and_then(|d| d.as_ref().map(|d| d.path.clone()))
}

/// Open (creating) the sessions database at `path`, running the legacy
/// file import once. `path.parent()` is the data dir — the legacy
/// `sessions/` tree and the global `*.json` files live next to the db.
fn open_db(path: &Path) -> std::io::Result<Arc<SessionDatabase>> {
    let (directory, source) = if path.is_file() {
        (
            path.with_extension("jsonl-sessions"),
            Some(path.to_path_buf()),
        )
    } else {
        let source = path
            .parent()
            .map(|p| p.join("sessions.db"))
            .filter(|p| p.is_file());
        (path.to_path_buf(), source)
    };
    let db = SessionDatabase::open(&directory)?;
    migration::import(
        &db,
        directory.parent().unwrap_or(Path::new(".")),
        source.as_deref(),
    )?;
    Ok(db)
}

/// `cargo test` binaries live under `target/*/deps/`; the app binary sits one
/// level up. `#[cfg(test)]` only reaches unit tests inside this lib — `tests/`
/// binaries link us as an external dep — so a runtime marker is the only way
/// BOTH kinds share the same rule: test code must never touch (or lock-fight
/// a running app over) the real `sessions.db`/`memory.db`.
pub fn is_test_binary() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| d.file_name()).map(|n| n == "deps"))
        .unwrap_or(false)
}

/// The db file the shared handle binds — the env override and test-binary
/// temp copy mirror `global_db`'s own rules so diagnostics name the right
/// file.
pub(crate) fn sessions_db_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AGENT_SESSION_DB") {
        Some(std::path::PathBuf::from(p))
    } else if is_test_binary() {
        Some(std::env::temp_dir().join(format!("agent-test-{}-jsonl-sessions", std::process::id())))
    } else {
        app_data_dir().map(|d| d.join(DB_FILE))
    }
}

#[derive(Default)]
struct SharedDb {
    db: Option<Arc<SessionDatabase>>,
    warning: Option<String>,
}

fn shared_db() -> &'static Mutex<SharedDb> {
    static DB: OnceLock<Mutex<SharedDb>> = OnceLock::new();
    DB.get_or_init(|| Mutex::new(SharedDb::default()))
}

fn open_persistent_db() -> std::io::Result<Arc<SessionDatabase>> {
    let path = sessions_db_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法确定会话数据库的数据目录")
    })?;
    open_db(&path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("打开 {} 失败: {error}", path.display()),
        )
    })
}

fn try_global_db() -> std::io::Result<Arc<SessionDatabase>> {
    let mut shared = shared_db().lock().map_err(io_err)?;
    if let Some(db) = &shared.db {
        return Ok(db.clone());
    }
    let db = open_persistent_db()?;
    // Cache successful opens only: a lock or path failure can be retried.
    shared.db = Some(db.clone());
    Ok(db)
}

fn global_db() -> Option<Arc<SessionDatabase>> {
    try_global_db().ok()
}

// ---------------------------------------------------------------------------
// Global rows — app-level singletons in the `globals` table.
//
// The preferences live in another module (and, one day, another crate), so
// they reach the shared database through this small public surface instead of
// the private helpers above.
// ---------------------------------------------------------------------------

/// Read a global row; `None` when there is no row, or no database yet.
pub fn global_get<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
    files::get(global_db()?.as_ref(), key)
}

/// Write a global row, replacing whatever was there.
pub fn global_put(key: &str, value: &impl serde::Serialize) -> std::io::Result<()> {
    files::put(try_global_db()?.as_ref(), key, value)
}

/// Read-modify-write a global row under its lock; a missing row starts from
/// `T::default()`.
pub fn global_update<T>(key: &str, f: impl FnOnce(&mut T)) -> std::io::Result<()>
where
    T: serde::de::DeserializeOwned + serde::Serialize + Default,
{
    let db = try_global_db()?;
    files::update_global(db.as_ref(), key, f)
}

/// Present on every workspace while the app uses a transient database.
/// Exposed through workspace_info so startup notices cannot be lost before
/// the frontend's event subscription is ready.
pub fn session_storage_warning() -> Option<String> {
    shared_db().lock().ok().and_then(|db| db.warning.clone())
}

/// Run the same validated one-time migration used by app startup. A CLI
/// entry point can surface errors without silently entering temporary mode.
pub fn migrate_session_database() -> std::io::Result<PathBuf> {
    Ok(try_global_db()?.root.clone())
}

// ---------------------------------------------------------------------------
// Legacy import — sessions/<ws>/{index.json,<id>.jsonl,prefs.json,workspace.json,
// <id>.*} plus the three app-level json files, all in ONE write tx so a crash
// leaves nothing half-migrated. Files are renamed aside only after the commit
// succeeds; a failure simply retries on the next open.
// ---------------------------------------------------------------------------

/// The newest complete line in an append-only `.jsonl` — the migration
/// equivalent of the old `load_history` tail scan. A torn trailing line is
/// skipped, so the last INTACT snapshot is what gets imported.
fn last_intact_jsonl_line(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    for line in bytes.split(|b| *b == b'\n').rev() {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if serde_json::from_slice::<Vec<ChatMessage>>(line).is_ok() {
            return Some(line.to_vec());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_llm_core::types::ChatMessage;

    /// A steer rides in as a `user` row but stays inside the running turn; the
    /// hidden recall row above a prompt is not a turn either. A steer written
    /// before the kernel tagged them carries only its prefix and still matches.
    #[test]
    fn only_real_prompts_open_a_turn() {
        let mut hidden = ChatMessage::user("recalled facts");
        hidden.notice = Some(agent_llm_core::types::NoticeKind::Hidden);
        let mut untagged_steer = ChatMessage::user("The user interrupted: 先别改 config");
        untagged_steer.notice = None;

        assert!(counts_as_turn(&ChatMessage::user("do the thing")));
        assert!(!counts_as_turn(&ChatMessage::user_steer("先别改 config")));
        assert!(!counts_as_turn(&untagged_steer));
        assert!(!counts_as_turn(&hidden));
        assert!(!counts_as_turn(&ChatMessage::assistant("ok")));
        assert!(!counts_as_turn(&ChatMessage::tool_result("c1", "ok")));
        assert!(!counts_as_turn(&ChatMessage::notice("已被用户中断")));
    }

    /// The 后台 record outlives the run it describes: the store keeps the list
    /// beside the journal, and a fresh handle reads it back.
    #[test]
    fn a_saved_job_list_outlives_the_store_handle() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = serde_json::json!([
            {"id": "bg-1", "command": "cargo test", "state": {"kind": "running"}},
        ]);
        {
            let store = test_store(&dir);
            assert_eq!(store.load_bg_jobs(0), None);
            store.save_bg_jobs(0, &jobs).unwrap();
        }
        let reopened = test_store(&dir);
        assert_eq!(reopened.load_bg_jobs(0), Some(jobs));
        // Another session's record is its own.
        assert_eq!(reopened.load_bg_jobs(1), None);
    }

    /// Deleting a session takes its job record with it.
    #[test]
    fn removing_a_session_drops_its_job_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);
        store.save_bg_jobs(3, &serde_json::json!([{"id": "bg-1"}])).unwrap();
        store.remove(3).unwrap();
        assert_eq!(store.load_bg_jobs(3), None);
    }

    /// One tool's results come out of the index with their call's arguments —
    /// across turns, through a re-rendered system prompt, without failed ones.
    #[test]
    fn tool_results_pick_one_tools_rows_out_of_the_journal() {
        use agent_llm_core::types::ToolCall;
        let call = |id: &str, name: &str, args: &str| ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.into(),
        };
        let mut a1 = ChatMessage::assistant("");
        a1.tool_calls = Some(vec![call("c1", "fuzzy_patch", "{\"path\":\"a\"}"), call("c2", "smart_read", "{}")]);
        let mut a2 = ChatMessage::assistant("");
        a2.tool_calls = Some(vec![call("c3", "apply_patch", "{}"), call("c4", "fuzzy_patch", "{\"path\":\"b\"}")]);
        let mut failed = ChatMessage::tool_result("c4", "refused");
        failed.is_error = Some(true);
        let history = vec![
            ChatMessage::system("v1"),
            ChatMessage::user("one"),
            a1,
            ChatMessage::tool_result("c1", "patched a"),
            ChatMessage::tool_result("c2", "body"),
            ChatMessage::user("two"),
            a2,
            ChatMessage::tool_result("c3", "updated b"),
            failed,
        ];

        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);
        store.commit_turn(&history[..5], meta(8, 1)).unwrap();
        // The next turn re-renders the system prompt — a `Replace` record.
        let mut next = history.clone();
        next[0] = ChatMessage::system("v2");
        store.commit_turn(&next, meta(8, 2)).unwrap();

        let rows = store.tool_results(8, &["fuzzy_patch", "apply_patch"]).unwrap();
        assert_eq!(
            rows,
            vec![
                ("fuzzy_patch".into(), "{\"path\":\"a\"}".into(), "patched a".into()),
                ("apply_patch".into(), "{}".into(), "updated b".into()),
            ]
        );
        // A fresh handle rebuilds the same links from the journal alone.
        let index = dir
            .path()
            .join("conversations")
            .join(workspace_key(dir.path()))
            .join("8.index.json");
        std::fs::remove_file(&index).unwrap();
        assert_eq!(test_store(&dir).tool_results(8, &["fuzzy_patch", "apply_patch"]).unwrap(), rows);
    }

    /// An isolated store: temp db file + the tempdir as the workspace root.
    fn test_store(dir: &tempfile::TempDir) -> SessionStore {
        let db = open_db(&dir.path().join("conversations")).unwrap();
        SessionStore::open_with(dir.path(), db).unwrap()
    }

    fn meta(id: i64, updated_at: u64) -> SessionMeta {
        SessionMeta {
            id,
            title: format!("s{id}"),
            preview: String::new(),
            updated_at,
            pinned: false,
            usage: None,
            model: None,
            provider: None,
            permission_mode: None,
            agent_mode: None,
            thinking_level: None,
            queued_prompts: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn snapshot_overwrites_so_only_the_latest_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.snapshot(7, &[ChatMessage::user("first")]).unwrap();
        store
            .snapshot(
                7,
                &[ChatMessage::user("first"), ChatMessage::assistant("one")],
            )
            .unwrap();
        let hist = store.load_history(7).expect("history");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[1].content.as_deref(), Some("one"));

        // Missing id → None.
        assert!(store.load_history(9).is_none());
    }

    #[test]
    fn metas_list_sorts_pinned_then_recency_and_next_id_grows() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.upsert_meta(meta(1, 100)).unwrap();
        store.upsert_meta(meta(2, 50)).unwrap();
        store.upsert_meta(meta(3, 10)).unwrap();
        assert_eq!(store.next_id(), 4);

        store.set_pinned(3, true).unwrap();
        let ids: Vec<i64> = store.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 1, 2]);

        // upsert replaces by id.
        store.upsert_meta(meta(2, 500)).unwrap();
        let ids: Vec<i64> = store.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    /// Pins, archive and a user title belong to the user: an actor's turn
    /// commit (which carries a derived title and default flags) keeps them.
    #[test]
    fn turn_commit_keeps_user_owned_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);
        store.upsert_meta(meta(1, 1)).unwrap();
        store.rename(1, "  我的标题 ").unwrap();
        store.set_pinned_global(1, true).unwrap();
        store.set_pinned(1, true).unwrap();
        store
            .commit_turn(&[ChatMessage::user("first prompt")], meta(1, 2))
            .unwrap();
        let m = store.list().into_iter().find(|m| m.id == 1).unwrap();
        assert_eq!(m.title, "我的标题");
        assert!(m.renamed && m.pinned && m.pinned_global);

        // A blank rename hands the title back to the derived one.
        store.rename(1, "  ").unwrap();
        store
            .commit_turn(&[ChatMessage::user("x")], meta(1, 3))
            .unwrap();
        let m = store.list().into_iter().find(|m| m.id == 1).unwrap();
        assert_eq!(m.title, "s1");
        assert!(!m.renamed);
    }

    /// Archiving hides nothing by itself but drops both pins; restoring
    /// does not bring them back.
    #[test]
    fn archive_drops_pins() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);
        store.upsert_meta(meta(1, 1)).unwrap();
        store.set_pinned(1, true).unwrap();
        store.set_pinned_global(1, true).unwrap();
        store.set_archived(1, true).unwrap();
        let m = store.list().into_iter().find(|m| m.id == 1).unwrap();
        assert!(m.archived && !m.pinned && !m.pinned_global);
        store.set_archived(1, false).unwrap();
        let m = store.list().into_iter().find(|m| m.id == 1).unwrap();
        assert!(!m.archived && !m.pinned);
        // Editing a session with no meta is a no-op, not an error.
        assert!(!store.set_archived(99, true).unwrap_or(false));
    }

    #[test]
    fn remove_drops_meta_history_state_and_last_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        store.upsert_meta(meta(7, 1)).unwrap();
        store.snapshot(7, &[ChatMessage::user("hi")]).unwrap();
        store.write_state(7, "todos.json", b"{}").unwrap();
        store.set_last_active(7).unwrap();

        store.remove(7).unwrap();
        assert!(store.list().is_empty());
        assert!(store.load_history(7).is_none());
        assert!(store.read_state(7, "todos.json").unwrap().is_none());
        assert_eq!(store.last_active(), None);
    }

    #[test]
    fn prefs_and_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        let prefs = WorkspacePrefs {
            provider: Some("devin".into()),
            model: Some("devin/swe-2".into()),
            thinking_level: Some("high".into()),
            permission_mode: Some("default".into()),
            agent_mode: Some("plan".into()),
            linked_roots: vec![LinkedRootPref {
                path: std::path::PathBuf::from("/tmp/elsewhere"),
                access: "readwrite".into(),
            }],
        };
        store.save_prefs(&prefs).unwrap();
        let loaded = store.load_prefs();
        assert_eq!(loaded.thinking_level.as_deref(), Some("high"));
        assert_eq!(loaded.model.as_deref(), Some("devin/swe-2"));
        assert_eq!(loaded.linked_roots, prefs.linked_roots);

        store
            .write_state(0, "todos.json", b"{\"next_id\":2}")
            .unwrap();
        assert_eq!(
            store.read_state(0, "todos.json").unwrap().as_deref(),
            Some(b"{\"next_id\":2}".as_slice())
        );
    }

    #[test]
    fn workspace_row_carries_root_and_last_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(&dir);

        assert_eq!(store.recorded_root(), Some(dir.path().to_path_buf()));
        // record_root is write-once.
        store.record_root(Path::new("/elsewhere"));
        assert_eq!(store.recorded_root(), Some(dir.path().to_path_buf()));

        store.set_last_active(4).unwrap();
        assert_eq!(store.last_active(), Some(4));
    }

    #[test]
    fn migrate_legacy_imports_the_json_tree() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let ws_dir = data.join("sessions").join("aa11bb22cc33dd44");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("workspace.json"), br#"{"root":"/proj/demo"}"#).unwrap();
        std::fs::write(
            ws_dir.join("index.json"),
            r#"{"last_active":7,"sessions":[{"id":7,"title":"t","preview":"","updated_at":9}]}"#,
        )
        .unwrap();
        std::fs::write(ws_dir.join("prefs.json"), br#"{"provider":"devin"}"#).unwrap();
        std::fs::write(
            ws_dir.join("7.jsonl"),
            concat!(
                r#"[{"role":"user","content":"old"}]"#,
                "\n",
                r#"[{"role":"user","content":"old"},{"role":"assistant","content":"new"}]"#,
                "\n",
                // torn trailing record — must be skipped
                r#"[{"role":"user","content":"brot"#
            ),
        )
        .unwrap();
        std::fs::write(ws_dir.join("7.todos.json"), b"{}").unwrap();
        std::fs::write(
            data.join("recent_workspaces.json"),
            r#"[{"path":"/proj/demo","name":"demo","last_opened":3}]"#,
        )
        .unwrap();
        std::fs::write(data.join("appearance.json"), br##"{"accent":"#000"}"##).unwrap();

        // `open_db` triggers the migration; the imported rows are keyed by
        // the legacy DIRECTORY name, so bind a store straight to that key.
        let store = SessionStore {
            ws: "aa11bb22cc33dd44".into(),
            db: open_db(&data.join(DB_FILE)).unwrap(),
        };
        let hist = store.load_history(7).expect("imported history");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[1].content.as_deref(), Some("new"));
        let m = &store.list()[0];
        assert_eq!(m.id, 7);
        assert_eq!(store.last_active(), Some(7));
        assert_eq!(store.load_prefs().provider.as_deref(), Some("devin"));
        assert_eq!(store.recorded_root(), Some(PathBuf::from("/proj/demo")));
        assert!(store.read_state(7, "todos.json").unwrap().is_some());

        // Migration is marked in the file store; original sources remain recoverable.
        assert!(data.join("sessions").exists());
        assert!(!data.join("sessions.legacy").exists());
        assert!(data.join("recent_workspaces.json").exists());
        assert!(!data.join("recent_workspaces.json.bak").exists());
        assert!(!data.join("appearance.json.bak").exists());
    }

    /// Snapshots written before run/task counts existed must read back as
    /// "unknown", not 0 — the UI falls back to loaded rows for them.
    #[test]
    fn legacy_meta_without_counts_reads_as_unknown() {
        let legacy: SessionMeta =
            serde_json::from_str(r#"{"id":1,"title":"t","preview":"","updated_at":0}"#).unwrap();
        assert_eq!((legacy.runs, legacy.tasks), (None, None));
        let mut current = meta(2, 0);
        current.runs = Some(0);
        current.tasks = Some(4);
        let back: SessionMeta =
            serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
        assert_eq!((back.runs, back.tasks), (Some(0), Some(4)));
    }
}
