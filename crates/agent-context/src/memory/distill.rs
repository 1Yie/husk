//! `memory/distill.rs` — the post-turn background extractor.
//!
//! Contract (capability-roadmap.md §Write path): at `Finished`/`Failed`,
//! spawn a low-priority task — summarize the turn → extract candidate facts
//! → dedupe → write episode + facts. **Never blocks the turn.**
//!
//! The distiller is deliberately model-agnostic: the engine hands it a
//! plain `TurnRecord` and a `summarize` callback (the kernel supplies the
//! active provider's text-generation), so this crate stays provider-free.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::store::{Episode, MemoryStore};

/// One-shot model call the kernel injects — takes the distill prompt, returns
/// the model's raw text. `None` keeps the deterministic path (tests, headless
/// and no-provider contexts).
pub type Summarizer =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

/// The model pass's output — everything optional; missing pieces degrade to
/// the deterministic distill, not to lost data.
#[derive(Debug, Default, serde::Deserialize)]
struct DistillJson {
    /// One-line turn summary — wins over the deterministic first-line when
    /// present.
    task: Option<String>,
    /// Project-level durable knowledge (build/test commands, conventions).
    #[serde(default)]
    facts: Vec<String>,
    /// Durable user preferences → persona k/v.
    #[serde(default)]
    persona: serde_json::Map<String, serde_json::Value>,
}

/// What one finished turn left behind — the distiller's input.
pub struct TurnRecord {
    /// One-line summary of the user's ask.
    pub task: String,
    /// `"success" | "failed" | "steered"`.
    pub outcome: String,
    /// Files the turn wrote (from the HunkTracker).
    pub files: Vec<String>,
    /// A user correction/deny reason — highest-value fact seed.
    pub correction: Option<String>,
    /// Steering text injected mid-turn, if any.
    pub steered_with: Option<String>,
}

/// Extract candidate facts from a turn — deterministic rules:
///
/// - A **user correction** (`deny_tool` reason or steering) → a fact at
///   `confidence: 0.6`.
/// - A **repeated outcome** (same task family succeeding) → confidence bump
///   via the store's dedupe merge.
/// - Anything matching the sanitizer denylist is **never** distilled.
///
/// With a [`Summarizer`] attached, one extra model call per turn produces a
/// semantic task line plus candidate facts and persona entries — the same
/// tables the deterministic rules, the `remember` tool, and `/remember`
/// write, so all writers share one dedupe/recall path.
pub struct TurnDistiller {
    store: Arc<MemoryStore>,
    /// Model callback — `None` ⇒ deterministic-only distill.
    summarizer: Option<Summarizer>,
}

impl TurnDistiller {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self {
            store,
            summarizer: None,
        }
    }

    /// Attach the kernel's model callback — enables the semantic pass on top
    /// of the deterministic rules.
    pub fn with_summarizer(mut self, s: Summarizer) -> Self {
        self.summarizer = Some(s);
        self
    }

    /// Fire-and-forget distill — runs in a spawned task, never blocks.
    pub fn spawn_distill(self: &Arc<Self>, record: TurnRecord) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            this.distill(record).await;
        })
    }

    /// The synchronous distill body — extracted for tests.
    pub async fn distill(&self, record: TurnRecord) {
        // 0. Model pass (optional) — its `task`/`facts`/`persona` land on the
        // same tables as the deterministic writes below. An unparsable or
        // missing response costs nothing: the deterministic path still runs.
        let distilled = self.model_pass(&record).await;

        // 1. Write the episode — the raw "what happened". `task` is
        // normalized to its first non-empty line (callers hand over the
        // full prompt; a multi-KB paste is not a sidebar entry). A turn
        // with no readable task leaves no episode — whitespace-only rows
        // are noise in the recall block.
        let task = distilled
            .as_ref()
            .and_then(|d| d.task.as_deref())
            .map(|t| one_line(t, 200))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| one_line(&record.task, 200));
        if !task.is_empty() {
            let episode = Episode {
                id: 0,
                workspace_id: self.store.workspace_id(),
                task,
                outcome: record.outcome.clone(),
                files: record.files.clone(),
                correction: record.correction.clone(),
                created_at: now(),
            };
            let _ = self.store.add_episode(episode);
        }

        // 2. Corrections seed candidate facts at 0.6 confidence.
        if let Some(c) = &record.correction {
            if !c.trim().is_empty() && !looks_secret(c) {
                let _ = self.store.upsert_fact(format!("user corrected: {c}"), 0.6);
            }
        }
        if let Some(steer) = &record.steered_with {
            if !steer.trim().is_empty() && !looks_secret(steer) {
                let _ = self
                    .store
                    .upsert_fact(format!("user steered: {steer}"), 0.6);
            }
        }

        // 3. Model-extracted candidates — same dedupe/secret gates as the
        // deterministic facts; persona lands in the same k/v table the
        // `remember` tool and `/remember` write.
        if let Some(d) = distilled {
            for fact in d.facts.iter().take(5) {
                let fact = fact.trim();
                if fact.is_empty() || fact.len() > 300 || looks_secret(fact) {
                    continue;
                }
                let _ = self.store.upsert_fact(fact.to_string(), 0.6);
            }
            for (k, v) in d.persona.iter().take(3) {
                let key = persona_key(k);
                let Some(value) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                if key.is_empty() || value.len() > 200 || looks_secret(value) {
                    continue;
                }
                let _ = self.store.set_persona(&key, value);
            }
        }
    }

    /// The optional model pass — absent callback, transport failure, or an
    /// unparsable body all degrade to `None` (deterministic-only distill).
    async fn model_pass(&self, record: &TurnRecord) -> Option<DistillJson> {
        let call = self.summarizer.as_ref()?;
        let out = call(distill_prompt(record)).await?;
        let parsed = parse_distill(&out);
        if parsed.is_none() {
            tracing::debug!("memory distill: unparsable model output");
        }
        parsed
    }
}

