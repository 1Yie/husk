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

use super::store::{Episode, MemoryStore, PersonaScope};

/// Persona labels the distill prompt lists. Enough to cover a real list
/// (dozens of entries) without letting the prompt grow with the store.
const PERSONA_KEYS_SHOWN: usize = 60;

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
    facts: Vec<DistilledFact>,
    /// Durable user preferences → persona k/v.
    #[serde(default)]
    persona: serde_json::Map<String, serde_json::Value>,
}

/// One candidate fact. A bare string is what the format used to be, and what
/// a model that ignores the grading still sends — it reads as `observed`.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum DistilledFact {
    Graded {
        text: String,
        #[serde(default)]
        durable: Durability,
    },
    Plain(String),
}

/// How sure the model is that a fact outlives the turn. Three named grades,
/// not a free number: a model asked for `0.0–1.0` answers 0.8 for everything.
#[derive(Debug, Default, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Durability {
    /// The user stated it as a rule ("always run fmt before test").
    Stated,
    /// Seen to hold in this turn — a command that worked, a convention used.
    #[default]
    Observed,
    /// Inferred, or only true of the tree as it is now.
    Guess,
}

impl Durability {
    fn confidence(self) -> f32 {
        match self {
            Durability::Stated => 0.8,
            Durability::Observed => 0.6,
            Durability::Guess => 0.4,
        }
    }
}

impl DistilledFact {
    fn parts(&self) -> (&str, Durability) {
        match self {
            DistilledFact::Graded { text, durable } => (text, *durable),
            DistilledFact::Plain(text) => (text, Durability::Observed),
        }
    }
}

/// A fact that only says where something lives — "X is in src/foo", "tests
/// live under __tests__". The tree answers that better than memory does, and
/// such facts were most of what reached the recall block. Kept, but graded as
/// a guess, so the lifecycle retires them unless they keep proving useful.
pub(crate) fn is_layout_fact(text: &str) -> bool {
    let lower = text.to_lowercase();
    const MARKERS: [&str; 9] = [
        " lives in ",
        " live in ",
        " lives under ",
        " live under ",
        " is in ",
        " are in ",
        " located in ",
        " is located ",
        " reside in ",
    ];
    const ZH_MARKERS: [&str; 5] = ["位于", "放在", "存放在", "在目录", "目录下"];
    let says_where =
        MARKERS.iter().any(|m| lower.contains(m)) || ZH_MARKERS.iter().any(|m| text.contains(m));
    says_where && text.contains('/')
}

/// What one finished turn left behind — the distiller's input.
pub struct TurnRecord {
    /// The user's ask, verbatim — the episode keeps its first line only.
    pub task: String,
    /// `"success" | "failed"`.
    pub outcome: String,
    /// Files the turn wrote (from the HunkTracker).
    pub files: Vec<String>,
    /// A correction/denial — the evidence the model pass needs to tell a
    /// durable rule ("先跑 fmt 再 test") from a one-off ("not that file").
    pub correction: Option<String>,
    /// Steering text injected mid-turn, if any.
    pub steered_with: Option<String>,
    /// What the turn answered, capped. Without it the memory pass only ever
    /// saw the ask, and wrote the user's bug report down as a project
    /// convention.
    pub reply: Option<String>,
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
        // A failed turn is not evidence of anything durable — it used to run
        // the pass anyway, which bought a duplicate episode and another
        // billing line on every retry.
        let distilled = if record.outcome == "failed" {
            None
        } else {
            self.model_pass(&record).await
        };

        // Every upsert below runs the semantic encoder when it is loaded —
        // CPU work that must not hold a runtime worker shared by every
        // session, so the writes go to the blocking pool.
        let store = self.store.clone();
        let _ =
            tokio::task::spawn_blocking(move || write_distilled(&store, record, distilled)).await;
    }

    /// The optional model pass — absent callback, transport failure, or an
    /// unparsable body all degrade to `None` (deterministic-only distill).
    async fn model_pass(&self, record: &TurnRecord) -> Option<DistillJson> {
        let call = self.summarizer.as_ref()?;
        // The labels already in use — "reuse an existing key" is only an
        // instruction the model can follow if it sees them.
        let mut keys: Vec<String> = self
            .store
            .all_persona()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.key)
            .collect();
        keys.dedup();
        let out = call(distill_prompt(record, &keys)).await?;
        let parsed = parse_distill(&out);
        if parsed.is_none() {
            tracing::debug!("memory distill: unparsable model output");
        }
        parsed
    }
}

