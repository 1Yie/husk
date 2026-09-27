//! `memory/store.rs` — hierarchical memory persistence.
//!
//! Layers: working / session / episodic / semantic / persona, partitioned per
//! workspace by `hash(canonical_root)`. redb plus a deterministic hash-embedding stand
//! in for the spec's LibSQL + FastEmbed, keeping the single-binary constraint, behind an
//! API that mirrors those semantics (facts + episodes + persona + top-k recall).
//!
//! Write path: summarize → extract facts → dedupe (`cosine > 0.92` merges). Read path:
//! embed prompt → HNSW top-8 facts + last 3 episodes → a `<memory>` block ≤2 KB.
//!
//! Recall runs an in-memory HNSW index over fact embeddings — approximate
//! nearest-neighbour instead of the old O(N) redb scan. The index is a cache:
//! writes insert into it after commit, deletes trigger a rebuild, and a 30s
//! TTL rebuild covers facts written by ANOTHER process (app vs CLI share the
//! same memory.db handle inside a process, but not across processes).

use std::collections::HashMap;
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
pub const SCHEMA_VERSION: u32 = 1;

/// Semantic memory fact — distilled convention/env/architecture note.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Fact {
    pub id: u64,
    pub workspace_id: u64,
    pub text: String,
    /// `f32` embedding — a deterministic hash-embed.
    pub embedding: Vec<f32>,
    /// `0.0..=1.0` — corrections seed 0.6; repeated confirmation raises.
    pub confidence: f32,
    /// When it was written (unix secs).
    pub created_at: u64,
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

/// Persona key-value (style prefs, approval patterns).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Persona {
    pub key: String,
    pub value: String,
}

/// The store — a shared `redb::Database` per `memory.db`, writes serialized
/// through a `Mutex` (redb is single-writer by design).
pub struct MemoryStore {
    pub(crate) db: Arc<Database>,
    /// `hash(canonical_root)` — partitions facts/episodes per workspace.
    workspace_id: u64,
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
            embed_dim: 256,
            index: RwLock::new((new_facts_index(), std::time::Instant::now())),
        };
        store.rebuild_index();
        Ok(store)
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
                    confidence,
                    created_at: now(),
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
            if !scored.is_empty() {
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                return Ok(scored.into_iter().take(k).map(|(_, f)| f).collect());
            }
        }
        // Fallback — empty index (fresh db) or every candidate invalidated.
        let mut scored: Vec<(f32, Fact)> = self
            .all_facts()?
            .into_iter()
            .filter(|f| f.workspace_id == self.workspace_id)
            .map(|f| (cosine(&f.embedding, &qemb), f))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(k).map(|(_, f)| f).collect())
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

    /// Append an episode.
    pub fn add_episode(&self, e: Episode) -> Result<u64, String> {
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let id = take_next_id(&w)?;
        let mut e = e;
        e.id = id;
        e.workspace_id = self.workspace_id;
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

    /// Wipe one area of the store. `facts`/`episodes` clear only THIS
    /// workspace's rows; `persona` clears the whole table (persona is
    /// global by design); `all` = facts + episodes + persona.
    /// Returns rows removed.
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

    pub fn set_persona(&self, key: &str, value: &str) -> Result<(), String> {
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        w.open_table(PERSONA)
            .map_err(|e| e.to_string())?
            .insert(key, value.as_bytes())
            .map_err(|e| e.to_string())?;
        w.commit().map_err(|e| e.to_string())
    }

    pub fn persona(&self, key: &str) -> Result<Option<String>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(PERSONA).map_err(|e| e.to_string())?;
        match t.get(key).map_err(|e| e.to_string())? {
            Some(v) => Ok(Some(String::from_utf8_lossy(v.value()).into_owned())),
            None => Ok(None),
        }
    }

    /// Remove a persona key — the `/forget` path. Returns whether it existed.
    pub fn remove_persona(&self, key: &str) -> Result<bool, String> {
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        let removed = w
            .open_table(PERSONA)
            .map_err(|e| e.to_string())?
            .remove(key)
            .map_err(|e| e.to_string())?
            .is_some();
        w.commit().map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// All persona entries for the `<memory>` block.
    pub fn all_persona(&self) -> Result<Vec<Persona>, String> {
        let r = self.db.begin_read().map_err(|e| e.to_string())?;
        let t = r.open_table(PERSONA).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for item in t.iter().map_err(|e| e.to_string())? {
            let (k, v) = item.map_err(|e| e.to_string())?;
            out.push(Persona {
                key: k.value().to_string(),
                value: String::from_utf8_lossy(v.value()).into_owned(),
            });
        }
        Ok(out)
    }

    /// Deterministic hash-embedding — a bag-of-hashed-tokens vector.
    /// Stands in for FastEmbed (ONNX pulls a C toolchain); the interface is
    /// identical so the swap is a backend change, not an API change.
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.embed_dim];
        for tok in text.split_whitespace() {
            let h = xxhash_rust::xxh3::xxh3_64(tok.as_bytes());
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

    /// The `<memory>` prompt block: top-k facts + last-3 episodes + persona,
    /// capped 2 KB (capability-roadmap.md §Read path).
    pub fn memory_block(&self, prompt: &str) -> Result<String, String> {
        let mut out = String::new();
        let facts = self.recall_facts(prompt, 8)?;
        if !facts.is_empty() {
            out.push_str("── relevant facts ──\n");
            for f in &facts {
                out.push_str(&format!("• {} (confidence {:.2})\n", f.text, f.confidence));
            }
        }
        let eps = self.recent_episodes(3)?;
        if !eps.is_empty() {
            out.push_str("── recent episodes ──\n");
            for e in &eps {
                out.push_str(&format!("• {} [{}]\n", e.task, e.outcome));
            }
        }
        let persona = self.all_persona()?;
        if !persona.is_empty() {
            out.push_str("── persona ──\n");
            for p in &persona {
                out.push_str(&format!("{}: {}\n", p.key, p.value));
            }
        }
        // UTF-8-safe 2 KB cap: round down to a char boundary — cutting inside
        // a multi-byte char would panic.
        if out.len() > 2048 {
            let mut n = 2048;
            while n > 0 && !out.is_char_boundary(n) {
                n -= 1;
            }
            out.truncate(n);
        }
        Ok(out)
    }
}

/// `hash(canonical_root)` — stable per-workspace partition.
fn workspace_id(root: &Path) -> u64 {
    xxhash_rust::xxh3::xxh3_64(root.to_string_lossy().as_bytes())
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
    fn memory_block_respects_2kb_cap() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for i in 0..20 {
            s.upsert_fact(format!("fact number {i} {}", "pad ".repeat(40)), 0.6)
                .unwrap();
        }
        let block = s.memory_block("query").unwrap();
        assert!(block.len() <= 2048);
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
}
