//! `memory/store.rs` — hierarchical memory persistence.
//!
//! Layers: working / session / episodic / semantic / persona, partitioned per
//! workspace by `hash(canonical_root)`. redb plus a deterministic hash-embedding stand
//! in for the spec's LibSQL + FastEmbed, keeping the single-binary constraint, behind an
//! API that mirrors those semantics (facts + episodes + persona + top-k recall).
//!
//! Write path: summarize → extract facts → dedupe (`cosine > 0.92` merges). Read path:
//! embed prompt → HNSW top-k facts → the per-message `<memory-context>` row; persona
//! heads the cached system prompt, and episodes are read on demand.
//!
//! Recall runs an in-memory HNSW index over fact embeddings — approximate
//! nearest-neighbour instead of the old O(N) redb scan. The index is a cache:
//! writes insert into it after commit, deletes trigger a rebuild, and a 30s
//! TTL rebuild covers facts written by ANOTHER process (app vs CLI share the
//! same memory.db handle inside a process, but not across processes).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// Approximate index over workspace fact embeddings — cosine ANN.
type FactsIndex = hnsw_rs::hnsw::Hnsw<'static, f32, hnsw_rs::anndists::dist::DistCosine>;

/// Fresh index — params sized for a personal-scale store (thousands of
/// facts, not millions): M=16 connections, ef_construction=200.
fn new_facts_index() -> FactsIndex {
    hnsw_rs::hnsw::Hnsw::new(16, 10_000, 16, 200, hnsw_rs::anndists::dist::DistCosine {})
}

/// How long a built index stays authoritative before the read path rebuilds
/// it — the drift bound for facts written by a different process.
const INDEX_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Minimum cosine similarity for a fact to enter the prompt block. Measured
/// on the hash embedding: related questions scored 0.30–0.45, unrelated ones
/// 0.00, and a single stray shared bigram reached ~0.15. 0.32 sits above the
/// noise and below the weakest real match — at 0.2 a Chinese prompt pulled
/// eight unrelated facts through on generic bigrams alone (系统, 逻辑, 显示).
const RELEVANCE_FLOOR: f32 = 0.32;

/// The same gate in the semantic space, which scores everything higher.
/// Measured on the multilingual model against real facts: related prompts
/// landed at 0.49–0.68, unrelated ones at 0.02–0.27. 0.30 sits in the gap —
/// one weak paraphrase (0.22) is lost, which the hash path missed too.
const SEMANTIC_FLOOR: f32 = 0.30;

/// A hit must also be this fraction of the best one. Clearing the absolute
/// floor is not enough: the weak tail is what made a correct top hit arrive
/// buried in eight irrelevant lines.
const RELATIVE_FLOOR: f32 = 0.55;

/// Facts injected per message. A few precise lines beat a list the model has
/// to sift — this block rides every request of the turn.
const FACTS_TOP_K: usize = 4;

/// How stale a fact's `last_used_at` must be before a recall rewrites it.
/// Without a floor, one turn that recalls the same fact on every message
/// rewrote the row every time.
const TOUCH_INTERVAL: u64 = 60;

/// Per-line cap for one recalled fact, in chars.
const FACT_LINE_CAP: usize = 160;

/// Char budget for the per-message facts block, heading included. Counted in
/// chars, not bytes: a CJK fact is three bytes per glyph, so a byte budget
/// spent its whole allowance on one line and a half and the other slots
/// ([`FACTS_TOP_K`]) never materialized. Chars track what the model pays for
/// far more closely than bytes do.
const FACTS_BLOCK_BUDGET: usize = 640;

/// Facts re-encoded per model call while catching up after the model loads.
/// One call per fact is how a first recall turned into seconds of inference;
/// batching is what makes the catch-up a single background pass.
const SEMANTIC_BACKFILL_BATCH: usize = 64;

/// Heading of the per-message facts section — counted against the budget, so
/// the cap below is on the whole block, heading included.
const FACTS_HEADER: &str = "── relevant facts ──\n";

/// One-line caps for the stable block. A preference is a line, and the
/// distiller has been known to hand over a paragraph.
const EPISODE_TASK_CAP: usize = 160;
const PERSONA_VALUE_CAP: usize = 240;
const PERSONA_KEY_CAP: usize = 40;

/// How close two persona values must be to count as the same preference in
/// different words. Measured on the bag-of-tokens hash embed: "用中文回答" vs
/// "中文回答" scores ~0.87, while "cargo test" vs "cargo test --workspace"
/// stays at 0.82 — this folds re-phrasings, not two commands that share words.
const PERSONA_MERGE_SIMILARITY: f32 = 0.85;

/// Two facts in one recall block this close say the same thing twice.
/// Measured on the hash embed with facts from real blocks: a same-language
/// re-phrasing ("state lives in src/stores/agent-store.ts" twice) scored 0.59,
/// two unrelated path facts sharing `__tests__`/`src` words up to 0.39. A
/// Chinese and an English wording of one fact share no tokens (0.08) — the
/// bag-of-hashes cannot see that twin at all.
const FACT_TWIN_SIMILARITY: f32 = 0.5;

/// Token similarity below which two facts share no meaningful word — the
/// floor for reading a pair as two languages rather than two statements.
const CROSS_LANGUAGE_GAP: f32 = 0.1;

/// Semantic similarity that makes a token-disjoint pair one fact in two
/// languages. Measured 0.71–0.80 for real zh/en twins of the same fact, while
/// unrelated pairs stayed at or below 0.42.
const CROSS_LANGUAGE_TWIN: f32 = 0.65;

/// Two consecutive episodes with the same task and outcome inside this window
/// are one event. A prompt retried twice used to burn all three recall slots
/// with duplicates of itself.
const EPISODE_DEDUPE_WINDOW: u64 = 600;

/// Char budget for [`MemoryStore::stable_block`], same reasoning as
/// [`FACTS_BLOCK_BUDGET`]: the block heads the provider's prompt cache as part
/// of the system prompt, so it stays small and moves only when the store is
/// written.
const STABLE_BLOCK_BUDGET: usize = 2048;

/// facts table: `id -> Fact` (JSON).
const FACTS: TableDefinition<u64, &[u8]> = TableDefinition::new("facts");
/// episodes table: `id -> Episode` (JSON).
const EPISODES: TableDefinition<u64, &[u8]> = TableDefinition::new("episodes");
/// persona table: `key -> value` (JSON string).
const PERSONA: TableDefinition<&str, &[u8]> = TableDefinition::new("persona");
/// meta table: `key -> value` — `schema_version` and migration stamps.
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

/// META key holding the next id to hand out — the cross-instance authority for
/// fact/episode ids.
const NEXT_ID_KEY: &str = "next_id";

/// Process-wide `Database` cache — redb permits one handle per file, so
/// every `open` resolves through this map keyed by the canonicalized path.
fn db_cache() -> &'static Mutex<HashMap<PathBuf, Arc<Database>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Database>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Current schema version — bump when a table changes; the open path runs
/// additive-or-transform migrations (never destructive) and backs up the
/// file before a failed migrate rather than crashing.
pub const SCHEMA_VERSION: u32 = 2;

/// Semantic memory fact — distilled convention/env/architecture note.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Fact {
    pub id: u64,
    pub workspace_id: u64,
    pub text: String,
    /// `f32` embedding — a deterministic hash-embed. Every similarity that
    /// means "the same statement" (write dedupe, persona folding, the recall
    /// block's own dedupe) uses this one: a sentence encoder cannot tell a
    /// rule from its reversal.
    pub embedding: Vec<f32>,
    /// The same text in the semantic space, when the model was ready at write
    /// time. Recall scoring prefers it; a missing vector is filled in on the
    /// next recall (see `MemoryStore::semantic_scores`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic: Vec<f32>,
    /// Which model produced [`Fact::semantic`]. Vectors from two models are
    /// not comparable — and two different models can return the same
    /// dimensionality, so the length is no test. An empty value means the
    /// vector predates this stamp and is re-computed on the next recall.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub semantic_model: String,
    /// `0.0..=1.0` — corrections seed 0.6; repeated confirmation raises.
    pub confidence: f32,
    /// When it was written (unix secs).
    pub created_at: u64,
    /// When it was last fed to the model (0 = never). Both halves matter:
    /// the ranking prefers what keeps proving useful, and
    /// [`MemoryStore::evict_stale_facts`] is what retires the rest.
    #[serde(default)]
    pub last_used_at: u64,
    /// How it got here. Older builds recorded no provenance, so the default
    /// reads them as [`FactSource::Legacy`] — serde's default keeps those rows
    /// loadable without a schema migration.
    #[serde(default)]
    pub source: FactSource,
}

/// Where a fact came from — the settings pane tells them apart, and the purge
/// pass only ever retires rows it can account for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// Written before provenance was recorded.
    #[default]
    Legacy,
    /// The post-turn distiller's model pass.
    Distiller,
    /// Written by a person (the pane's add form) — never evicted for age.
    Manual,
}

/// Episodic memory — "what happened" on one turn.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Episode {
    pub id: u64,
    pub workspace_id: u64,
    /// One-line task summary.
    pub task: String,
    /// Outcome: `"success" | "failed" | "steered"`.
    pub outcome: String,
    /// Files the turn touched.
    pub files: Vec<String>,
    /// Any user correction text (highest-value distill input).
    pub correction: Option<String>,
    pub created_at: u64,
}

/// Where a persona entry applies. `Global` follows the user across every
/// workspace; `Workspace` is this store's partition — a convention that
/// belongs to one repo must not surface in another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaScope {
    /// Every workspace.
    #[default]
    Global,
    /// This store's `workspace_id` partition.
    Workspace,
}

/// Persona key-value (style prefs, approval patterns).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Persona {
    pub key: String,
    pub value: String,
    /// Which scope the row was read from — `/memory` shows it, `/forget`
    /// removes the key from both.
    pub scope: PersonaScope,
}

impl PersonaScope {
    /// The wire/`/memory` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            PersonaScope::Global => "global",
            PersonaScope::Workspace => "workspace",
        }
    }
}

impl Persona {
    /// Identity of the row across scopes. Two entries may carry the same
    /// label — one global, one for this workspace — and a caller that books
    /// what it has already shown the model must tell them apart, or the one
    /// whose value differs rides every single message.
    pub fn delta_key(&self) -> String {
        format!("{}:{}", self.scope.as_str(), self.key)
    }
}

/// What a [`MemoryStore::purge_polluted`] pass removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PurgeReport {
    /// Facts the old "every steer/denial is a fact" rule wrote.
    pub superseded_facts: u64,
    /// Episodes that repeat the one before them — same task and outcome inside
    /// [`EPISODE_DEDUPE_WINDOW`].
    pub duplicate_episodes: u64,
    /// Facts nothing has recalled in months (see
    /// [`MemoryStore::evict_stale_facts`]).
    pub stale_facts: u64,
    /// "X lives in src/foo" facts graded down to
    /// [`LAYOUT_FACT_CONFIDENCE`] — kept, left to the lifecycle.
    #[serde(default)]
    pub layout_facts: u64,
}

/// Where a layout-only fact is graded to: below the eviction line
/// (`evict_stale_facts(_, 0.65)`), so one that goes unrecalled for the
/// eviction window retires. Recall still reaches it — confidence only orders
/// the block — and each recall pushes its `last_used_at` past the window.
const LAYOUT_FACT_CONFIDENCE: f32 = 0.4;

/// What [`MemoryStore::purge_polluted`] knows to clean. 2 added grading down
/// layout-only facts.
const PURGE_VERSION: u32 = 2;

/// The store — a shared `redb::Database` per `memory.db`, writes serialized
/// through a `Mutex` (redb is single-writer by design).
pub struct MemoryStore {
    pub(crate) db: Arc<Database>,
    /// `hash(canonical_root)` — partitions facts/episodes per workspace.
    workspace_id: u64,
    /// The root those facts describe. A fact that names paths is checked
    /// against it at recall: a tree that moved makes the fact wrong, not old.
    workspace_root: PathBuf,
    /// Embedding dim for the hash-embedder.
    embed_dim: usize,
    /// In-memory ANN index over THIS workspace's fact embeddings + the
    /// build timestamp (TTL'd; see INDEX_TTL). Cache, not truth — redb is.
    index: RwLock<(FactsIndex, std::time::Instant)>,
}