/// The store writes of one distill — synchronous, run on the blocking pool.
fn write_distilled(store: &MemoryStore, record: TurnRecord, distilled: Option<DistillJson>) {
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
        // A correction can quote anything the user pasted: the sanitizer
        // that guards facts guards the episode column too, and the row now
        // reaches the model through `memory_search`.
        let correction = record
            .correction
            .clone()
            .filter(|c| !c.trim().is_empty() && !looks_secret(c));
        let episode = Episode {
            id: 0,
            workspace_id: store.workspace_id(),
            task,
            outcome: record.outcome.clone(),
            files: record.files.clone(),
            correction,
            created_at: now(),
        };
        let _ = store.add_episode(episode);
    }

    // 2. Corrections and steers stay on the episode (`correction`) and
    // feed the model pass as evidence. They used to seed
    // `user corrected: …` / `user steered: …` facts directly, which made
    // every "改一下这个文件" and every denied tool a permanent pseudo-fact:
    // a one-off complaint is not project knowledge.

    // 3. Model-extracted candidates — same dedupe/secret gates as the
    // deterministic facts; persona lands in the same k/v table the
    // `remember` tool and `/remember` write, scoped to this workspace:
    // something inferred inside one repo must not follow the user into
    // the next one.
    if let Some(d) = distilled {
        for fact in d.facts.iter().take(5) {
            let (fact, durable) = fact.parts();
            let fact = fact.trim();
            if fact.is_empty() || fact.len() > 300 || looks_secret(fact) {
                continue;
            }
            let durable = if is_layout_fact(fact) {
                Durability::Guess
            } else {
                durable
            };
            let _ = store.upsert_fact(fact.to_string(), durable.confidence());
        }
        for (k, v) in d.persona.iter().take(3) {
            let key = persona_key(k);
            let Some(value) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            if key.is_empty() || value.len() > 200 || looks_secret(value) {
                continue;
            }
            let _ = store.set_persona_scoped(PersonaScope::Workspace, &key, value);
        }
    }
}