/// Build the model's distill prompt — compact, one user message, strict JSON
/// contract. The raw task text is truncated generously (the model reads
/// multi-line context better than `one_line` does) but bounded so a paste
/// bomb can't blow the call up.
fn distill_prompt(record: &TurnRecord) -> String {
    let mut p = String::from(
        "Distill one coding-agent turn into durable memory. Reply with ONLY a JSON object \
         (no prose, no fences):\n\
         {\"task\": \"<one-line summary>\", \"facts\": [\"<project fact>\"], \
         \"persona\": {\"<key>\": \"<user preference>\"}}\n\n\
         - \"task\": one line, ≤120 chars, what the turn actually did.\n\
         - \"facts\": project knowledge worth remembering across sessions — build/test \
         commands, conventions, architecture. NOT user preferences, NOT what is already \
         obvious from the repo. `[]` when nothing durable.\n\
         - \"persona\": stable user preferences (language, style, workflow). `{}` when \
         none. Keys are short snake_case labels.\n\
         - Never emit secrets, tokens, absolute paths, or one-off requests.\n\nTurn:\n",
    );
    p.push_str(&format!(
        "asked: {}\noutcome: {}\n",
        truncate(&record.task, 2000),
        record.outcome
    ));
    if !record.files.is_empty() {
        let files: Vec<&str> = record.files.iter().map(|f| truncate(f, 120)).collect();
        p.push_str(&format!("files: {}\n", files.join(", ")));
    }
    if let Some(c) = &record.correction {
        p.push_str(&format!("user correction: {}\n", truncate(c, 400)));
    }
    if let Some(s) = &record.steered_with {
        p.push_str(&format!("user steering: {}\n", truncate(s, 400)));
    }
    p
}

/// Parse the model's reply tolerantly — strip a ```` ```json ```` fence if
/// one slipped in, then take the outermost `{…}` span.
fn parse_distill(out: &str) -> Option<DistillJson> {
    let t = out.trim();
    let t = t
        .strip_prefix("```")
        .map(|s| s.strip_prefix("json").unwrap_or(s).trim_start())
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    serde_json::from_str(t.get(start..=end)?).ok()
}

/// Normalize a model-chosen persona key to a safe label — alphanumerics plus
/// `_` `-` `:` survive, everything else folds to `_`, capped at 40 chars.
fn persona_key(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | ':') {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect()
}