impl MemoryStore {
    /// Open (or create) `memory.db` for `workspace_root`.
    pub fn open(path: &Path, workspace_root: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // redb permits exactly one `Database` per file per process — every
        // session actor, the settings IPC and tests all open the same
        // memory.db, so a second `create` would fail with "cannot acquire
        // lock". Dedupe through a path-keyed cache instead (the same seam
        // `SessionStore::open` uses via `global_db`).
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let db = {
            let mut cache = db_cache().lock().map_err(|e| e.to_string())?;
            match cache.get(&key) {
                Some(db) => db.clone(),
                None => {
                    let db = Arc::new(Database::create(path).map_err(|e| e.to_string())?);
                    cache.insert(key, db.clone());
                    db
                }
            }
        };
        Self::open_with(db, workspace_root)
    }

    /// Open on a shared database — `open` funnels through here so every
    /// caller gets a `MemoryStore` on the same `Database` handle.
    pub fn open_with(db: Arc<Database>, workspace_root: &Path) -> Result<Self, String> {
        // Create tables + stamp schema_version up-front so reads never race
        // creation and migrations have a version to compare against.
        {
            let w = db.begin_write().map_err(|e| e.to_string())?;
            w.open_table(FACTS).map_err(|e| e.to_string())?;
            w.open_table(EPISODES).map_err(|e| e.to_string())?;
            w.open_table(PERSONA).map_err(|e| e.to_string())?;
            {
                let mut meta = w.open_table(META).map_err(|e| e.to_string())?;
                let existing = meta
                    .get("schema_version")
                    .map_err(|e| e.to_string())?
                    .map(|v| String::from_utf8_lossy(v.value()).into_owned());
                if existing.is_none() {
                    meta.insert("schema_version", SCHEMA_VERSION.to_string().as_bytes())
                        .map_err(|e| e.to_string())?;
                }
                // Migration: a DB written before the counter row existed has
                // none — seed it above the max persisted id so the first
                // allocation can't collide with existing facts/episodes.
                let has_counter = meta.get(NEXT_ID_KEY).map_err(|e| e.to_string())?.is_some();
                if !has_counter {
                    let seed = max_persisted_id(&w)? + 1;
                    meta.insert(NEXT_ID_KEY, seed.to_le_bytes().as_slice())
                        .map_err(|e| e.to_string())?;
                }
            }
            w.commit().map_err(|e| e.to_string())?;
        }
        let store = Self {
            db,
            workspace_id: workspace_id(workspace_root),
            workspace_root: workspace_root.to_path_buf(),
            embed_dim: 256,
            index: RwLock::new((new_facts_index(), std::time::Instant::now())),
        };
        store.migrate_embeddings()?;
        store.purge_once();
        store.rebuild_index();
        Ok(store)
    }

    /// Run [`purge_polluted`](Self::purge_polluted) the first time this
    /// workspace's partition is opened by a build that has it. The polluted
    /// rows are only ever recalled, never rewritten, so leaving the clean-up
    /// to a command the user has to know about kept them in every prompt.
    /// Best effort: a failure leaves the marker unset and the next open
    /// retries.
    fn purge_once(&self) {
        // Versioned: a pass that learns a new kind of stale row bumps it, and
        // every partition runs once more.
        let marker = format!("purged:v{PURGE_VERSION}:{:016x}", self.workspace_id);
        let done = self
            .db
            .begin_read()
            .ok()
            .and_then(|r| r.open_table(META).ok())
            .and_then(|m| m.get(marker.as_str()).ok().flatten().map(|_| ()))
            .is_some();
        if done || self.purge_polluted().is_err() {
            return;
        }
        let Ok(w) = self.db.begin_write() else { return };
        if let Ok(mut meta) = w.open_table(META) {
            let _ = meta.insert(marker.as_str(), b"1".as_slice());
        }
        let _ = w.commit();
    }

    /// Schema 2 changed how text becomes a vector (CJK bigrams). Vectors stored
    /// under schema 1 live in the old space and would score against new queries
    /// as noise, so every fact is re-embedded from its own text. IDs, text and
    /// confidence stay put, so dedupe and references are unaffected. One write
    /// transaction: either every vector moves to the new space or none does.
    fn migrate_embeddings(&self) -> Result<(), String> {
        let stored: u32 = {
            let r = self.db.begin_read().map_err(|e| e.to_string())?;
            let m = r.open_table(META).map_err(|e| e.to_string())?;
            m.get("schema_version")
                .map_err(|e| e.to_string())?
                .and_then(|v| String::from_utf8_lossy(v.value()).parse().ok())
                .unwrap_or(1)
        };
        if stored >= SCHEMA_VERSION {
            return Ok(());
        }
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        {
            let mut facts = w.open_table(FACTS).map_err(|e| e.to_string())?;
            let ids: Vec<u64> = facts
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .map(|(k, _)| k.value())
                .collect();
            for id in ids {
                let Some(raw) = facts
                    .get(id)
                    .map_err(|e| e.to_string())?
                    .map(|v| v.value().to_vec())
                else {
                    continue;
                };
                let Ok(mut f) = serde_json::from_slice::<Fact>(&raw) else {
                    continue;
                };
                f.embedding = self.embed(&f.text);
                let bytes = serde_json::to_vec(&f).map_err(|e| e.to_string())?;
                facts
                    .insert(id, bytes.as_slice())
                    .map_err(|e| e.to_string())?;
            }
        }
        {
            let mut meta = w.open_table(META).map_err(|e| e.to_string())?;
            meta.insert("schema_version", SCHEMA_VERSION.to_string().as_bytes())
                .map_err(|e| e.to_string())?;
        }
        w.commit().map_err(|e| e.to_string())
    }

    /// `hash(canonical_root)` — stable per-repo partition key.
    pub fn workspace_id(&self) -> u64 {
        self.workspace_id
    }

    /// Re-scan the FACTS table into a fresh HNSW — cheap at this scale and
    /// the only correct move for deletes (HNSW has no remove).
    fn rebuild_index(&self) {
        let facts = match self.all_facts() {
            Ok(f) => f,
            Err(_) => return,
        };
        let idx = new_facts_index();
        for f in facts.iter().filter(|f| f.workspace_id == self.workspace_id) {
            idx.insert((f.embedding.as_slice(), f.id as usize));
        }
        *self.index.write().unwrap_or_else(|e| e.into_inner()) = (idx, std::time::Instant::now());
    }

    /// Rebuild on TTL expiry — facts written by another process land in the
    /// shared redb file, not in this index.
    fn index_fresh(&self) {
        let stale = self
            .index
            .read()
            .map(|g| g.1.elapsed() >= INDEX_TTL)
            .unwrap_or(true);
        if stale {
            self.rebuild_index();
        }
    }

    /// Post-commit insert — kept outside the redb txn (the index is a cache;
    /// a failed insert only means a short-lived recall miss).
    fn index_insert(&self, emb: &[f32], id: u64) {
        if let Ok(g) = self.index.write() {
            g.0.insert((emb, id as usize));
        }
    }

    /// Insert a fact — dedupes vs. existing by `cosine > 0.92` (merge =
    /// bump confidence, else insert). Returns the fact's id.
    ///
    /// Scan, decision and write share ONE write transaction: with the old
    /// split (read txn for the scan, then a separate write txn) two
    /// back-to-back upserts could both pass the 0.92 gate and land duplicate
    /// facts.
    pub fn upsert_fact(&self, text: impl Into<String>, confidence: f32) -> Result<u64, String> {
        let text = text.into();
        let emb = self.embed(&text);
        // Inference costs seconds of CPU and this transaction holds the
        // database's only writer lock: the encoder runs before the txn opens,
        // never inside it.
        let semantic = crate::memory::semantic::embed(&text).unwrap_or_default();
        let semantic_model = if semantic.is_empty() {
            String::new()
        } else {
            crate::memory::semantic::model_id()
        };
        let w = self.db.begin_write().map_err(|e| e.to_string())?;

        // Nearest fact above the merge threshold — scans the workspace's facts
        // (small N; a proper ANN lands with FastEmbed), inside this txn.
        let merged: Option<(u64, Fact)> = {
            let facts = w.open_table(FACTS).map_err(|e| e.to_string())?;
            let mut best: Option<(f32, u64, Fact)> = None;
            for item in facts.iter().map_err(|e| e.to_string())? {
                let (_, v) = item.map_err(|e| e.to_string())?;
                let Ok(f) = serde_json::from_slice::<Fact>(v.value()) else {
                    continue;
                };
                if f.workspace_id != self.workspace_id {
                    continue;
                }
                let sim = cosine(&f.embedding, &emb);
                if sim >= 0.92 && best.as_ref().map(|(s, _, _)| sim > *s).unwrap_or(true) {
                    best = Some((sim, f.id, f));
                }
            }
            best.map(|(_, id, mut f)| {
                f.confidence = (f.confidence + confidence).min(1.0);
                // A row written while the model was loading — or scored by a
                // model that has since changed — gets this vector here: the
                // encoder already ran, and a merge is the cheapest moment.
                if !semantic.is_empty()
                    && (f.semantic.is_empty() || f.semantic_model != semantic_model)
                {
                    f.semantic = semantic.clone();
                    f.semantic_model = semantic_model.clone();
                }
                (id, f)
            })
        };

        let is_new = merged.is_none();
        let id = match merged {
            Some((id, f)) => {
                let bytes = serde_json::to_vec(&f).map_err(|e| e.to_string())?;
                w.open_table(FACTS)
                    .map_err(|e| e.to_string())?
                    .insert(id, bytes.as_slice())
                    .map_err(|e| e.to_string())?;
                id
            }
            None => {
                let id = take_next_id(&w)?;
                let f = Fact {
                    id,
                    workspace_id: self.workspace_id,
                    text,
                    // `index_insert` still borrows `emb` after commit.
                    embedding: emb.clone(),
                    semantic,
                    semantic_model,
                    confidence,
                    created_at: now(),
                    last_used_at: 0,
                    source: FactSource::Distiller,
                };
                let bytes = serde_json::to_vec(&f).map_err(|e| e.to_string())?;
                w.open_table(FACTS)
                    .map_err(|e| e.to_string())?
                    .insert(id, bytes.as_slice())
                    .map_err(|e| e.to_string())?;
                id
            }
        };
        w.commit().map_err(|e| e.to_string())?;
        // The index is a post-commit cache insert — a merge only bumped
        // confidence, so only a brand-new fact id needs a node.
        if is_new {
            self.index_insert(&emb, id);
        }
        Ok(id)
    }

    /// Top-k recall: ANN search over this workspace's facts — the HNSW gives
    /// approximate neighbours by `DistCosine` (distance = 1 − cosine); ids
    /// re-resolve through redb so a stale index can never fabricate a fact,
    /// and a rebuild runs when the index is older than INDEX_TTL. An empty
    /// index falls back to the linear scan (first-run, post-clear).
    pub fn recall_facts(&self, query: &str, k: usize) -> Result<Vec<Fact>, String> {
        Ok(self
            .recall_scored(query, k)?
            .into_iter()
            .map(|(_, f)| f)
            .collect())
    }