/// Build the model's distill prompt — compact, one user message, strict JSON
/// contract. The raw task text is truncated generously (the model reads
/// multi-line context better than `one_line` does) but bounded so a paste
/// bomb can't blow the call up.
fn distill_prompt(record: &TurnRecord, persona_keys: &[String]) -> String {
    let mut p = String::from(
        "Distill one coding-agent turn into durable memory. Reply with ONLY a JSON object \
         (no prose, no fences):\n\
         {\"task\": \"<one-line summary>\", \"facts\": [{\"text\": \"<project fact>\", \
         \"durable\": \"stated|observed|guess\"}], \
         \"persona\": {\"<key>\": \"<user preference>\"}}\n\n\
         - \"task\": one line, ≤120 chars, what the turn actually did.\n\
         - \"facts\": project knowledge a fresh session would still need — build/test \
         commands, conventions, architecture. NOT the user's request, NOT a bug report or \
         a TODO, NOT anything already visible in the repo or its tests, NOT where a file \
         or directory lives. `[]` when nothing durable.\n\
         - \"durable\": `stated` = the user said it as a rule; `observed` = it held in \
         this turn (a command that worked, a convention the code follows); `guess` = \
         inferred, or true only of the tree as it is right now.\n\
         - \"persona\": stable user preferences (language, style, workflow). `{}` when \
         none. Keys are short snake_case labels — reuse an existing one rather than \
         coining a near-synonym.\n\
         - Never emit secrets, tokens, absolute paths, or one-off requests.\n\nTurn:\n",
    );
    p.push_str(&format!(
        "asked: {}\noutcome: {}\n",
        truncate(&record.task, 2000),
        record.outcome
    ));
    if let Some(reply) = &record.reply {
        p.push_str(&format!("reply: {}\n", truncate(reply, 800)));
    }
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
    if !persona_keys.is_empty() {
        let keys: Vec<&str> = persona_keys
            .iter()
            .take(PERSONA_KEYS_SHOWN)
            .map(|k| truncate(k, 40))
            .collect();
        p.push_str(&format!("existing persona keys: {}\n", keys.join(", ")));
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

/// Never distill or remember secrets. Two checks, because either alone misses
/// real cases: the env-var name denylist catches `OPENAI_API_KEY=…`, and the
/// value-shape check catches a bare `sk-…` token or `password: …` pasted into
/// a sentence. Ordinary preferences pass both.
pub fn looks_secret(text: &str) -> bool {
    let t = text.to_uppercase();
    let by_name = [
        "_KEY",
        "_TOKEN",
        "_SECRET",
        "_PASSWORD",
        "PRIVATE",
        "AWS_",
        "GITHUB_",
    ]
    .iter()
    .any(|p| t.contains(p));
    by_name || has_credential_shape(text)
}

/// Known credential prefixes, and a secret keyword followed by `:` or `=`
/// and a value. The keyword must be followed by an assignment, so a sentence
/// that merely mentions a password is not flagged.
fn has_credential_shape(text: &str) -> bool {
    const PREFIXES: [&str; 6] = ["sk-", "ghp_", "github_pat_", "AKIA", "xox", "AIza"];
    for word in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        if PREFIXES.iter().any(|p| word.starts_with(p)) && word.len() >= 16 {
            return true;
        }
    }
    let lower = text.to_lowercase();
    for kw in [
        "password", "passwd", "token", "secret", "api key", "api_key", "apikey",
    ] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(kw) {
            let after = &lower[from + pos + kw.len()..];
            let after = after.trim_start();
            if let Some(rest) = after.strip_prefix(':').or_else(|| after.strip_prefix('=')) {
                let value = rest.trim_start();
                if value.chars().take_while(|c| !c.is_whitespace()).count() >= 4 {
                    return true;
                }
            }
            from += pos + kw.len();
        }
    }
    false
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
            reply: None,
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

    /// A correction belongs to its turn, not to the fact table: a denied tool
    /// is a one-off decision, not something a fresh session needs.
    #[tokio::test]
    async fn corrections_stay_on_the_episode_and_write_no_fact() {
        let dir = tempfile::tempdir().unwrap();
        let d = distiller(dir.path());
        d.distill(TurnRecord {
            correction: Some("denied tool(s): apply_patch".into()),
            ..record("do a thing")
        })
        .await;
        d.distill(TurnRecord {
            correction: Some("先跑 fmt 再 test".into()),
            steered_with: Some("不要改那个文件".into()),
            ..record("another")
        })
        .await;
        assert!(d
            .store
            .recall_facts("apply_patch fmt test 那个文件", 10)
            .unwrap()
            .is_empty());
        let eps = d.store.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().all(|e| e.correction.is_some()));
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
            reply: None,
        })
        .await;

        let eps = store.recent_episodes(5).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].task, "fixed the macOS build"); // model's task wins

        let facts = store.recall_facts("release builds", 5).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].text.contains("rustflags"));

        assert_eq!(store.persona("language").unwrap().as_deref(), Some("zh"));
        // A preference the model inferred here stays with this workspace.
        let persona = store.all_persona().unwrap();
        assert_eq!(persona.len(), 1);
        assert_eq!(persona[0].scope, PersonaScope::Workspace);
    }

    /// The grade decides the stored confidence — and a fact that only says
    /// where something lives is a guess whatever the model called it. A bare
    /// string still parses, as `observed`.
    #[tokio::test]
    async fn graded_facts_land_at_their_grades_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap());
        let d = TurnDistiller::new(store.clone()).with_summarizer(Arc::new(|_prompt| {
            Box::pin(async move {
                Some(
                    r#"{"task": "t", "facts": [
                        {"text": "always run cargo fmt before cargo test", "durable": "stated"},
                        {"text": "the dock tests live under src/features/dock/__tests__", "durable": "stated"},
                        {"text": "zz release builds pin rustflags", "durable": "guess"},
                        "zz plain fact about the build matrix"
                    ]}"#
                    .into(),
                )
            })
        }));
        d.distill(record("x")).await;
        let by_text = |needle: &str| {
            store
                .list_facts(10)
                .unwrap()
                .into_iter()
                .find(|f| f.text.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing"))
                .confidence
        };
        assert_eq!(by_text("cargo fmt"), 0.8);
        assert_eq!(by_text("__tests__"), 0.4);
        assert_eq!(by_text("rustflags"), 0.4);
        assert_eq!(by_text("build matrix"), 0.6);
    }

    #[test]
    fn layout_facts_are_the_ones_that_only_say_where() {
        assert!(is_layout_fact("Frontend state stores live in src/stores"));
        assert!(is_layout_fact("聊天组件位于 src/features/chat"));
        assert!(!is_layout_fact(
            "run `cargo test -p husk-host` before committing"
        ));
        // A where-sentence without a path is not about the tree.
        assert!(!is_layout_fact("the lock lives in memory"));
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
            reply: None,
        })
        .await;
        assert_eq!(store.recent_episodes(5).unwrap()[0].task, "fix the thing");
        // The correction is evidence for the model pass, never a fact itself.
        assert!(store.recall_facts("denied", 5).unwrap().is_empty());
    }

    /// A failed turn writes its episode and nothing else: it is not evidence
    /// of anything durable, so it must not buy a model call either.
    #[tokio::test]
    async fn a_failed_turn_writes_only_the_episode() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap());
        let d = Arc::new(
            TurnDistiller::new(store.clone()).with_summarizer(Arc::new(|_p| {
                Box::pin(async move {
                    Some(
                        r#"{"task": "model said", "facts": ["a durable fact"],
                           "persona": {"language": "zh"}}"#
                            .into(),
                    )
                })
            })),
        );
        d.distill(TurnRecord {
            outcome: "failed".into(),
            ..record("deploy the thing")
        })
        .await;
        assert_eq!(
            store.recent_episodes(5).unwrap()[0].task,
            "deploy the thing"
        );
        assert!(store.list_facts(10).unwrap().is_empty());
        assert!(store.all_persona().unwrap().is_empty());
    }

    /// The prompt carries the turn's answer and names what is *not* a fact —
    /// without either, "user asked about a bug" and "the project's convention
    /// is X" reach the model looking identical.
    #[test]
    fn the_distill_prompt_carries_the_reply_and_the_exclusions() {
        let prompt = distill_prompt(
            &TurnRecord {
                reply: Some("改了 memory 的召回".into()),
                ..record("看下记忆系统")
            },
            &[],
        );
        assert!(prompt.contains("reply: 改了 memory 的召回"), "{prompt}");
        assert!(prompt.contains("NOT a bug report"), "{prompt}");
        assert!(!distill_prompt(&record("x"), &[]).contains("reply:"));
        assert!(!prompt.contains("existing persona keys"), "{prompt}");
    }

    /// "Reuse an existing key" needs the keys in front of the model.
    #[tokio::test]
    async fn the_model_pass_sees_the_persona_keys_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap());
        store
            .set_persona("communication_language", "Chinese")
            .unwrap();
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let sink = seen.clone();
        let d = TurnDistiller::new(store).with_summarizer(Arc::new(move |p| {
            *sink.lock().unwrap() = p;
            Box::pin(async { None })
        }));
        d.distill(record("改一下")).await;
        let prompt = seen.lock().unwrap().clone();
        assert!(
            prompt.contains("existing persona keys: communication_language"),
            "{prompt}"
        );
    }

    /// A fenced ```json reply still parses.
    #[test]
    fn parse_distill_tolerates_fences_and_prose() {
        let fenced = "```json\n{\"task\": \"x\", \"facts\": [], \"persona\": {}}\n```";
        assert_eq!(parse_distill(fenced).unwrap().task.as_deref(), Some("x"));
        let prose = "sure! {\"task\": \"y\", \"facts\": [\"f\"], \"persona\": {}} done";
        assert_eq!(parse_distill(prose).unwrap().facts[0].parts().0, "f");
        assert!(parse_distill("no braces").is_none());
    }

    /// Model-chosen persona keys are normalized to safe labels.
    #[test]
    fn persona_key_strips_unsafe_chars() {
        assert_eq!(persona_key(" Prefers Chinese "), "Prefers_Chinese");
        assert_eq!(persona_key("a".repeat(60).as_str()), "a".repeat(40));
        assert_eq!(persona_key("not:allowed chars!"), "not:allowed_chars_");
    }

    /// Real credential shapes must be refused — the old name-only denylist let
    /// every one of these through.
    #[test]
    fn looks_secret_catches_credential_values() {
        for v in [
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-1234-5678-abcdefghijkl",
            "password: hunter2",
            "my token = xyz123",
            "api_key=abc123def456",
        ] {
            assert!(looks_secret(v), "should refuse {v:?}");
        }
    }

    /// Ordinary preferences and project facts must still be kept — a filter
    /// that refuses everything would silently empty the memory.
    #[test]
    fn looks_secret_keeps_ordinary_preferences() {
        for v in [
            "使用中文回答",
            "测试命令是 cargo test --workspace",
            "先跑 fmt 再 test",
            "project uses pnpm instead of npm",
            "commit messages follow conventional commits",
        ] {
            assert!(!looks_secret(v), "should keep {v:?}");
        }
    }

    /// Multi-byte text around a keyword must not panic on a byte-offset slice.
    #[test]
    fn looks_secret_is_safe_on_multibyte_text() {
        assert!(!looks_secret(
            "请记住：我的密码管理器是 1Password，不要存明文"
        ));
        assert!(looks_secret("密码是 password: hunter22 请记住"));
        assert!(!looks_secret("İİİ token 使用说明"));
    }
}