/// Byte-cap a str on a char boundary.
fn truncate(text: &str, cap: usize) -> &str {
    let mut end = text.len().min(cap);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// First non-empty line of `text`, trimmed and capped at `cap` bytes on a
/// char boundary — the one-line task summary `Episode.task` promises.
fn one_line(text: &str, cap: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut end = line.len().min(cap);
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

/// Never distill secrets — the same denylist patterns as env sanitization.
fn looks_secret(text: &str) -> bool {
    let t = text.to_uppercase();
    [
        "_KEY",
        "_TOKEN",
        "_SECRET",
        "_PASSWORD",
        "PRIVATE",
        "AWS_",
        "GITHUB_",
    ]
    .iter()
    .any(|p| t.contains(p))
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
    use std::path::Path;

    fn distiller(dir: &Path) -> Arc<TurnDistiller> {
        let store = Arc::new(MemoryStore::open(&dir.join("m.db"), dir).unwrap());
        Arc::new(TurnDistiller::new(store))
    }

    fn record(task: &str) -> TurnRecord {
        TurnRecord {
            task: task.into(),
            outcome: "success".into(),
            files: vec![],
            correction: None,
            steered_with: None,
        }
    }

    /// `task` carries the caller's full prompt — the episode keeps the first
    /// non-empty line only, so a multi-line paste stays a one-line summary.
    #[tokio::test]
    async fn task_is_normalized_to_its_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let d = distiller(dir.path());
        d.distill(TurnRecord {
            task: "\n\nfix the crash\nwith a longer explanation here".into(),
            ..record("")
        })
        .await;
        let eps = d.store.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "fix the crash");
    }

    /// A turn whose task is whitespace-only writes no episode — blank rows
    /// are pure noise in the recall block.
    #[tokio::test]
    async fn blank_task_writes_no_episode() {
        let dir = tempfile::tempdir().unwrap();
        let d = distiller(dir.path());
        d.distill(record("  \n\n  ")).await;
        assert!(d.store.recent_episodes(5).unwrap().is_empty());
    }

    /// A correction becomes a `user corrected:` fact; a whitespace-only
    /// correction does not.
    #[tokio::test]
    async fn corrections_seed_facts_and_blank_ones_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let d = distiller(dir.path());
        d.distill(TurnRecord {
            correction: Some("denied tool(s): apply_patch".into()),
            ..record("do a thing")
        })
        .await;
        d.distill(TurnRecord {
            correction: Some("   ".into()),
            ..record("another")
        })
        .await;
        let facts = d.store.recall_facts("apply_patch", 10).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].text.contains("denied tool(s): apply_patch"));
    }

    /// `one_line` caps at the byte budget without splitting a multi-byte char.
    #[test]
    fn one_line_trims_and_caps_on_char_boundary() {
        let long = format!("{}\nignored", "x".repeat(300));
        assert_eq!(one_line(&long, 200).len(), 200);
        let mb = "中".repeat(100); // 300 bytes
        assert_eq!(one_line(&mb, 7), "中中"); // 6 bytes — boundary-safe, not 7
        assert_eq!(one_line("\n\n\n", 10), "");
    }

    /// A canned model reply lands on the same tables the deterministic rules
    /// write: `task` into the episode, `facts` into facts, `persona` into the
    /// shared k/v table.
    #[tokio::test]
    async fn summarizer_output_writes_episode_facts_and_persona() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap());
        let d = Arc::new(
            TurnDistiller::new(store.clone()).with_summarizer(Arc::new(|_prompt| {
                Box::pin(async move {
                    Some(
                        r#"{"task": "fixed the macOS build",
                           "facts": ["release builds need rustflags for symbols"],
                           "persona": {"language": "zh"}}"#
                            .into(),
                    )
                })
            })),
        );
        d.distill(TurnRecord {
            task: "help me fix the macOS CI".into(),
            outcome: "success".into(),
            files: vec!["macos.rs".into()],
            correction: None,
            steered_with: None,
        })
        .await;

        let eps = store.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "fixed the macOS build"); // model's task wins

        let facts = store.recall_facts("release builds", 5).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].text.contains("rustflags"));

        assert_eq!(store.persona("language").unwrap().as_deref(), Some("zh"));
    }

    /// An unparsable model body degrades to the deterministic path — episode
    /// still written from `one_line`, correction facts still seeded.
    #[tokio::test]
    async fn unparsable_summarizer_falls_back_to_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap());
        let d = Arc::new(
            TurnDistiller::new(store.clone()).with_summarizer(Arc::new(|_p| {
                Box::pin(async move { Some("not json at all".into()) })
            })),
        );
        d.distill(TurnRecord {
            task: "fix the thing".into(),
            outcome: "success".into(),
            files: vec![],
            correction: Some("denied tool(s): bash".into()),
            steered_with: None,
        })
        .await;
        assert_eq!(store.recent_episodes(5).unwrap()[0].task, "fix the thing");
        assert_eq!(store.recall_facts("denied", 5).unwrap().len(), 1);
    }

    /// A fenced ```json reply still parses.
    #[test]
    fn parse_distill_tolerates_fences_and_prose() {
        let fenced = "```json\n{\"task\": \"x\", \"facts\": [], \"persona\": {}}\n```";
        assert_eq!(parse_distill(fenced).unwrap().task.as_deref(), Some("x"));
        let prose = "sure! {\"task\": \"y\", \"facts\": [\"f\"], \"persona\": {}} done";
        assert_eq!(parse_distill(prose).unwrap().facts, vec!["f"]);
        assert!(parse_distill("no braces").is_none());
    }

    /// Model-chosen persona keys are normalized to safe labels.
    #[test]
    fn persona_key_strips_unsafe_chars() {
        assert_eq!(persona_key(" Prefers Chinese "), "Prefers_Chinese");
        assert_eq!(persona_key("a".repeat(60).as_str()), "a".repeat(40));
        assert_eq!(persona_key("not:allowed chars!"), "not:allowed_chars_");
    }
}