    /// Top-`k` facts with their cosine similarity to `query`, best first.
    /// Callers that gate on relevance (the prompt block) need the score; the
    /// rest use [`Self::recall_facts`].
    ///
    /// Scored in the semantic space whenever the model is loaded — a linear
    /// scan over a few thousand 384-dim vectors beats keeping a second ANN
    /// index in sync. Without it, the hash index answers as before.
    pub fn recall_scored(&self, query: &str, k: usize) -> Result<Vec<(f32, Fact)>, String> {
        if let Some(mut scored) = self.semantic_scores(query)? {
            scored.truncate(k);
            return Ok(scored);
        }
        let qemb = self.embed(query);
        self.index_fresh();
        let neighbours: Vec<(u64, f32)> = self
            .index
            .read()
            .map(|g| {
                g.0.search(qemb.as_slice(), k + 8, 40)
                    .into_iter()
                    .map(|n| (n.d_id as u64, n.distance))
                    .collect()
            })
            .unwrap_or_default();
        if !neighbours.is_empty() {
            let r = self.db.begin_read().map_err(|e| e.to_string())?;
            let t = r.open_table(FACTS).map_err(|e| e.to_string())?;
            let mut scored: Vec<(f32, Fact)> = Vec::with_capacity(neighbours.len());
            for (id, dist) in neighbours {
                let Some(v) = t.get(id).map_err(|e| e.to_string())? else {
                    continue; // index saw a since-deleted fact — skipped
                };
                if let Ok(f) = serde_json::from_slice::<Fact>(v.value()) {
                    if f.workspace_id == self.workspace_id {
                        scored.push((1.0 - dist, f));
                    }
                }
            }
            // The index answers approximately, and one that is behind the store
            // (a write landed since the last rebuild) answers short. A fact
            // missing from recall is worse than a scan, so an answer that does
            // not fill `k` is completed exactly below.
            if scored.len() >= k {
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                return Ok(scored.into_iter().take(k).collect());
            }
        }
        // Fallback — empty or short index (fresh db, a stale one), or every
        // candidate invalidated.
        let mut scored: Vec<(f32, Fact)> = self
            .all_facts()?
            .into_iter()
            .filter(|f| f.workspace_id == self.workspace_id)
            .map(|f| (cosine(&f.embedding, &qemb), f))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(k).collect())
    }

    /// The workspace's facts scored in the semantic space, best first. `None`
    /// when the model is not loaded — the caller then keeps to the hash
    /// vector.
    ///
    /// A fact written before the model was ready carries no vector; it is
    /// embedded here and written back in one transaction, so the first recall
    /// after the model loads pays for the store once and later recalls do not.
    fn semantic_scores(&self, query: &str) -> Result<Option<Vec<(f32, Fact)>>, String> {
        let Some(qemb) = crate::memory::semantic::embed(query) else {
            return Ok(None);
        };
        let model = crate::memory::semantic::model_id();
        let mut facts: Vec<Fact> = self
            .all_facts()?
            .into_iter()
            .filter(|f| f.workspace_id == self.workspace_id)
            .collect();
        // Rows with no vector from THIS model need the encoder: a fresh fact
        // written while the model was loading, a row from before scoring
        // existed, or a vector left behind by a model that has since changed
        // (two models can share a width, so the length proves nothing).
        let stale: Vec<usize> = facts
            .iter()
            .enumerate()
            .filter(|(_, f)| f.semantic.is_empty() || f.semantic_model != model)
            .map(|(at, _)| at)
            .collect();
        if !stale.is_empty() {
            let mut written: Vec<(u64, Vec<f32>)> = Vec::with_capacity(stale.len());
            for chunk in stale.chunks(SEMANTIC_BACKFILL_BATCH) {
                let texts: Vec<String> = chunk.iter().map(|&at| facts[at].text.clone()).collect();
                let vectors = crate::memory::semantic::embed_batch(&texts);
                if vectors.iter().all(Option::is_none) {
                    // The model went away mid-backfill: the rest keep to no
                    // vector rather than a wrong one, and the next recall
                    // picks the work up again.
                    break;
                }
                for (&at, vector) in chunk.iter().zip(vectors) {
                    let Some(vector) = vector else { continue };
                    facts[at].semantic = vector.clone();
                    facts[at].semantic_model = model.clone();
                    written.push((facts[at].id, vector));
                }
            }
            if !written.is_empty() {
                self.store_semantic(&model, &written);
            }
        }
        let mut scored: Vec<(f32, Fact)> = facts
            .into_iter()
            .filter(|f| !f.semantic.is_empty())
            .map(|f| (cosine(&f.semantic, &qemb), f))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Some(scored))
    }

    /// Write back vectors computed during a recall. Best effort — a failed
    /// write costs one re-embedding next time, never the answer.
    fn store_semantic(&self, model: &str, rows: &[(u64, Vec<f32>)]) {
        let Ok(w) = self.db.begin_write() else { return };
        let mut updates: Vec<(u64, Vec<u8>)> = Vec::with_capacity(rows.len());
        {
            let Ok(t) = w.open_table(FACTS) else { return };
            for (id, vector) in rows {
                let parsed = match t.get(*id) {
                    Ok(Some(raw)) => serde_json::from_slice::<Fact>(raw.value()).ok(),
                    _ => None,
                };
                let Some(mut f) = parsed else { continue };
                f.semantic = vector.clone();
                f.semantic_model = model.to_string();
                if let Ok(bytes) = serde_json::to_vec(&f) {
                    updates.push((*id, bytes));
                }
            }
        }
        if !updates.is_empty() {
            if let Ok(mut t) = w.open_table(FACTS) {
                for (id, bytes) in updates {
                    let _ = t.insert(id, bytes.as_slice());
                }
            }
        }
        let _ = w.commit();
    }

    /// The relevance gate for whichever space scores a query right now.
    fn relevance_floor(&self) -> f32 {
        if crate::memory::semantic::is_ready() {
            SEMANTIC_FLOOR
        } else {
            RELEVANCE_FLOOR
        }
    }

    fn all_facts(&self) -> Result<Vec<Fact>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(FACTS).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for item in t.iter().map_err(|e| e.to_string())? {
            let (_, v) = item.map_err(|e| e.to_string())?;
            if let Ok(f) = serde_json::from_slice::<Fact>(v.value()) {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// A fact whose paths all vanished names a tree that no longer exists —
    /// the file moved, the crate was renamed, the module was deleted. It may
    /// still be true of something, but not of this workspace, so the recall
    /// path skips it instead of spending a slot on it.
    fn fact_is_current(&self, text: &str) -> bool {
        let paths = path_tokens(text);
        paths.is_empty() || paths.iter().any(|p| self.workspace_root.join(p).exists())
    }

    /// Record that these facts were just put in front of the model. One write
    /// transaction for the handful a message selects; a failure costs the
    /// ranking and the eviction heuristic, never the answer.
    fn touch_facts(&self, ids: &[u64]) {
        if ids.is_empty() {
            return;
        }
        let Ok(w) = self.db.begin_write() else { return };
        {
            let Ok(mut t) = w.open_table(FACTS) else {
                return;
            };
            let at = now();
            for id in ids {
                let raw = match t.get(*id) {
                    Ok(Some(v)) => v.value().to_vec(),
                    _ => continue,
                };
                let Ok(mut f) = serde_json::from_slice::<Fact>(&raw) else {
                    continue;
                };
                // A fact recalled on every message of a long turn would
                // otherwise be rewritten each time. The stamp only has to be
                // accurate to the day — it drives ranking and eviction, not
                // ordering — so a minute of resolution is plenty.
                if at.saturating_sub(f.last_used_at) < TOUCH_INTERVAL {
                    continue;
                }
                f.last_used_at = at;
                let Ok(bytes) = serde_json::to_vec(&f) else {
                    continue;
                };
                let _ = t.insert(*id, bytes.as_slice());
            }
        }
        let _ = w.commit();
    }

    /// Append an episode — unless it repeats the one just written. Same task
    /// and outcome inside [`EPISODE_DEDUPE_WINDOW`] is one event: a prompt
    /// retried three times reports the same failure three times, and the
    /// recall read path only ever needs one.
    pub fn add_episode(&self, e: Episode) -> Result<u64, String> {
        let mut e = e;
        e.workspace_id = self.workspace_id;
        let last = self.recent_episodes(1)?.pop();
        if let Some(last) = &last {
            let repeat = last.task == e.task && last.outcome == e.outcome;
            let close = e.created_at.saturating_sub(last.created_at) <= EPISODE_DEDUPE_WINDOW;
            if repeat && close {
                return Ok(last.id);
            }
        }
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let id = take_next_id(&w)?;
        e.id = id;
        let bytes = serde_json::to_vec(&e).map_err(|e| e.to_string())?;
        w.open_table(EPISODES)
            .map_err(|e| e.to_string())?
            .insert(id, bytes.as_slice())
            .map_err(|e| e.to_string())?;
        w.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Last-`n` episode summaries for this workspace (recall path).
    pub fn recent_episodes(&self, n: usize) -> Result<Vec<Episode>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(EPISODES).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for item in t.iter().map_err(|e| e.to_string())? {
            let (_, v) = item.map_err(|e| e.to_string())?;
            if let Ok(e) = serde_json::from_slice::<Episode>(v.value()) {
                if e.workspace_id == self.workspace_id && !e.task.trim().is_empty() {
                    out.push(e);
                }
            }
        }
        out.sort_by_key(|e| e.created_at);
        Ok(out.into_iter().rev().take(n).collect())
    }

    /// All facts for this workspace, newest first — the settings list view
    /// needs no query embedding, so `recall_facts` doesn't fit.
    pub fn list_facts(&self, limit: usize) -> Result<Vec<Fact>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(FACTS).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for item in t.iter().map_err(|e| e.to_string())? {
            let (k, v) = item.map_err(|e| e.to_string())?;
            if let Ok(mut f) = serde_json::from_slice::<Fact>(v.value()) {
                f.id = k.value();
                if f.workspace_id == self.workspace_id {
                    out.push(f);
                }
            }
        }
        out.sort_by_key(|f| f.created_at);
        Ok(out.into_iter().rev().take(limit).collect())
    }

    /// Delete one fact row — the settings pane's per-item remove.
    pub fn remove_fact(&self, id: u64) -> Result<bool, String> {
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let removed = w
            .open_table(FACTS)
            .map_err(|e| e.to_string())?
            .remove(id)
            .map_err(|e| e.to_string())?
            .is_some();
        w.commit().map_err(|e| e.to_string())?;
        if removed {
            self.rebuild_index();
        }
        Ok(removed)
    }

    /// Episode row count for this workspace — the pane's stats line.
    pub fn count_episodes(&self) -> Result<u64, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(EPISODES).map_err(|e| e.to_string())?;
        let mut n = 0u64;
        for item in t.iter().map_err(|e| e.to_string())? {
            let (_k, v) = item.map_err(|e| e.to_string())?;
            if let Ok(e) = serde_json::from_slice::<Episode>(v.value()) {
                if e.workspace_id == self.workspace_id {
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// Wipe one area of the store. Every area clears only what this workspace
    /// can see: its own facts and episodes, and persona rows in its own scope
    /// or the global one — another repo's conventions are not this pane's to
    /// drop. `all` = facts + episodes + persona. Returns rows removed.
    pub fn clear(&self, what: &str) -> Result<u64, String> {
        let ws = self.workspace_id;
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let mut removed = 0u64;
        if what == "facts" || what == "all" {
            let mut t = w.open_table(FACTS).map_err(|e| e.to_string())?;
            let ids: Vec<u64> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter(|(_, v)| {
                    serde_json::from_slice::<Fact>(v.value())
                        .map(|f| f.workspace_id == ws)
                        .unwrap_or(false)
                })
                .map(|(k, _)| k.value())
                .collect();
            for id in ids {
                if t.remove(id).map_err(|e| e.to_string())?.is_some() {
                    removed += 1;
                }
            }
        }
        if what == "episodes" || what == "all" {
            let mut t = w.open_table(EPISODES).map_err(|e| e.to_string())?;
            let ids: Vec<u64> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter(|(_, v)| {
                    serde_json::from_slice::<Episode>(v.value())
                        .map(|e| e.workspace_id == ws)
                        .unwrap_or(false)
                })
                .map(|(k, _)| k.value())
                .collect();
            for id in ids {
                if t.remove(id).map_err(|e| e.to_string())?.is_some() {
                    removed += 1;
                }
            }
        }
        if what == "persona" || what == "all" {
            let mut t = w.open_table(PERSONA).map_err(|e| e.to_string())?;
            let keys: Vec<String> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .map(|(k, _)| k.value().to_string())
                .filter(|row| match parse_persona_row(row) {
                    (Some(id), _) => id == ws,
                    (None, _) => true,
                })
                .collect();
            for k in keys {
                if t.remove(k.as_str()).map_err(|e| e.to_string())?.is_some() {
                    removed += 1;
                }
            }
        }
        w.commit().map_err(|e| e.to_string())?;
        if what == "facts" || what == "all" {
            self.rebuild_index();
        }
        Ok(removed)
    }

    /// Remove the rows earlier builds wrote and this one no longer can, in one
    /// pass: facts that only restate a steer or a denial (`user steered: …` /
    /// `user corrected: …`), and episodes that repeat the one before them.
    /// Purely a migration for databases written before those rules changed —
    /// nothing here can be produced by the current write path.
    pub fn purge_polluted(&self) -> Result<PurgeReport, String> {
        let ws = self.workspace_id;
        let mut report = PurgeReport::default();
        let w = self.db.begin_write().map_err(|e| e.to_string())?;

        {
            let mut t = w.open_table(FACTS).map_err(|e| e.to_string())?;
            let doomed: Vec<u64> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter_map(|(k, v)| {
                    let f = serde_json::from_slice::<Fact>(v.value()).ok()?;
                    let legacy = f.text.starts_with("user steered:")
                        || f.text.starts_with("user corrected:");
                    (f.workspace_id == ws && legacy).then_some(k.value())
                })
                .collect();
            for id in doomed {
                if t.remove(id).map_err(|e| e.to_string())?.is_some() {
                    report.superseded_facts += 1;
                }
            }
            // "X lives in src/foo" is what the tree already says, and such facts
            // were most of what an older build left behind. Graded down rather
            // than deleted: one that keeps getting recalled stays, one nobody
            // needs goes in the next eviction.
            let layout: Vec<(u64, Fact)> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter_map(|(k, v)| {
                    let f = serde_json::from_slice::<Fact>(v.value()).ok()?;
                    (f.workspace_id == ws
                        && f.source != FactSource::Manual
                        && f.confidence > LAYOUT_FACT_CONFIDENCE
                        && crate::memory::distill::is_layout_fact(&f.text))
                    .then(|| (k.value(), f))
                })
                .collect();
            for (id, mut f) in layout {
                f.confidence = LAYOUT_FACT_CONFIDENCE;
                let bytes = serde_json::to_vec(&f).map_err(|e| e.to_string())?;
                t.insert(id, bytes.as_slice()).map_err(|e| e.to_string())?;
                report.layout_facts += 1;
            }
        }

        {
            let mut t = w.open_table(EPISODES).map_err(|e| e.to_string())?;
            let mut rows: Vec<(u64, Episode)> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter_map(|(k, v)| {
                    let e = serde_json::from_slice::<Episode>(v.value()).ok()?;
                    (e.workspace_id == ws).then_some((k.value(), e))
                })
                .collect();
            rows.sort_by_key(|(_, e)| e.created_at);
            let mut doomed = Vec::new();
            let mut last: Option<(String, String, u64)> = None;
            for (id, e) in rows {
                let repeat = last.as_ref().is_some_and(|(task, outcome, at)| {
                    *task == e.task
                        && *outcome == e.outcome
                        && e.created_at.saturating_sub(*at) <= EPISODE_DEDUPE_WINDOW
                });
                if repeat {
                    doomed.push(id);
                }
                last = Some((e.task, e.outcome, e.created_at));
            }
            for id in doomed {
                if t.remove(id).map_err(|e| e.to_string())?.is_some() {
                    report.duplicate_episodes += 1;
                }
            }
        }

        w.commit().map_err(|e| e.to_string())?;
        if report.superseded_facts > 0 {
            self.rebuild_index();
        }
        // Facts nothing ever asked for are the next thing to go: the same
        // "clean this store up" gesture, one pass.
        report.stale_facts = self.evict_stale_facts(180, 0.65).unwrap_or(0);
        Ok(report)
    }

    /// Retire facts nothing has needed: never recalled in `max_age_days` as
    /// recorded by this build, and never reinforced above `min_confidence`. A
    /// person-written fact ([`FactSource::Manual`]) is never evicted for age —
    /// it was a decision, not an inference — and a row predating
    /// `last_used_at` is skipped until its first recall, because its usage
    /// history is unknown rather than empty.
    pub fn evict_stale_facts(&self, max_age_days: u64, min_confidence: f32) -> Result<u64, String> {
        let ws = self.workspace_id;
        let horizon = now().saturating_sub(max_age_days.saturating_mul(86_400));
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let mut removed = 0u64;
        {
            let mut t = w.open_table(FACTS).map_err(|e| e.to_string())?;
            let doomed: Vec<u64> = t
                .iter()
                .map_err(|e| e.to_string())?
                .filter_map(|kv| kv.ok())
                .filter_map(|(k, v)| {
                    let f = serde_json::from_slice::<Fact>(v.value()).ok()?;
                    // A row this build has never recalled carries
                    // `last_used_at == 0`, and that zero is not evidence of
                    // neglect: the column did not exist before, so the fact may
                    // have been recalled on every turn for months. Only rows
                    // whose history this build actually recorded are eligible;
                    // a legacy row becomes eligible the first time it is used.
                    let touched = match (f.last_used_at, f.source) {
                        (0, FactSource::Legacy) => return None,
                        (0, _) => f.created_at,
                        (at, _) => at,
                    };
                    (f.workspace_id == ws
                        && f.source != FactSource::Manual
                        && f.confidence <= min_confidence
                        && touched <= horizon)
                        .then_some(k.value())
                })
                .collect();
            for id in doomed {
                if t.remove(id).map_err(|e| e.to_string())?.is_some() {
                    removed += 1;
                }
            }
        }
        w.commit().map_err(|e| e.to_string())?;
        if removed > 0 {
            self.rebuild_index();
        }
        Ok(removed)
    }

    /// Write a **global** persona entry — the `/remember` path: a preference
    /// the user states once applies wherever they work.
    pub fn set_persona(&self, key: &str, value: &str) -> Result<(), String> {
        self.set_persona_scoped(PersonaScope::Global, key, value)
            .map(|_| ())
    }

    /// Write a persona entry in `scope`, returning the key it landed on: a
    /// re-phrasing of a value the store already keeps folds into that entry
    /// instead of adding a synonym.
    pub fn set_persona_scoped(
        &self,
        scope: PersonaScope,
        key: &str,
        value: &str,
    ) -> Result<String, String> {
        // The writers are a model (`remember`, the distiller) and a human
        // (`/remember`): normalize here so no call site can inject a
        // multi-line value into a block the reader renders one preference per
        // line, and so a label can never look like a workspace row key.
        let key = persona_key(key);
        let value = one_line(value, PERSONA_VALUE_CAP);
        if key.is_empty() || value.is_empty() {
            return Err("persona needs a non-empty key and value".to_string());
        }
        // An existing label in THIS scope is an explicit update. Looking across
        // scopes here would let a global note with the same label stand in for
        // the workspace one, and the write would land in the other partition —
        // two rows sharing a label, both rendered, the workspace one silently
        // unchanged. A brand-new label may still be the same preference in
        // different words: fold it into that entry rather than grow the list by
        // one synonym per session.
        let key = if self.persona_in(scope, &key)?.is_some() {
            key
        } else {
            self.persona_twin(scope, &value)?.unwrap_or(key)
        };
        let row = persona_row_key(scope, self.workspace_id, &key);
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        w.open_table(PERSONA)
            .map_err(|e| e.to_string())?
            .insert(row.as_str(), value.as_bytes())
            .map_err(|e| e.to_string())?;
        w.commit().map_err(|e| e.to_string())?;
        Ok(key)
    }

    /// A persona value — this workspace's row wins over the global one.
    pub fn persona(&self, key: &str) -> Result<Option<String>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(PERSONA).map_err(|e| e.to_string())?;
        let key = persona_key(key);
        for scope in [PersonaScope::Workspace, PersonaScope::Global] {
            let row = persona_row_key(scope, self.workspace_id, &key);
            if let Some(v) = t.get(row.as_str()).map_err(|e| e.to_string())? {
                return Ok(Some(String::from_utf8_lossy(v.value()).into_owned()));
            }
        }
        Ok(None)
    }

    /// A persona value in `scope` only — the write path's "this label already
    /// exists" question. [`persona`](Self::persona) looks across scopes, which
    /// is what a reader wants and the wrong answer here: a global row with the
    /// same label would stand in for the workspace one and push the write into
    /// the other partition.
    pub fn persona_in(&self, scope: PersonaScope, key: &str) -> Result<Option<String>, String> {
        let row = persona_row_key(scope, self.workspace_id, &persona_key(key));
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(PERSONA).map_err(|e| e.to_string())?;
        match t.get(row.as_str()).map_err(|e| e.to_string())? {
            Some(v) => Ok(Some(String::from_utf8_lossy(v.value()).into_owned())),
            None => Ok(None),
        }
    }

    /// Remove a persona key from **both** scopes — the `/forget` path. A user
    /// asking to forget a preference means it, whichever scope it landed in.
    /// Returns whether any row existed.
    pub fn remove_persona(&self, key: &str) -> Result<bool, String> {
        let key = persona_key(key);
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let mut removed = false;
        {
            let mut t = w.open_table(PERSONA).map_err(|e| e.to_string())?;
            for scope in [PersonaScope::Workspace, PersonaScope::Global] {
                let row = persona_row_key(scope, self.workspace_id, &key);
                if t.remove(row.as_str()).map_err(|e| e.to_string())?.is_some() {
                    removed = true;
                }
            }
        }
        w.commit().map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// Remove a persona key from `scope` only — the settings pane's per-row
    /// delete, which names the entry the user clicked. The same label may be a
    /// separate entry in the other scope and must survive. Returns whether the
    /// row existed.
    pub fn remove_persona_scoped(&self, scope: PersonaScope, key: &str) -> Result<bool, String> {
        let row = persona_row_key(scope, self.workspace_id, &persona_key(key));
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let removed = w
            .open_table(PERSONA)
            .map_err(|e| e.to_string())?
            .remove(row.as_str())
            .map_err(|e| e.to_string())?
            .is_some();
        w.commit().map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// Every persona entry this workspace sees: its own rows plus the global
    /// ones. Another repo's rows stay in their own partition.
    pub fn all_persona(&self) -> Result<Vec<Persona>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(PERSONA).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for item in t.iter().map_err(|e| e.to_string())? {
            let (k, v) = item.map_err(|e| e.to_string())?;
            let row = k.value().to_string();
            let (scope, label) = match parse_persona_row(&row) {
                (Some(id), label) if id == self.workspace_id => (PersonaScope::Workspace, label),
                // Another repo's convention is not this repo's business.
                (Some(_), _) => continue,
                (None, label) => (PersonaScope::Global, label),
            };
            if label.is_empty() {
                continue;
            }
            out.push(Persona {
                key: label.to_string(),
                value: String::from_utf8_lossy(v.value()).into_owned(),
                scope,
            });
        }
        // Deterministic order: this block heads the cached system prompt.
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    /// The key in `scope` whose value already says this, if any — the point
    /// where a persona list stops growing one synonym per session. Compared on
    /// the same hash embedding the fact recall uses; persona counts are small
    /// enough that a linear scan is the whole implementation.
    fn persona_twin(&self, scope: PersonaScope, value: &str) -> Result<Option<String>, String> {
        let emb = self.embed(value);
        let mut best: Option<(f32, String)> = None;
        for p in self.all_persona()? {
            if p.scope != scope {
                continue;
            }
            let sim = cosine(&self.embed(&p.value), &emb);
            if sim >= PERSONA_MERGE_SIMILARITY
                && best.as_ref().map(|(s, _)| sim > *s).unwrap_or(true)
            {
                best = Some((sim, p.key));
            }
        }
        Ok(best.map(|(_, key)| key))
    }

    /// Deterministic hash-embedding — a bag-of-hashed-tokens vector.
    /// Stands in for FastEmbed (ONNX pulls a C toolchain); the interface is
    /// identical so the swap is a backend change, not an API change.
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.embed_dim];
        for feat in features(text) {
            let h = xxhash_rust::xxh3::xxh3_64(feat.as_bytes());
            let idx = (h as usize) % self.embed_dim;
            v[idx] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
        v
    }

    /// The `<memory>` prompt block: facts for `prompt` + the stable half,
    /// capped 2 KB (capability-roadmap.md §Read path).
    pub fn memory_block(&self, prompt: &str) -> Result<String, String> {
        let mut out = self.facts_block(prompt)?;
        out.push_str(&self.stable_block()?);
        Ok(cap_block(out))
    }

    /// The prompt-dependent half of [`memory_block`](Self::memory_block):
    /// stored facts that relate to `prompt`, empty when none clears the
    /// relevance gates. It changes with every question, so callers keep it
    /// out of the cached system prompt and send it beside the user turn.
    pub fn facts_block(&self, prompt: &str) -> Result<String, String> {
        Ok(self.facts_block_excluding(prompt, &HashSet::new())?.0)
    }

    /// [`facts_block`](Self::facts_block) minus the facts `seen` already put
    /// in front of the model. Returns the block **and** the ids it used, so
    /// the caller can remember them: a fact recalled for an earlier message is
    /// still in the history verbatim, and re-sending it is pure token spend.
    pub fn facts_block_excluding(
        &self,
        prompt: &str,
        seen: &HashSet<u64>,
    ) -> Result<(String, Vec<u64>), String> {
        // Over-fetch a little: the session has likely consumed the nearest
        // neighbours already, and the next-best ones are still relevant.
        let floor = self.relevance_floor();
        let ranked: Vec<(f32, Fact)> = self
            .recall_scored(prompt, FACTS_TOP_K + seen.len().min(8))?
            .into_iter()
            .filter(|(score, f)| {
                *score >= floor && !seen.contains(&f.id) && self.fact_is_current(&f.text)
            })
            .collect();
        let best = ranked.first().map(|(score, _)| *score).unwrap_or(0.0);
        // The weak tail is what made a correct top hit arrive buried in eight
        // irrelevant lines: keep only what is close to the best match.
        let mut ranked: Vec<(f32, Fact)> = ranked
            .into_iter()
            .filter(|(score, _)| *score >= best * RELATIVE_FLOOR)
            .collect();

        // Two facts that match equally well are not equally worth a slot: the
        // one the user keeps confirming, and the one that keeps proving useful,
        // go first. The gates above already decided what may enter; this only
        // orders it.
        let at = now();
        ranked.sort_by(|a, b| {
            rank_score(
                b.0,
                b.1.confidence,
                b.1.created_at.max(b.1.last_used_at),
                at,
            )
            .partial_cmp(&rank_score(
                a.0,
                a.1.confidence,
                a.1.created_at.max(a.1.last_used_at),
                at,
            ))
            .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut lines: Vec<String> = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        let mut picked: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
        let mut used = FACTS_HEADER.chars().count();
        for (_, f) in ranked {
            if lines.len() >= FACTS_TOP_K {
                break;
            }
            // A re-phrasing of a fact already in this block costs a slot and
            // says nothing new. It is booked as delivered all the same, so the
            // next message does not send the twin as if it were news.
            if picked
                .iter()
                .any(|(hash, semantic)| says_the_same_thing(hash, semantic, &f))
            {
                ids.push(f.id);
                continue;
            }
            // Whole lines only, as in the stable block: a fact cut in half is
            // worse than a fact left out.
            let line = format!("• {}\n", one_line(&f.text, FACT_LINE_CAP));
            let width = line.chars().count();
            if used + width > FACTS_BLOCK_BUDGET {
                break;
            }
            used += width;
            lines.push(line);
            ids.push(f.id);
            picked.push((f.embedding, f.semantic));
        }
        self.touch_facts(&ids);
        if lines.is_empty() {
            return Ok((String::new(), ids));
        }
        let mut out = String::from(FACTS_HEADER);
        for line in lines {
            out.push_str(&line);
        }
        Ok((out, ids))
    }

    /// The `memory_search` read path: facts that match `query` plus the most
    /// recent episodes. Explicit recall may return what the injected block
    /// withholds — the model asked, and there is no session dedupe here.
    pub fn search_report(&self, query: &str, limit: usize) -> Result<String, String> {
        let mut out = String::new();
        let floor = self.relevance_floor();
        let ranked: Vec<(f32, Fact)> = self
            .recall_scored(query, limit.clamp(1, 20))?
            .into_iter()
            .filter(|(score, f)| *score >= floor && self.fact_is_current(&f.text))
            .collect();
        let best = ranked.first().map(|(score, _)| *score).unwrap_or(0.0);
        let hits: Vec<&(f32, Fact)> = ranked
            .iter()
            .filter(|(score, _)| *score >= best * RELATIVE_FLOOR)
            .collect();
        if hits.is_empty() {
            out.push_str("no stored fact matches this query\n");
        } else {
            out.push_str(FACTS_HEADER);
            // An explicit search is usage too — it feeds the same ranking.
            let used: Vec<u64> = hits.iter().map(|hit| hit.1.id).collect();
            for hit in &hits {
                out.push_str(&format!("• {}\n", one_line(&hit.1.text, FACT_LINE_CAP)));
            }
            self.touch_facts(&used);
        }
        let episodes = self.episode_lines(limit.clamp(1, 5))?;
        if !episodes.is_empty() {
            out.push_str("── recent episodes ──\n");
            out.push_str(&episodes);
        }
        Ok(cap_block(out))
    }

    /// Last-`n` episodes as recall lines — the `memory_search` read path.
    /// Episodes left the system prompt: "some task [failed]", with no turn
    /// behind it, answered nothing and cost tokens on every request.
    pub fn episode_lines(&self, n: usize) -> Result<String, String> {
        let mut out = String::new();
        for e in self.recent_episodes(n)? {
            let line = format!("• {} [{}]", one_line(&e.task, EPISODE_TASK_CAP), e.outcome);
            if !push_line(&mut out, &line, FACTS_BLOCK_BUDGET) {
                break;
            }
        }
        Ok(out)
    }

    /// The prompt-independent half: persona, and nothing else. It only moves
    /// when a preference is written, so it can live in the system prompt
    /// without invalidating the provider's prompt cache every turn.
    pub fn stable_block(&self) -> Result<String, String> {
        Ok(Self::persona_block(
            &self.all_persona()?,
            STABLE_BLOCK_BUDGET,
        ))
    }

    /// [`stable_block`](Self::stable_block) plus the entries it actually
    /// carries. The caller must book only those as delivered: an entry the
    /// budget dropped never reached the model, and the recall row is the only
    /// way left to send it.
    pub fn stable_block_carried(&self) -> Result<(String, Vec<(String, String)>), String> {
        Ok(Self::persona_lines(
            &self.all_persona()?,
            STABLE_BLOCK_BUDGET,
        ))
    }

    /// Persona entries this session has not carried yet. The stable block is
    /// rendered once per spawn, so a `remember` in the middle of a session
    /// would otherwise stay invisible until a restart — the delta rides the
    /// next `<memory-context>` row instead of rewriting the cached prompt.
    pub fn persona_delta_block(
        &self,
        seen: &HashMap<String, String>,
    ) -> Result<(String, Vec<(String, String)>), String> {
        let fresh: Vec<Persona> = self
            .all_persona()?
            .into_iter()
            .filter(|p| seen.get(&p.delta_key()) != Some(&p.value))
            .collect();
        let rows = fresh
            .iter()
            .map(|p| (p.delta_key(), p.value.clone()))
            .collect();
        Ok((Self::persona_block(&fresh, FACTS_BLOCK_BUDGET), rows))
    }

    /// Render persona entries as the `── persona ──` section.
    pub fn persona_block(entries: &[Persona], budget: usize) -> String {
        Self::persona_lines(entries, budget).0
    }

    /// The section and the entries that fit in it. Assembled line by line
    /// against `budget` rather than cut by bytes: a truncation drops whatever
    /// the key order put last — the role's own language and commit rules,
    /// silently, in every session — and says how many went.
    fn persona_lines(entries: &[Persona], budget: usize) -> (String, Vec<(String, String)>) {
        // (identity, value, rendered line): the caller books the identity as
        // delivered, the model reads the line. They differ on purpose — two
        // scopes may share a label, and a booked entry that never appeared is
        // how a preference goes permanently missing.
        let rows: Vec<(String, String, String)> = entries
            .iter()
            .filter_map(|p| {
                let key = p.key.trim();
                let value = one_line(&p.value, PERSONA_VALUE_CAP);
                (!key.is_empty() && !value.is_empty())
                    .then(|| (p.delta_key(), value.clone(), format!("{key}: {value}")))
            })
            .collect();
        if rows.is_empty() {
            return (String::new(), Vec::new());
        }
        let mut out = String::from("── persona ──\n");
        let mut carried = Vec::new();
        for (delta, value, line) in &rows {
            if !push_line(&mut out, line, budget) {
                break;
            }
            carried.push((delta.clone(), value.clone()));
        }
        let omitted = rows.len() - carried.len();
        if omitted > 0 {
            // Keep the loss visible: a short list read as complete is how the
            // user gets the same preference contradicted every turn.
            out.push_str(&format!("… +{omitted} more\n"));
        }
        (out, carried)
    }
}

/// Append `line` and its newline, but only if the block still has room.
/// Whole lines only — an entry cut in half is worse than an entry missing —
/// and measured in chars, so a CJK entry is charged what the model pays.
fn push_line(out: &mut String, line: &str, budget: usize) -> bool {
    if out.chars().count() + line.chars().count() + 1 > budget {
        return false;
    }
    out.push_str(line);
    out.push('\n');
    true
}

/// Collapse `text` to one line of at most `cap` chars — whitespace runs become
/// a single space, and a token longer than the cap is cut on a char boundary
/// (a byte cut would panic on CJK).
fn one_line(text: &str, cap: usize) -> String {
    let mut out = String::with_capacity(text.len().min(cap * 4));
    let mut used = 0usize;
    for word in text.split_whitespace() {
        if !out.is_empty() {
            if used + 1 >= cap {
                break;
            }
            out.push(' ');
            used += 1;
        }
        let len = word.chars().count();
        let take = len.min(cap - used);
        out.extend(word.chars().take(take));
        used += take;
        if take < len {
            break;
        }
    }
    out
}

/// Hard cap on the whole block, in chars. The per-section budgets already add
/// up below it, so this only catches a block a caller assembled without them.
/// Counted in chars to match those budgets, and cut on a char boundary —
/// truncating inside a multi-byte char would panic.
fn cap_block(mut out: String) -> String {
    const CAP: usize = STABLE_BLOCK_BUDGET + FACTS_BLOCK_BUDGET;
    if out.chars().count() > CAP {
        let n = out
            .char_indices()
            .nth(CAP)
            .map(|(at, _)| at)
            .unwrap_or(out.len());
        out.truncate(n);
    }
    out
}

/// Hashing features for one text. ASCII runs split on whitespace, as before.
/// CJK has no spaces, so each run of CJK characters yields its character
/// bigrams (a lone character stands alone): "安装依赖" and "来安装依赖" then
/// share most features, and unrelated questions share almost none.
fn features(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let chars: Vec<char> = word.chars().collect();
        let mut ascii = String::new();
        let mut i = 0;
        while i < chars.len() {
            if is_cjk(chars[i]) {
                if !ascii.is_empty() {
                    out.push(std::mem::take(&mut ascii));
                }
                if i + 1 < chars.len() && is_cjk(chars[i + 1]) {
                    out.push(chars[i..i + 2].iter().collect());
                } else {
                    out.push(chars[i].to_string());
                }
            } else {
                ascii.push(chars[i]);
            }
            i += 1;
        }
        if !ascii.is_empty() {
            out.push(ascii);
        }
    }
    out
}

/// Path-like words in a fact — a `/` in the middle and a dot in the last
/// segment. Conservative on purpose: a false positive can hide a real fact, so
/// a URL, a glob, an absolute path and a bare directory (`src/`) all fail it.
fn path_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']')
    })
    .filter_map(|word| {
        // `src/lib.rs:120` names a line, not a different file.
        let word = match word.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => {
                head
            }
            _ => word,
        };
        let word = word.trim_end_matches(|c: char| c == '.' || c == ':');
        let looks_like_path = word.contains('/')
            && !word.starts_with('/')
            && !word.starts_with('~')
            && !word.starts_with("http")
            && !word.contains('*')
            && word
                .rsplit('/')
                .next()
                .is_some_and(|last| last.contains('.') && last.len() > 1);
        looks_like_path.then(|| word.to_string())
    })
    .collect()
}

