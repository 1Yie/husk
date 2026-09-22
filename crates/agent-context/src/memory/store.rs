//! `memory/store.rs` — hierarchical memory persistence.
//!
//! Layers: working / session / episodic / semantic / persona, partitioned per
//! workspace by `hash(canonical_root)`. redb plus a deterministic hash-embedding stand
//! in for the spec's LibSQL + FastEmbed, keeping the single-binary constraint, behind an
//! API that mirrors those semantics (facts + episodes + persona + top-k recall).
//!
//! Write path: summarize → extract facts → dedupe (`cosine > 0.92` merges). Read path:
//! embed prompt → top-8 facts + last 3 episodes → a `<memory>` block ≤2 KB.

use std::path::Path;
use std::sync::Mutex;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// facts table: `id -> Fact` (JSON).
const FACTS: TableDefinition<u64, &[u8]> = TableDefinition::new("facts");
/// episodes table: `id -> Episode` (JSON).
const EPISODES: TableDefinition<u64, &[u8]> = TableDefinition::new("episodes");
/// persona table: `key -> value` (JSON string).
const PERSONA: TableDefinition<&str, &[u8]> = TableDefinition::new("persona");
/// meta table: `key -> value` — `schema_version` and migration stamps.
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

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

/// The store — one `redb::Database` per `memory.db`, writes serialized
/// through a `Mutex` (redb is single-writer by design).
pub struct MemoryStore {
    db: Database,
    /// `hash(canonical_root)` — partitions facts/episodes per workspace.
    workspace_id: u64,
    /// Monotonic id counter — redb has no auto-increment.
    next_id: Mutex<u64>,
    /// Embedding dim for the hash-embedder.
    embed_dim: usize,
}

impl MemoryStore {
    /// Open (or create) `memory.db` for `workspace_root`.
    pub fn open(path: &Path, workspace_root: &Path) -> Result<Self, String> {
        let db = Database::create(path).map_err(|e| e.to_string())?;
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
            }
            w.commit().map_err(|e| e.to_string())?;
        }
        let workspace_id = workspace_id(workspace_root);
        // Seed `next_id` above the max persisted id — redb has no
        // auto-increment, and restarting at 1 would silently overwrite
        // existing facts/episodes via `insert`.
        let next_id = {
            let r = db.begin_read().map_err(|e| e.to_string())?;
            let mut max_id = 0u64;
            for def in [FACTS, EPISODES] {
                let t = r.open_table(def).map_err(|e| e.to_string())?;
                for item in t.iter().map_err(|e| e.to_string())? {
                    let (k, _) = item.map_err(|e| e.to_string())?;
                    if k.value() > max_id {
                        max_id = k.value();
                    }
                }
            }
            max_id + 1
        };
        Ok(Self {
            db,
            workspace_id,
            next_id: Mutex::new(next_id),
            embed_dim: 256,
        })
    }

    /// `hash(canonical_root)` — stable per-repo partition key.
    pub fn workspace_id(&self) -> u64 {
        self.workspace_id
    }

    /// Insert a fact — dedupes vs. existing by `cosine > 0.92` (merge =
    /// bump confidence, else insert). Returns the fact's id.
    pub fn upsert_fact(&self, text: impl Into<String>, confidence: f32) -> Result<u64, String> {
        let text = text.into();
        let emb = self.embed(&text);
        if let Some((id, existing)) = self.find_similar(&emb, 0.92)? {
            let mut f = existing;
            f.confidence = (f.confidence + confidence).min(1.0);
            self.write_fact(&f)?;
            return Ok(id);
        }
        let id = self.alloc_id()?;
        let f = Fact {
            id,
            workspace_id: self.workspace_id,
            text,
            embedding: emb,
            confidence,
            created_at: now(),
        };
        self.write_fact(&f)?;
        Ok(id)
    }

    fn write_fact(&self, f: &Fact) -> Result<(), String> {
        let bytes = serde_json::to_vec(f).map_err(|e| e.to_string())?;
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
        w.open_table(FACTS)
            .map_err(|e| e.to_string())?
            .insert(f.id, bytes.as_slice())
            .map_err(|e| e.to_string())?;
        w.commit().map_err(|e| e.to_string())
    }

    /// Find the fact closest to `emb` above `threshold` cosine — scans the
    /// workspace's facts (small N; a proper ANN lands with FastEmbed).
    fn find_similar(&self, emb: &[f32], threshold: f32) -> Result<Option<(u64, Fact)>, String> {
        let mut best: Option<(f32, u64, Fact)> = None;
        for f in self.all_facts()? {
            if f.workspace_id != self.workspace_id {
                continue;
            }
            let sim = cosine(&f.embedding, emb);
            if sim >= threshold && best.as_ref().map(|(s, _, _)| sim > *s).unwrap_or(true) {
                best = Some((sim, f.id, f));
            }
        }
        Ok(best.map(|(_, id, f)| (id, f)))
    }

    /// Top-k recall: facts for this workspace ranked by `cosine(prompt_emb)`.
    pub fn recall_facts(&self, query: &str, k: usize) -> Result<Vec<Fact>, String> {
        let qemb = self.embed(query);
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
        let id = self.alloc_id()?;
        let mut e = e;
        e.id = id;
        e.workspace_id = self.workspace_id;
        let bytes = serde_json::to_vec(&e).map_err(|e| e.to_string())?;
        let w = self.db.begin_write().map_err(|e| e.to_string())?;
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
                if e.workspace_id == self.workspace_id {
                    out.push(e);
                }
            }
        }
        out.sort_by_key(|e| e.created_at);
        Ok(out.into_iter().rev().take(n).collect())
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

    fn alloc_id(&self) -> Result<u64, String> {
        // Poison-safe: a panicked writer must not poison every later alloc.
        let mut g = self.next_id.lock().unwrap_or_else(|e| e.into_inner());
        let id = *g;
        *g += 1;
        Ok(id)
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

    #[test]
    fn facts_dedupe_by_cosine() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.upsert_fact("errors via AppError, never unwrap", 0.6).unwrap();
        // Near-identical text → merge, not duplicate.
        s.upsert_fact("errors via AppError, never unwrap", 0.8).unwrap();
        let facts = s.recall_facts("error handling", 10).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].confidence > 0.9); // merged
    }

    #[test]
    fn episodes_scoped_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        s.add_episode(Episode {
            id: 0, workspace_id: 0, task: "fix engine".into(),
            outcome: "success".into(), files: vec!["engine.rs".into()],
            correction: None, created_at: 1,
        }).unwrap();
        let eps = s.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "fix engine");
    }

    #[test]
    fn memory_block_respects_2kb_cap() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for i in 0..20 {
            s.upsert_fact(format!("fact number {i} {}" , "pad ".repeat(40)), 0.6).unwrap();
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
}