/// CJK ideographs, kana and hangul — the scripts written without spaces.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'   // hiragana, katakana
        | '\u{3400}'..='\u{4DBF}' // CJK ext A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified
        | '\u{AC00}'..='\u{D7AF}' // hangul
        | '\u{F900}'..='\u{FAFF}' // CJK compatibility
    )
}

/// `hash(canonical_root)` — stable per-workspace partition.
fn workspace_id(root: &Path) -> u64 {
    xxhash_rust::xxh3::xxh3_64(root.to_string_lossy().as_bytes())
}

/// The label a caller's key becomes. Writers and readers both go through it:
/// a key the write path shortened or de-slashed has to be readable and
/// removable by the spelling the user was shown.
fn persona_key(raw: &str) -> String {
    one_line(raw, PERSONA_KEY_CAP).replace('/', "_")
}

/// Row key for a scoped persona entry. Global rows keep the bare label, so
/// everything written before scoping existed keeps working unchanged;
/// workspace rows carry their partition, which is what keeps one repo's
/// conventions out of another's prompt.
fn persona_row_key(scope: PersonaScope, ws: u64, key: &str) -> String {
    match scope {
        PersonaScope::Global => key.to_string(),
        PersonaScope::Workspace => format!("w{ws}/{key}"),
    }
}

/// Inverse of [`persona_row_key`]. Only a parseable `<id>/` prefix names a
/// partition — a label a `/` slipped into stays global.
fn parse_persona_row(row: &str) -> (Option<u64>, &str) {
    if let Some((ws, label)) = row.strip_prefix('w').and_then(|r| r.split_once('/')) {
        if let Ok(id) = ws.parse::<u64>() {
            return (Some(id), label);
        }
    }
    (None, row)
}

/// Highest id persisted in the id-keyed tables — the migration seed for
/// [`NEXT_ID_KEY`].
fn max_persisted_id(w: &redb::WriteTransaction) -> Result<u64, String> {
    let mut max_id = 0u64;
    {
        let t = w.open_table(FACTS).map_err(|e| e.to_string())?;
        for item in t.iter().map_err(|e| e.to_string())? {
            let (k, _) = item.map_err(|e| e.to_string())?;
            max_id = max_id.max(k.value());
        }
    }
    {
        let t = w.open_table(EPISODES).map_err(|e| e.to_string())?;
        for item in t.iter().map_err(|e| e.to_string())? {
            let (k, _) = item.map_err(|e| e.to_string())?;
            max_id = max_id.max(k.value());
        }
    }
    Ok(max_id)
}

/// Allocate the next id inside the caller's write transaction.
///
/// The counter lives in META instead of an in-process `Mutex`: two
/// `MemoryStore`s on one workspace each used to seed their own counter from
/// max+1, so concurrent inserts minted the same id and silently overwrote each
/// other. Reading + incrementing in the same txn as the insert makes the
/// database's writer lock the serialization point.
fn take_next_id(w: &redb::WriteTransaction) -> Result<u64, String> {
    let mut meta = w.open_table(META).map_err(|e| e.to_string())?;
    let seeded = match meta.get(NEXT_ID_KEY).map_err(|e| e.to_string())? {
        Some(v) if v.value().len() == 8 => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(v.value());
            u64::from_le_bytes(buf)
        }
        _ => 0,
    };
    // `open` seeds the row; zero means a DB it could not seed or one written
    // without the row — re-derive from the tables inside this same txn.
    let id = if seeded == 0 {
        max_persisted_id(w)? + 1
    } else {
        seeded
    };
    meta.insert(NEXT_ID_KEY, (id + 1).to_le_bytes().as_slice())
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Whether `candidate` says what an already-picked fact says — the block's
/// own dedupe.
///
/// Same-language twins are the token vector's job. The semantic vector only
/// adds the case the token space cannot see at all: a pair that shares no
/// words but reads the same, which is one fact written in two languages. It
/// deliberately does **not** widen the same-language gate — a sentence and its
/// reordering are 0.97 apart in that space, and dropping the newer one would
/// hide what the user just said.
fn says_the_same_thing(picked_hash: &[f32], picked_semantic: &[f32], candidate: &Fact) -> bool {
    let hash = cosine(picked_hash, &candidate.embedding);
    if hash >= FACT_TWIN_SIMILARITY {
        return true;
    }
    if picked_semantic.is_empty() || candidate.semantic.is_empty() || hash >= CROSS_LANGUAGE_GAP {
        return false;
    }
    cosine(picked_semantic, &candidate.semantic) >= CROSS_LANGUAGE_TWIN
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Ordering score for an already-eligible fact: relevance, weighted by how
/// often it was confirmed and how recently it proved useful. A fact the user
/// keeps repeating outranks a one-off note on the same topic, and a stale one
/// sinks — at most 25%, reached after a year, because a durable convention
/// decays slowly and a note about last month's bug does not.
///
/// `touched_at` is `max(created_at, last_used_at)`: a fact recalled yesterday
/// is fresh even if it was written a year ago.
fn rank_score(sim: f32, confidence: f32, touched_at: u64, now: u64) -> f32 {
    let age_days = now.saturating_sub(touched_at) as f32 / 86_400.0;
    let recency = 1.0 - 0.25 * (age_days / 365.0).clamp(0.0, 1.0);
    sim * (0.6 + 0.4 * confidence.clamp(0.0, 1.0)) * recency
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> MemoryStore {
        MemoryStore::open(&dir.join("m.db"), dir).unwrap()
    }

    /// Read the counter through the live store — a second `Database` on the
    /// same file is refused (`DatabaseAlreadyOpen`).
    fn read_counter(s: &MemoryStore) -> Option<u64> {
        let r = s.db.begin_read().unwrap();
        let t = r.open_table(META).unwrap();
        let v = t.get(NEXT_ID_KEY).unwrap().map(|v| {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(v.value());
            u64::from_le_bytes(buf)
        });
        v
    }

    /// The counter row is seeded at open and advanced inside the write txn —
    /// the authority that keeps a reopen from re-minting live ids.
    #[test]
    fn counter_row_is_seeded_and_advances() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let s = MemoryStore::open(&path, dir.path()).unwrap();
        assert_eq!(read_counter(&s), Some(1)); // fresh DB: next id is 1

        let f1 = s.upsert_fact("errors via AppError", 0.6).unwrap();
        let e1 = s
            .add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "a".into(),
                outcome: "success".into(),
                files: vec![],
                correction: None,
                created_at: 1,
            })
            .unwrap();
        assert_eq!((f1, e1), (1, 2));
        assert_eq!(read_counter(&s), Some(3));
    }

    /// Two stores on one workspace must never mint the same id. redb's default
    /// `ExclusiveWriter` mode forbids them living at once, so the overlapping
    /// case is a reopen: the counter row — not a max-scan — must carry over.
    #[test]
    fn reopened_store_continues_above_everything_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let a = MemoryStore::open(&path, dir.path()).unwrap();
        let f1 = a.upsert_fact("first", 0.5).unwrap();
        let e1 = a
            .add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "one".into(),
                outcome: "success".into(),
                files: vec![],
                correction: None,
                created_at: 1,
            })
            .unwrap();
        drop(a);

        let b = MemoryStore::open(&path, dir.path()).unwrap();
        let f2 = b
            .upsert_fact("second completely different text", 0.5)
            .unwrap();
        let e2 = b
            .add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "two".into(),
                outcome: "success".into(),
                files: vec![],
                correction: None,
                created_at: 2,
            })
            .unwrap();
        assert!(f2 > f1 && f2 > e1, "fact id reused: {f1}/{e1} → {f2}");
        assert!(e2 > e1 && e2 > f2, "episode id reused: {e1}/{f2} → {e2}");
    }

    /// Migration: a DB written before the counter existed has no row — open
    /// seeds it from the highest persisted id, so nothing collides.
    #[test]
    fn open_seeds_a_missing_counter_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let s = MemoryStore::open(&path, dir.path()).unwrap();
        s.upsert_fact("first", 0.5).unwrap();
        let e1 = s
            .add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "one".into(),
                outcome: "success".into(),
                files: vec![],
                correction: None,
                created_at: 1,
            })
            .unwrap();
        let db = s.db.clone();
        drop(s);

        // Simulate the pre-counter schema — the shared-handle cache keeps
        // one `Database` per path, so mutate it directly rather than
        // opening a second handle (which redb forbids).
        {
            let w = db.begin_write().unwrap();
            {
                let mut meta = w.open_table(META).unwrap();
                meta.remove(NEXT_ID_KEY).unwrap();
            }
            w.commit().unwrap();
        }

        let reopened = MemoryStore::open(&path, dir.path()).unwrap();
        let e2 = reopened
            .add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "two".into(),
                outcome: "success".into(),
                files: vec![],
                correction: None,
                created_at: 2,
            })
            .unwrap();
        assert!(e2 > e1, "seeded counter collided with persisted id {e1}");
    }

    #[test]
    fn facts_dedupe_by_cosine() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("errors via AppError, never unwrap", 0.6)
            .unwrap();
        // Near-identical text → merge, not duplicate.
        s.upsert_fact("errors via AppError, never unwrap", 0.8)
            .unwrap();
        let facts = s.recall_facts("error handling", 10).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].confidence > 0.9); // merged
    }

    /// ANN recall — the HNSW index must surface the same nearest facts the
    /// linear scan would (the index is a cache, redb stays the truth).
    #[test]
    fn recall_ranks_by_embedding_similarity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let s = MemoryStore::open(&path, dir.path()).unwrap();
        s.upsert_fact("rust cargo build workspace", 0.8).unwrap();
        s.upsert_fact("typescript npm install", 0.8).unwrap();
        s.upsert_fact("python pip venv setup", 0.8).unwrap();
        let hits = s.recall_facts("cargo build rust", 2).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].text, "rust cargo build workspace");
    }

    /// After `remove_fact` the index must not resurface the deleted row —
    /// the post-remove rebuild is the only correct move (HNSW has no delete).
    #[test]
    fn removed_fact_stays_removed_from_recall() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let s = MemoryStore::open(&path, dir.path()).unwrap();
        let id = s.upsert_fact("ephemeral build hint", 0.8).unwrap();
        assert!(!s
            .recall_facts("ephemeral build hint", 5)
            .unwrap()
            .is_empty());
        s.remove_fact(id).unwrap();
        assert!(s
            .recall_facts("ephemeral build hint", 5)
            .unwrap()
            .iter()
            .all(|f| f.id != id));
    }

    #[test]
    fn episodes_scoped_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.add_episode(Episode {
            id: 0,
            workspace_id: 0,
            task: "fix engine".into(),
            outcome: "success".into(),
            files: vec!["engine.rs".into()],
            correction: None,
            created_at: 1,
        })
        .unwrap();
        let eps = s.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "fix engine");
    }

    #[test]
    fn memory_block_respects_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for i in 0..20 {
            s.upsert_fact(format!("fact number {i} {}", "pad ".repeat(40)), 0.6)
                .unwrap();
        }
        let block = s.memory_block("query").unwrap();
        // The cap is in chars — a CJK block is three bytes per glyph.
        assert!(block.chars().count() <= STABLE_BLOCK_BUDGET + FACTS_BLOCK_BUDGET);
    }

    #[test]
    fn persona_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("style", "no comments").unwrap();
        assert_eq!(s.persona("style").unwrap().as_deref(), Some("no comments"));
    }

    /// `remove_persona` is the `/forget` path — present key drops, missing
    /// key reports false, the table stays usable.
    #[test]
    fn remove_persona_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("style", "no comments").unwrap();
        assert!(s.remove_persona("style").unwrap());
        assert!(!s.remove_persona("style").unwrap()); // already gone
        assert_eq!(s.persona("style").unwrap(), None);
    }

    /// Rows written before task normalization (blank/whitespace tasks) are
    /// filtered on read so they never reach the `<memory>` block.
    #[test]
    fn recent_episodes_skips_blank_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.add_episode(Episode {
            id: 0,
            workspace_id: 0,
            task: " \n ".into(),
            outcome: "success".into(),
            files: vec![],
            correction: None,
            created_at: 1,
        })
        .unwrap();
        s.add_episode(Episode {
            id: 0,
            workspace_id: 0,
            task: "real".into(),
            outcome: "success".into(),
            files: vec![],
            correction: None,
            created_at: 2,
        })
        .unwrap();
        let eps = s.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "real");
    }

    /// CJK text has no spaces, so a whitespace tokenizer makes each sentence
    /// one token. A near-duplicate Chinese fact must still merge.
    #[test]
    fn chinese_near_duplicate_facts_merge() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
            .unwrap();
        s.upsert_fact("项目使用 pnpm 而不是 npm 来安装依赖", 0.6)
            .unwrap();
        assert_eq!(s.list_facts(20).unwrap().len(), 1);
    }

    /// Different questions that share wording with different facts must
    /// surface different top hits. Before the fix every Chinese query returned
    /// the same first fact, because the vectors were near-orthogonal and
    /// ranking fell back to insertion order.
    ///
    /// This is lexical overlap, not semantics: "提交规范" does not reach
    /// "约定式提交" (one shared bigram, no shared meaning). Semantic recall
    /// needs a real embedding model, which is out of scope here.
    #[test]
    fn chinese_recall_follows_shared_wording() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
            .unwrap();
        s.upsert_fact("测试命令是 cargo test --workspace", 0.6)
            .unwrap();

        let install = s.recall_facts("怎么安装依赖", 1).unwrap();
        let test = s.recall_facts("测试命令怎么跑", 1).unwrap();
        assert!(
            install[0].text.contains("安装依赖"),
            "{:?}",
            install[0].text
        );
        assert!(test[0].text.contains("测试命令"), "{:?}", test[0].text);
    }

    /// A schema-1 database holds vectors from the old whitespace-only embed.
    /// Reopening must re-embed every fact in place: same ids, same text, and
    /// a vector that matches what the current `embed` produces. The version
    /// stamp moves to the current schema so the pass runs once.
    #[test]
    fn reopen_migrates_schema1_vectors_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let db_key = {
            let s = MemoryStore::open(&path, dir.path()).unwrap();
            s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
                .unwrap();
            let f = s.list_facts(10).unwrap().remove(0);
            // Simulate a schema-1 file: a stale vector and the old stamp.
            let stale = vec![0.0f32; 256];
            let w = s.db.begin_write().unwrap();
            {
                let mut t = w.open_table(FACTS).unwrap();
                let bytes = serde_json::to_vec(&Fact {
                    embedding: stale,
                    ..f.clone()
                })
                .unwrap();
                t.insert(f.id, bytes.as_slice()).unwrap();
            }
            {
                let mut m = w.open_table(META).unwrap();
                m.insert("schema_version", b"1".as_slice()).unwrap();
            }
            w.commit().unwrap();
            f.id
        };

        let s = MemoryStore::open(&path, dir.path()).unwrap();
        let f = s
            .list_facts(10)
            .unwrap()
            .into_iter()
            .find(|f| f.id == db_key)
            .expect("fact survives migration");
        assert_eq!(f.text, "项目使用 pnpm 而不是 npm 安装依赖");
        assert_eq!(f.embedding, s.embed(&f.text), "vector re-embedded");
        assert!(f.embedding.iter().any(|x| *x != 0.0), "not left stale");
        let r = s.db.begin_read().unwrap();
        let v = r
            .open_table(META)
            .unwrap()
            .get("schema_version")
            .unwrap()
            .unwrap();
        assert_eq!(v.value(), SCHEMA_VERSION.to_string().as_bytes());
    }

    /// The per-message block never re-sends a fact the session already read:
    /// without this a long conversation accumulates the same recall row until
    /// the useful ones are buried.
    #[test]
    fn facts_block_excluding_skips_facts_already_in_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
            .unwrap();
        s.upsert_fact("测试命令是 cargo test --workspace", 0.6)
            .unwrap();

        let (first, ids) = s
            .facts_block_excluding("怎么安装依赖", &HashSet::new())
            .unwrap();
        assert!(first.contains("pnpm"), "{first}");
        assert_eq!(ids.len(), 1);

        let seen: HashSet<u64> = ids.into_iter().collect();
        let (again, ids2) = s.facts_block_excluding("怎么安装依赖", &seen).unwrap();
        assert!(again.is_empty(), "fact re-sent: {again:?}");
        assert!(ids2.is_empty(), "{ids2:?}");
    }

    /// A workspace persona row stays out of another workspace's block; a
    /// global one follows the user everywhere.
    #[test]
    fn workspace_persona_does_not_leak_into_another_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let a = MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap();
        a.set_persona_scoped(PersonaScope::Workspace, "build", "use make -j4")
            .unwrap();
        a.set_persona("language", "zh").unwrap();
        assert!(a.stable_block().unwrap().contains("use make -j4"));

        let other_root = dir.path().join("other");
        std::fs::create_dir_all(&other_root).unwrap();
        let b = MemoryStore::open_with(a.db.clone(), &other_root).unwrap();
        let block = b.stable_block().unwrap();
        assert!(!block.contains("use make -j4"), "leaked: {block}");
        assert!(block.contains("language: zh"), "global row lost: {block}");
        assert_eq!(b.persona("build").unwrap(), None);
    }

    /// `/forget` means it in whichever scope the key landed.
    #[test]
    fn remove_persona_clears_both_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("style", "global value").unwrap();
        s.set_persona_scoped(PersonaScope::Workspace, "style", "workspace value")
            .unwrap();
        assert_eq!(
            s.persona("style").unwrap().as_deref(),
            Some("workspace value")
        );
        assert!(s.remove_persona("style").unwrap());
        assert_eq!(s.persona("style").unwrap(), None);
    }

    /// A scoped delete names one row: the same label kept in the other scope is
    /// a preference the user never clicked and has to survive.
    #[test]
    fn remove_persona_scoped_leaves_the_other_scope_alone() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("style", "global value").unwrap();
        s.set_persona_scoped(PersonaScope::Workspace, "style", "workspace value")
            .unwrap();

        assert!(s
            .remove_persona_scoped(PersonaScope::Workspace, "style")
            .unwrap());
        assert_eq!(
            s.persona_in(PersonaScope::Workspace, "style").unwrap(),
            None
        );
        assert_eq!(
            s.persona_in(PersonaScope::Global, "style")
                .unwrap()
                .as_deref(),
            Some("global value")
        );
        assert!(!s
            .remove_persona_scoped(PersonaScope::Workspace, "style")
            .unwrap()); // already gone

        s.set_persona_scoped(PersonaScope::Workspace, "style", "workspace value")
            .unwrap();
        assert!(s
            .remove_persona_scoped(PersonaScope::Global, "style")
            .unwrap());
        assert_eq!(s.persona_in(PersonaScope::Global, "style").unwrap(), None);
        assert_eq!(
            s.persona("style").unwrap().as_deref(),
            Some("workspace value")
        );
    }

    /// The delta is what makes a mid-session `remember` visible — the stable
    /// block is only rendered at spawn.
    #[test]
    fn persona_delta_reports_only_what_the_session_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("language", "zh").unwrap();
        let mut seen = HashMap::new();
        seen.insert("global:language".to_string(), "zh".to_string());

        let (block, rows) = s.persona_delta_block(&seen).unwrap();
        assert!(block.is_empty() && rows.is_empty(), "{block:?}");

        s.set_persona_scoped(PersonaScope::Workspace, "test_cmd", "cargo test")
            .unwrap();
        let (block, rows) = s.persona_delta_block(&seen).unwrap();
        assert!(block.contains("test_cmd: cargo test"), "{block}");
        assert_eq!(rows.len(), 1);
    }

    /// One label may live in both scopes with different values. Booking it by
    /// label alone makes the two rows disagree forever, and the one whose value
    /// differs is re-sent with every message.
    #[test]
    fn a_label_kept_in_both_scopes_stays_two_entries() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("style", "global value").unwrap();
        s.set_persona_scoped(PersonaScope::Workspace, "style", "workspace value")
            .unwrap();
        let all = s.all_persona().unwrap();
        assert_eq!(all.len(), 2, "{all:?}");

        // Exactly what the session books as delivered — every row, both scopes.
        let seen: HashMap<String, String> = all
            .iter()
            .map(|p| (p.delta_key(), p.value.clone()))
            .collect();
        let (block, rows) = s.persona_delta_block(&seen).unwrap();
        assert!(block.is_empty(), "re-sent in full: {block}");
        assert!(rows.is_empty(), "{rows:?}");
    }

    /// Two identical episodes in a row are one event: a retried prompt used
    /// to fill the whole recall budget with copies of itself.
    #[test]
    fn a_repeated_episode_collapses_into_one() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for at in [100u64, 160, 1200] {
            s.add_episode(Episode {
                id: 0,
                workspace_id: 0,
                task: "review the memory system".into(),
                outcome: "failed".into(),
                files: vec![],
                correction: None,
                created_at: at,
            })
            .unwrap();
        }
        // 100 and 160 are the same event; 1200 is a new one outside the window.
        assert_eq!(s.recent_episodes(5).unwrap().len(), 2);
    }

    /// Episodes left the system prompt — they answer nothing without the turn
    /// behind them — and the explicit read path still serves them.
    #[test]
    fn stable_block_carries_persona_only() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.add_episode(Episode {
            id: 0,
            workspace_id: 0,
            task: "fix the parser".into(),
            outcome: "success".into(),
            files: vec![],
            correction: None,
            created_at: 1,
        })
        .unwrap();
        let block = s.stable_block().unwrap();
        assert!(!block.contains("fix the parser"), "{block}");
        assert!(s.episode_lines(3).unwrap().contains("fix the parser"));
    }

    /// Two phrasings of one preference share a key: without the fold, a list
    /// grows one synonym per session and the reader has to reconcile them.
    #[test]
    fn a_rephrased_preference_lands_on_the_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let landed = s
            .set_persona_scoped(PersonaScope::Global, "note1", "用中文回答")
            .unwrap();
        assert_eq!(landed, "note1");

        let landed = s
            .set_persona_scoped(PersonaScope::Global, "note2", "中文回答")
            .unwrap();
        assert_eq!(landed, "note1", "a re-phrasing took a new key");
        assert_eq!(s.all_persona().unwrap().len(), 1);
        assert_eq!(s.persona("note1").unwrap().as_deref(), Some("中文回答"));
    }

    /// Two commands that merely share words stay apart — the fold is about
    /// wording, not about vocabulary.
    #[test]
    fn a_different_value_keeps_its_own_key() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("test_cmd", "cargo test").unwrap();
        let landed = s
            .set_persona_scoped(PersonaScope::Global, "fmt_cmd", "cargo test --workspace")
            .unwrap();
        assert_eq!(landed, "fmt_cmd");
        assert_eq!(s.all_persona().unwrap().len(), 2);
    }

    /// An explicit label is an update in place, never a fold into someone
    /// else's key.
    #[test]
    fn an_explicit_key_updates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona("language", "中文回答").unwrap();
        let landed = s
            .set_persona_scoped(PersonaScope::Global, "language", "用中文回答")
            .unwrap();
        assert_eq!(landed, "language");
        assert_eq!(s.all_persona().unwrap().len(), 1);
    }

    /// The purge removes exactly what the old rules wrote — legacy steer and
    /// denial facts, repeated episodes — and leaves real knowledge alone.
    #[test]
    fn purge_removes_legacy_facts_and_duplicate_episodes() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("user steered: 用 pnpm", 0.6).unwrap();
        s.upsert_fact("user corrected: denied tool(s): bash", 0.6)
            .unwrap();
        s.upsert_fact("项目使用 pnpm 安装依赖", 0.6).unwrap();

        // Retry duplicates from an older build: the write-time dedupe can only
        // stop new ones, so the rows go in raw.
        {
            let w = s.db.begin_write().unwrap();
            {
                let mut t = w.open_table(EPISODES).unwrap();
                for (id, at) in [(100u64, 10u64), (101, 20)] {
                    let e = Episode {
                        id,
                        workspace_id: s.workspace_id(),
                        task: "review the memory system".into(),
                        outcome: "failed".into(),
                        files: vec![],
                        correction: None,
                        created_at: at,
                    };
                    let bytes = serde_json::to_vec(&e).unwrap();
                    t.insert(id, bytes.as_slice()).unwrap();
                }
            }
            w.commit().unwrap();
        }

        let report = s.purge_polluted().unwrap();
        assert_eq!(report.superseded_facts, 2, "{report:?}");
        assert_eq!(report.duplicate_episodes, 1, "{report:?}");
        let facts = s.list_facts(10).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].text.contains("pnpm 安装依赖"), "{:?}", facts[0]);
        assert_eq!(s.recent_episodes(5).unwrap().len(), 1);
    }

    /// A layout-only fact an older build left at the default grade is graded
    /// down, not deleted — and a manual one is left alone.
    #[test]
    fn purge_grades_layout_facts_down() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("agent state lives in src/stores/agent-store.ts", 0.6)
            .unwrap();
        s.upsert_fact("项目使用 pnpm 安装依赖", 0.6).unwrap();
        let report = s.purge_polluted().unwrap();
        assert_eq!(report.layout_facts, 1, "{report:?}");
        let facts = s.list_facts(10).unwrap();
        let layout = facts
            .iter()
            .find(|f| f.text.contains("src/stores"))
            .unwrap();
        assert_eq!(layout.confidence, LAYOUT_FACT_CONFIDENCE);
        let other = facts.iter().find(|f| f.text.contains("pnpm")).unwrap();
        assert_eq!(other.confidence, 0.6);
    }

    /// A re-phrasing of a fact already in the block does not take a second
    /// slot, but counts as delivered — the next message must not send it as
    /// news. Measured pair from a real block (0.59).
    #[test]
    fn a_twin_fact_is_booked_but_not_repeated() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let a = s
            .upsert_fact(
                "Frontend state stores live in src/stores; agent state is in src/stores/agent-store.ts.",
                0.6,
            )
            .unwrap();
        let b = s
            .upsert_fact(
                "Frontend agent/session state is centralized in frontend/src/stores/agent-store.ts.",
                0.6,
            )
            .unwrap();
        // Recall checks paths against the tree; give both facts a real one.
        std::fs::create_dir_all(dir.path().join("src/stores")).unwrap();
        std::fs::create_dir_all(dir.path().join("frontend/src/stores")).unwrap();
        std::fs::write(dir.path().join("src/stores/agent-store.ts"), "").unwrap();
        std::fs::write(dir.path().join("frontend/src/stores/agent-store.ts"), "").unwrap();

        // A prompt naming both facts reaches the block with both — the point
        // here is what the block does with a pair that says one thing twice,
        // not how a query scores.
        let (block, ids) = s
            .facts_block_excluding(
                "Frontend state stores live in src/stores; agent state is in \
                 src/stores/agent-store.ts. Frontend agent/session state is \
                 centralized in frontend/src/stores/agent-store.ts.",
                &HashSet::new(),
            )
            .unwrap();
        assert_eq!(block.matches('•').count(), 1, "{block}");
        // Both are booked: the twin must not come back as news next message.
        assert!(ids.contains(&a) && ids.contains(&b), "{ids:?}");
    }

    /// The block's dedupe: token twins, plus a cross-language pair — never a
    /// same-language reordering, whose semantic score is just as high.
    #[test]
    fn block_dedupe_reads_tokens_first_and_languages_second() {
        let fact = |embedding: Vec<f32>, semantic: Vec<f32>| Fact {
            id: 1,
            workspace_id: 0,
            text: String::new(),
            embedding,
            semantic,
            semantic_model: String::new(),
            confidence: 0.6,
            created_at: 0,
            last_used_at: 0,
            source: FactSource::Legacy,
        };
        // Token-close: a twin whatever the semantic vectors say.
        assert!(says_the_same_thing(
            &[1.0, 0.0],
            &[1.0, 0.0],
            &fact(vec![1.0, 0.0], vec![0.0, 1.0])
        ));
        // No shared tokens, same meaning, semantic vectors close: two languages.
        assert!(says_the_same_thing(
            &[1.0, 0.0],
            &[1.0, 0.0],
            &fact(vec![0.0, 1.0], vec![1.0, 0.05])
        ));
        // No shared tokens and unrelated meaning: two facts.
        assert!(!says_the_same_thing(
            &[1.0, 0.0],
            &[1.0, 0.0],
            &fact(vec![0.0, 1.0], vec![0.0, 1.0])
        ));
        // Sharing some vocabulary is the token gate's business: however close
        // it reads semantically, the pair stays two lines. A rule and its
        // reversal look exactly like this in the semantic space.
        assert!(!says_the_same_thing(
            &[1.0, 0.0],
            &[1.0, 0.0],
            &fact(vec![0.2, 0.9798], vec![1.0, 0.05])
        ));
    }

    /// A row written before the semantic field existed still loads, and reads
    /// as "no semantic vector" — which is what sends it down the hash path.
    #[test]
    fn a_fact_without_a_semantic_vector_still_loads() {
        let row = r#"{"id":1,"workspace_id":2,"text":"用 pnpm","embedding":[0.1,0.2],
                      "confidence":0.6,"created_at":5,"last_used_at":0,"source":"legacy"}"#;
        let f: Fact = serde_json::from_str(row).unwrap();
        assert!(f.semantic.is_empty());
        // And an empty vector is not written back into the row.
        let out = serde_json::to_string(&f).unwrap();
        assert!(!out.contains("semantic"), "{out}");
    }

    /// The first open of a partition runs the purge on its own — the polluted
    /// rows were reaching every prompt while the clean-up waited on a command
    /// nobody ran — and the marker keeps it to that one time.
    #[test]
    fn opening_a_partition_purges_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("user steered: 用 pnpm", 0.6).unwrap();
        // The first open already stamped the marker: a reopen leaves it.
        let s = store(dir.path());
        assert_eq!(s.list_facts(10).unwrap().len(), 1);

        // A partition with no marker — a database from an older build.
        {
            let w = s.db.begin_write().unwrap();
            w.open_table(META)
                .unwrap()
                .remove(format!("purged:v{PURGE_VERSION}:{:016x}", s.workspace_id()).as_str())
                .unwrap();
            w.commit().unwrap();
        }
        let s = store(dir.path());
        assert!(s.list_facts(10).unwrap().is_empty());
    }

    /// Putting a fact in front of the model is usage — it feeds the recency
    /// half of the ranking and the eviction pass.
    #[test]
    fn a_recalled_fact_records_when_it_was_used() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("zzuse cargo build runs the whole workspace", 0.6)
            .unwrap();
        assert_eq!(s.list_facts(5).unwrap()[0].last_used_at, 0);

        let (block, ids) = s
            .facts_block_excluding("zzuse cargo build", &HashSet::new())
            .unwrap();
        assert!(block.contains("cargo build"), "{block}");
        assert_eq!(ids.len(), 1);
        assert!(
            s.list_facts(5).unwrap()[0].last_used_at > 0,
            "usage was not recorded"
        );
    }

    /// A fact nothing ever needed ages out; one that keeps being recalled does
    /// not, however old it is.
    #[test]
    fn evict_retires_unused_old_facts_and_keeps_used_ones() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let unused = s.upsert_fact("an unused old convention", 0.6).unwrap();
        let used = s.upsert_fact("a convention still in use", 0.6).unwrap();

        // Age both rows; only one was ever put in front of the model.
        let mut rows = s.list_facts(10).unwrap();
        for f in &mut rows {
            f.created_at = 1;
            f.last_used_at = if f.id == used { now() } else { 0 };
        }
        {
            let w = s.db.begin_write().unwrap();
            {
                let mut t = w.open_table(FACTS).unwrap();
                for f in &rows {
                    let bytes = serde_json::to_vec(f).unwrap();
                    t.insert(f.id, bytes.as_slice()).unwrap();
                }
            }
            w.commit().unwrap();
        }

        assert_eq!(s.evict_stale_facts(180, 0.65).unwrap(), 1);
        let left = s.list_facts(10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, used);
        let _ = unused;
    }

    /// A fact whose paths all vanished describes a tree that no longer exists:
    /// the recall path skips it rather than spending a slot on it.
    #[test]
    fn a_fact_naming_only_missing_paths_is_not_recalled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/live.rs"), "//").unwrap();
        let s = store(dir.path());
        s.upsert_fact("zzpath the parser lives in src/gone.rs", 0.6)
            .unwrap();
        s.upsert_fact("zzpath the parser lives in src/live.rs", 0.6)
            .unwrap();

        let (block, _) = s
            .facts_block_excluding("zzpath the parser", &HashSet::new())
            .unwrap();
        assert!(block.contains("src/live.rs"), "{block}");
        assert!(
            !block.contains("src/gone.rs"),
            "a vanished path was recalled: {block}"
        );
    }

    /// URLs, bare directories, globs, absolute paths and `file:line` forms are
    /// not tracked paths — one false positive hides a real fact.
    #[test]
    fn path_tokens_skip_urls_directories_and_line_numbers() {
        let toks = path_tokens(
            "see https://x.dev/a/b.md and src/ and crates/x/y.rs:12, also README.md and a/b",
        );
        assert_eq!(toks, vec!["crates/x/y.rs".to_string()]);
        assert!(path_tokens("no paths here").is_empty());
    }
}

#[cfg(test)]
mod relevance_tests {
    use super::*;

    fn store(dir: &Path) -> MemoryStore {
        MemoryStore::open(&dir.join("m.db"), dir).unwrap()
    }

    /// A question with no overlap with anything stored must not pull facts
    /// into the prompt. Before the floor, the block always carried the top 8
    /// neighbours even at similarity 0.
    #[test]
    fn unrelated_query_injects_no_facts() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for i in 0..4 {
            s.upsert_fact(format!("unrelated fact number {i} about apples"), 0.6)
                .unwrap();
        }
        let block = s.memory_block("completely different topic zebra").unwrap();
        assert!(
            !block.contains("relevant facts"),
            "unrelated question leaked facts: {block:?}"
        );
    }

    /// A question that does share wording with a stored fact still gets it.
    #[test]
    fn related_query_still_injects_the_fact() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
            .unwrap();
        let block = s.memory_block("怎么安装依赖").unwrap();
        assert!(block.contains("pnpm"), "related fact dropped: {block:?}");
    }

    /// The stable half never depends on the question — it is what the
    /// system prompt carries, so a varying prompt must not move it.
    #[test]
    fn stable_block_ignores_the_prompt_and_facts_block_tracks_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("项目使用 pnpm 而不是 npm 安装依赖", 0.6)
            .unwrap();
        s.set_persona("lang", "zh").unwrap();
        let stable = s.stable_block().unwrap();
        assert!(stable.contains("lang: zh"), "{stable:?}");
        assert!(
            !stable.contains("pnpm"),
            "facts leaked into stable: {stable:?}"
        );
        assert!(s.facts_block("怎么安装依赖").unwrap().contains("pnpm"));
        assert!(s.facts_block("zebra topic").unwrap().is_empty());
    }

    /// The distiller hands over paragraphs and CJK: one_line owns both cases —
    /// whitespace collapses, and a cap is cut on a char boundary, never a byte.
    #[test]
    fn one_line_collapses_whitespace_and_caps_on_a_char_boundary() {
        assert_eq!(one_line("  a \n b\t c  ", 40), "a b c");
        assert_eq!(one_line("安装依赖并运行测试", 4).chars().count(), 4);
        assert_eq!(one_line("ab", 0), "");
        assert_eq!(one_line("one", 3), "one");
    }

    /// Past the budget a persona list loses WHOLE entries and says how many:
    /// a short list that reads as complete is how a preference gets
    /// contradicted on the next turn. Built as entries, not writes — the write
    /// path folds identical values into one key, which is a different rule.
    #[test]
    fn stable_block_drops_whole_entries_and_reports_the_count() {
        let value: String = "值".repeat(100);
        let entries: Vec<Persona> = (0..40)
            .map(|i| Persona {
                key: format!("key{i:02}"),
                value: value.clone(),
                scope: PersonaScope::Global,
            })
            .collect();

        let block = MemoryStore::persona_block(&entries, STABLE_BLOCK_BUDGET);
        assert!(block.contains("── persona ──"), "{block}");
        assert!(block.contains("more"), "loss must stay visible: {block}");
        for line in block.lines().filter(|l| l.starts_with("key")) {
            let (_, shown) = line.split_once(": ").unwrap();
            assert_eq!(shown.chars().count(), 100, "torn entry: {line}");
        }
        assert!(
            block.chars().count() < STABLE_BLOCK_BUDGET + 40,
            "over budget"
        );
    }

    /// The truncation is honest in both directions: the block says how many
    /// entries it dropped, and the carried list names only what the model
    /// actually sees. Claiming a dropped entry as delivered would leave that
    /// preference permanently invisible.
    #[test]
    fn stable_block_carried_lists_only_what_fits() {
        let value: String = "值".repeat(100);
        let entries: Vec<Persona> = (0..40)
            .map(|i| Persona {
                key: format!("key{i:02}"),
                value: value.clone(),
                scope: PersonaScope::Global,
            })
            .collect();

        let (block, carried) = MemoryStore::persona_lines(&entries, STABLE_BLOCK_BUDGET);
        assert!(block.contains("more"), "loss must stay visible: {block}");
        assert!(carried.len() < 40, "every entry claimed as carried");
        for (delta, shown) in &carried {
            // The booked identity carries the scope; the block shows the label.
            let label = delta.split_once(':').expect("scoped delta key").1;
            assert!(
                block.contains(&format!("{label}: {shown}")),
                "{delta} reported carried but not in the block"
            );
        }
    }

    /// The write path normalizes, so one call site cannot put a newline into a
    /// block the reader renders one preference per line.
    #[test]
    fn set_persona_normalizes_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.set_persona(" style ", "be\n  brief ").unwrap();
        assert_eq!(s.persona("style").unwrap().as_deref(), Some("be brief"));
        assert!(s.set_persona("empty", "   ").is_err());
    }
}
