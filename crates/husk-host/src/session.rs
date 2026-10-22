//! `SessionActor` — the orchestrator actor that owns history, state, and the
//! engine, communicating with the UI over `tokio::mpsc`.
//!
//! Owns: conversation history, `AgentState`, tool dispatch (via Engine),
//! and — in later stages — permission manager, compaction, hunk tracker.
//! This is the single writer of `history`: tools never mutate it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::checkpoint::TurnCheckpoint;
use crate::command::UiCommand;
use agent_capabilities::{SandboxCtx, WorkspaceCtx, WorkspacePaths};
use agent_coding::tracker::WriteTracker;
use agent_context::{
    git_snapshot, memory::TurnRecord, MemoryStore, TrackingMode, TurnDistiller, WorkspaceScanner,
};
use agent_ipc::{AgentState, UiEvent};
use agent_llm::types::ChatMessage;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::commands::{CommandCtx, CommandRegistry, CommandResult, ControlOp};
use agent_kernel::channels::ui_channels;
use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::hooks::HookChain;
use agent_kernel::tools::registry::ToolClass;
use agent_kernel::tools::{ToolCtx, ToolRegistry};

/// The rendered system prompt template — embedded at build time from the
/// skill's canonical source (single source of truth, never hand-edited here).
pub const SYSTEM_PROMPT_TEMPLATE: &str = include_str!("../assets/kernel-system-prompt.md");

/// Opening tag of the hidden per-message recall row — facts the memory store
/// matched to one user message, sent just before it instead of inside the
/// system prompt (see `run_prompt`).
const MEMORY_CONTEXT_OPEN: &str =
    "<memory-context>\nRecalled for the next message — guidance only; live workspace files take precedence.";
/// Char cap on the turn's final answer handed to the distiller — enough to
/// tell a project convention from a bug report, small enough to ride a
/// one-shot call.
const REPLY_CAP: usize = 1200;

/// Product event carrying the persisted run/task counts after each commit.
pub(crate) const SESSION_COUNTS: &str = "husk:session_counts";

/// Product event carrying the session's background jobs — the whole list, so
/// the view replaces rather than merges.
pub const BG_JOBS: &str = "husk:bg_jobs";

/// How long the bg feed waits for more changes before it sends one snapshot:
/// `remove` on a batch of finished jobs or a burst of exits is one update.
const BG_FEED_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// Notice turns an idle session may open on its own before it waits for the
/// user. Past the cap the notices stay queued and ride the next prompt.
pub(crate) const MAX_WAKES_IN_ROW: u32 = 3;

/// Entries a session read back from its job record — jobs an earlier process
/// ran. Shared between the actor (its feed writes them back with every
/// snapshot) and the manager (a removal prunes them).
pub type BgRestored = Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

/// Saved entries kept per session. The record grows by every job the session
/// ever ran; the pane only needs the recent ones.
const BG_RECORD_CAP: usize = 50;

/// The list the 后台 view shows and the record keeps: what earlier processes
/// ran, then this process's live jobs. Without the first half a reopened
/// session shows only what ran since the restart, and its first new job
/// overwrites the record with that.
pub fn bg_snapshot(
    restored: &BgRestored,
    reg: &agent_tools_shell::bg::BgRegistry,
) -> serde_json::Value {
    let mut jobs = restored.lock().map(|r| r.clone()).unwrap_or_default();
    if let serde_json::Value::Array(live) = serde_json::json!(reg.jobs()) {
        jobs.extend(live);
    }
    serde_json::Value::Array(jobs)
}

/// Log lines a saved job keeps. Its log file goes with the process that wrote
/// it, so this tail is all a later process can show.
const BG_RECORD_TAIL_LINES: usize = 40;

/// Byte cap on one saved tail — the record is rewritten on every change.
const BG_RECORD_TAIL_BYTES: usize = 4 * 1024;

/// [`bg_snapshot`] as the record keeps it: every live job also carries the
/// tail of its log (`log_tail`). Written on every change, so a finished job's
/// tail is its final output, and one cut off by the app closing keeps what it
/// had printed by its last change.
pub fn bg_record(
    restored: &BgRestored,
    reg: &agent_tools_shell::bg::BgRegistry,
) -> serde_json::Value {
    let mut jobs = bg_snapshot(restored, reg);
    for job in jobs.as_array_mut().into_iter().flatten() {
        if job.get("restored").is_some() {
            continue;
        }
        let Some(id) = job.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let Ok(tail) = reg.tail(&id, BG_RECORD_TAIL_LINES) else {
            continue;
        };
        let tail = if tail.len() > BG_RECORD_TAIL_BYTES {
            let mut cut = tail.len() - BG_RECORD_TAIL_BYTES;
            while !tail.is_char_boundary(cut) {
                cut += 1;
            }
            tail[cut..].to_string()
        } else {
            tail
        };
        if let Some(fields) = job.as_object_mut() {
            fields.insert("log_tail".into(), serde_json::Value::String(tail));
        }
    }
    jobs
}

/// A saved job list as it reads in a later process. Nothing in it is still
/// running — the process that ran it is gone — so a running or paused entry
/// becomes interrupted, while one that had ended keeps its own ending. Every
/// entry is marked `restored`: no process is behind it, nothing can be paused,
/// killed or tailed.
///
/// The ids get the start stamp appended: the new process numbers its jobs
/// from `bg-1` again, and two rows sharing an id would remove each other. An
/// entry restored before keeps the id it already has, so a record survives
/// any number of restarts unchanged.
pub fn restored_bg_jobs(saved: serde_json::Value) -> Vec<serde_json::Value> {
    let serde_json::Value::Array(mut list) = saved else {
        return Vec::new();
    };
    for job in list.iter_mut() {
        let Some(fields) = job.as_object_mut() else {
            continue;
        };
        if fields.get("restored") != Some(&serde_json::Value::Bool(true)) {
            let stamp = fields
                .get("started_at_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if let Some(serde_json::Value::String(id)) = fields.get_mut("id") {
                *id = format!("{id}@{stamp}");
            }
            fields.insert("restored".into(), serde_json::Value::Bool(true));
        }
        let kind = fields
            .get("state")
            .and_then(|s| s.get("kind"))
            .and_then(|k| k.as_str())
            .unwrap_or_default();
        if kind == "running" || kind == "paused" {
            fields.insert(
                "state".into(),
                serde_json::json!({
                    "kind": "done",
                    "label": "应用已退出",
                    "code": null,
                    "reason": "error",
                }),
            );
        }
    }
    let skip = list.len().saturating_sub(BG_RECORD_CAP);
    list.drain(..skip);
    list
}

/// Real execution counts, not user-turn counts: one prompt can run the
/// model/tool loop many times. One assistant row is one **step** — one model
/// request plus the tool calls it triggered — and one tool row is one **call**
/// inside a step, of which a step usually has several. A **turn** is one
/// accepted prompt drained to completion. Counts the CURRENT history, so
/// `/clear`, compaction and retry move them down with it.
///
/// This is the AUTHORITATIVE figure — the frontend's live tick is an upper
/// bound (see the `Usage` arm in `apply-event.ts`), and every commit re-bases
/// the header on this. Rows are the only definition the store can rebuild from
/// a snapshot, so this is the one the persisted number has to mean.
pub(crate) fn history_counts(history: &[ChatMessage]) -> (u32, u32) {
    let mut runs = 0u32;
    let mut tasks = 0u32;
    for m in history {
        match m.role {
            agent_llm::types::Role::Assistant => runs += 1,
            agent_llm::types::Role::Tool => tasks += 1,
            _ => {}
        }
    }
    (runs, tasks)
}

/// A round's usage as a persistable meter reading, or `None` when the wire
/// reported nothing real (missing, or all-zero on a degraded/failed round).
pub(crate) fn measured_usage(
    usage: Option<(u32, u32, u32)>,
    cache_write: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (prompt, completion, cached) = usage?;
    let real = prompt > 0 || completion > 0 || cached > 0 || cache_write > 0;
    real.then_some((prompt, completion, cached, cache_write))
}

/// A cache reading belongs to the model that produced it: another model has
/// another cache, so a reading carried across a switch would mis-attribute the
/// old hit rate and write count to the new model — in the header until the next
/// turn, and permanently in the stats pane. The prompt size survives: it
/// describes the history, not the cache.
fn cache_reading_after_switch(
    usage: Option<(u32, u32, u32, u32)>,
    model_changed: bool,
) -> Option<(u32, u32, u32, u32)> {
    match usage {
        Some((prompt, completion, _, _)) if model_changed => Some((prompt, completion, 0, 0)),
        other => other,
    }
}

/// What the distiller's model call runs on. Kept in step with the session's
/// model, so a `/model` switch moves distillation to the model in use.
struct DistillCtx {
    provider: std::sync::Arc<dyn agent_llm::LlmProvider>,
    model: String,
    params: agent_llm::ModelParams,
}

/// This session's distiller, or `None` when memory is off. One constructor for
/// the three places that (re)build it — spawn, the memory switch and a model
/// switch — so they cannot drift apart.
fn build_distiller(
    memory: Option<&std::sync::Arc<MemoryStore>>,
    distill: bool,
    ctx: &DistillCtx,
    record: Option<DistillRecorder>,
) -> Option<std::sync::Arc<TurnDistiller>> {
    let store = memory?.clone();
    let mut d = TurnDistiller::new(store);
    if distill {
        d = d.with_summarizer(distill_summarizer(
            ctx.provider.clone(),
            ctx.model.clone(),
            ctx.params.clone(),
            record,
        ));
    }
    Some(std::sync::Arc::new(d))
}

/// Replace the region between `<!-- name -->` and `<!-- /name -->` in the
/// rendered system prompt. The template ships the markers, so a per-turn
/// refresh can swap just its own block instead of rebuilding the whole prompt.
fn replace_block(content: &str, name: &str, block: &str) -> String {
    let open = format!("<!-- {name} -->");
    let close = format!("<!-- /{name} -->");
    let (Some(start), Some(end)) = (content.find(&open), content.rfind(&close)) else {
        return content.to_string();
    };
    let head = &content[..start + open.len()];
    let tail = &content[end..];
    format!("{head}\n{block}\n{tail}")
}

/// The `<!-- linked -->` region — the directories outside the workspace this
/// session may reach. Each carries its access and the one rule the model has
/// to keep: a relative path still means the workspace, so a linked file is
/// named by absolute path. Empty means none, and the region then says nothing
/// — a session with no grants should not spend prompt on the concept.
pub(crate) fn linked_roots_block(roots: &[agent_capabilities::LinkedRoot]) -> String {
    if roots.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "### Linked workspaces\n\n\
         These directories are granted to this session. The file tools and `bash`\n\
         reach them exactly as they reach the workspace; treat their contents as\n\
         in scope.\n\n",
    );
    for r in roots {
        let access = match r.access {
            agent_capabilities::RootAccess::ReadWrite => "read-write",
            agent_capabilities::RootAccess::Read => "read-only",
        };
        out.push_str(&format!("- {}  ({access})\n", r.path.display()));
    }
    out.push_str(
        "\nRelative paths always resolve against the workspace root, so name a\n\
         linked file by its absolute path. `@-mentions` reach a linked file the same\n\
         way — absolute only. A path outside every link comes back as a refusal; the\n\
         user is offered the grant for it on the approval card, so do not retry the\n\
         same call unchanged — if they declined, work inside what you reach.\n",
    );
    out
}

/// The `<!-- permission -->` region — one bullet naming the gate's mode. The
/// live gate is what approves calls, so the prompt's copy is rewritten from it
/// on every switch rather than left at the value the session was spawned with.
fn permission_block(mode: &str) -> String {
    format!(
        "- Permission mode: {mode}  (default | acceptEdits | auto | dontAsk | bypassPermissions)"
    )
}

/// Swap the recalled-memory region of a system prompt.
///
/// New prompts carry `<!-- memory -->` / `<!-- /memory -->` markers, so the
/// region is replaced verbatim at spawn and on resume. It is deliberately not
/// touched per turn: the system prompt heads the provider's prompt cache.
/// Prompts persisted before
/// the markers existed fall back to the old literal swap, which only fires
/// when the store-failure string is present, i.e. the pre-marker behaviour.
fn set_memory_block(content: &str, block: &str) -> String {
    if content.contains("<!-- memory -->") {
        return replace_block(content, "memory", block);
    }
    content.replace("(no memory)", block)
}

/// The hidden recall row `run_prompt` sends ahead of a user message.
pub(crate) fn is_memory_context(m: &ChatMessage) -> bool {
    m.role == agent_llm::Role::User
        && m.notice == Some(agent_llm::types::NoticeKind::Hidden)
        && m.content
            .as_deref()
            .is_some_and(|c| c.starts_with(MEMORY_CONTEXT_OPEN))
}

/// Drop all but the last `keep` recall rows, with their bookkeeping. A row
/// exists for the message it precedes: once that turn is history, the facts it
/// carried are in the transcript anyway and the session never re-sends one.
/// Compaction rewrites the history wholesale, so that is where deleting them
/// costs no prompt cache.
fn prune_recall_rows_in(
    history: &mut Vec<ChatMessage>,
    facts: &mut Vec<(usize, u64)>,
    keep: usize,
) {
    let rows: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| is_memory_context(m))
        .map(|(i, _)| i)
        .collect();
    if rows.len() <= keep {
        return;
    }
    let doomed: Vec<usize> = rows[..rows.len() - keep].to_vec();
    let doomed_set: std::collections::HashSet<usize> = doomed.iter().copied().collect();
    // The bookkeeping is keyed by the prompt row that follows each recall row:
    // a dropped row takes its facts with it, and every later index shifts down
    // by the number removed before it.
    facts.retain(|(at, _)| !doomed_set.contains(&at.saturating_sub(1)));
    for (at, _) in facts.iter_mut() {
        *at -= doomed.iter().filter(|drop| **drop < *at).count();
    }
    // Ascending, so dropping from the back never shifts a pending index.
    for idx in doomed.iter().rev() {
        history.remove(*idx);
    }
}

/// Join a turn's injected steers into the single correction string the
/// distiller turns into a `user steered: …` fact — `None` when the turn ran
/// without steering.
fn steered_with(steers: &[String]) -> Option<String> {
    let kept: Vec<&str> = steers
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(kept.join("; "))
}

/// Join the turn's denied tool names into the distiller's correction string —
/// an approval-card deny is a correction ("don't run that"), so it seeds the
/// same `user corrected: …` fact a typed steer would.
fn denied_tools(denials: &[String]) -> Option<String> {
    if denials.is_empty() {
        return None;
    }
    Some(format!("denied tool(s): {}", denials.join(", ")))
}

/// Receives one distillation call's `(prompt, completion)` tokens — the host
/// writes them to the session's store so the spend is visible in 统计.
type DistillRecorder = std::sync::Arc<dyn Fn(u32, u32) + Send + Sync>;

/// The kernel's end of the distill contract — a one-shot completion on the
/// session's side provider (tools off, temperature 0). `Sampler::sample`
/// carries the retry policy; an `Err` or an empty body degrades to `None`
/// so the distiller falls back to its deterministic path. Rebuilt through
/// `build_distiller` on a `/model` switch, so it runs on the model in use;
/// `record` gets each call's token spend.
fn distill_summarizer(
    provider: std::sync::Arc<dyn agent_llm::LlmProvider>,
    model: String,
    params: agent_llm::ModelParams,
    record: Option<DistillRecorder>,
) -> agent_context::memory::Summarizer {
    std::sync::Arc::new(move |prompt: String| {
        let provider = provider.clone();
        let model = model.clone();
        let params = params.clone();
        let record = record.clone();
        Box::pin(async move {
            let sampler = agent_llm::sampler::Sampler::new(provider);
            let mut out = String::new();
            let req = agent_llm::sampler::SampleRequest {
                model: &model,
                temperature: 0.0,
                tools: None,
                reasoning_effort: None,
                params: &params,
            };
            sampler
                .sample(
                    req,
                    &[agent_llm::types::ChatMessage::user(prompt)],
                    |chunk| match chunk {
                        agent_llm::types::StreamChunk::ContentDelta(t) => out.push_str(t.as_str()),
                        // The side channel's numbers: what this call cost.
                        agent_llm::types::StreamChunk::Done {
                            prompt_tokens,
                            completion_tokens,
                            ..
                        } => {
                            if let Some(record) = &record {
                                record(prompt_tokens.unwrap_or(0), completion_tokens.unwrap_or(0));
                            }
                        }
                        _ => {}
                    },
                    |_| {},
                    || {},
                )
                .await
                .ok()?;
            (!out.trim().is_empty()).then_some(out)
        })
    })
}

/// Append one instructions file to the rendered prompt as a `## User
/// instructions` section — empty/missing files are no-ops.
fn append_instructions(out: &mut String, path: &std::path::Path, label: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push_str(&format!(
        "\n\n## User instructions — {label} ({})\n\n{text}",
        path.display()
    ));
}

/// Everything the prompt appends from disk: the AGENTS.md sections, then the
/// standing `## Rules`. One function because spawn and resume must render the
/// same tail — a resumed conversation that re-read only half of it would keep
/// the other half frozen at the build that wrote it.
fn append_prompt_tail(out: &mut String, root: &Path, paths: &agent_paths::AppPaths) {
    append_instructions(out, &paths.config_file("AGENTS.md"), "user");
    append_instructions(out, &root.join("AGENTS.md"), "project");
    out.push_str(&crate::rules::prompt_block(&crate::rules::discover_rules(
        root, paths,
    )));
}

/// Where the re-rendered tail starts: the first of the two markers. The rules
/// marker stands on its own because a workspace with rules but no AGENTS.md
/// has no instructions section to find.
fn prompt_tail_start(prompt: &str) -> Option<usize> {
    ["\n\n## User instructions — ", crate::rules::PROMPT_MARKER]
        .iter()
        .filter_map(|marker| prompt.find(marker))
        .min()
}

/// The children's route for one parent: `base` widened to every configured
/// provider, with the subagent default applied (see `child_route`).
fn routed_child(
    base: &agent_kernel::tools::delegate::ChildRoute,
    config_dir: &Path,
    parent_name: &str,
    default: Option<&(String, String)>,
    thinking: Option<&str>,
    conversation: &str,
) -> agent_kernel::tools::delegate::ChildRoute {
    let cfg = agent_llm::AppConfig::load(config_dir, None).unwrap_or_default();
    let mut route = base.clone();
    crate::child_route::install(
        &mut route,
        &cfg,
        parent_name,
        base.provider.clone(),
        default,
        thinking,
        conversation,
    );
    route
}

/// Open the workspace-keyed memory store — `memory.db` under the app's
/// data dir. Test builds redirect to a per-process temp file so spawned
/// sessions don't write into the user's real db (nor contend on its
/// cross-process redb lock while the app is running). The same `deps/`
/// marker the session store uses covers `tests/` binaries too —
/// `#[cfg(test)]` alone only reaches unit tests inside this lib.
fn open_memory_store(workspace_root: &Path) -> Option<MemoryStore> {
    let db_dir = if agent_store::is_test_binary() {
        Some(std::env::temp_dir().join(format!("husk-test-{}", std::process::id())))
    } else {
        agent_store::app_data_dir()
    };
    db_dir.and_then(|d| {
        let _ = std::fs::create_dir_all(&d);
        MemoryStore::open(&d.join("memory.db"), workspace_root).ok()
    })
}

/// SessionActor configuration for one workspace.
pub struct SessionConfig {
    /// Where the host keeps its config and workspace-local state. Supplied by
    /// the product; the kernel never guesses a name.
    pub paths: agent_paths::AppPaths,
    pub workspace_root: PathBuf,
    /// Live plugin router — shared with the manager so a reload swaps what the
    /// engine advertises without rebuilding the session.
    pub plugins: Option<crate::plugins::RuntimePluginHandle>,
    pub provider: Arc<dyn agent_llm::LlmProvider>,
    /// Config-level provider key (`deepseek`, `anthropic`, …). The adapter's own
    /// `id()` names the wire protocol, not the deployment, so stats group by
    /// this instead.
    pub provider_name: String,
    pub model: String,
    pub temperature: f32,
    /// Permission mode label substituted into the prompt.
    pub permission_mode: String,
    /// Agent mode (`build` | `plan` | `goal`) — governs the tool registry
    /// and the turn contract; substituted into the prompt.
    pub agent_mode: String,
    /// Whether to seed the tracker with git-dirty files (`AllDirty` mode).
    pub track_dirty: bool,
    /// Initial thinking / reasoning effort level.
    pub thinking_level: Option<String>,
    /// Model-specific thinking level mapping.
    pub thinking_level_map: Option<std::collections::HashMap<String, Option<String>>>,
    /// Model's context window from config — `None` falls back to the
    /// engine's 256_000 default (compaction + usage % share the bound).
    pub context_window: Option<u64>,
    /// Declared input modalities (`ModelConfig.input`, e.g.
    /// `["text", "image"]`) — `"image"` gates real image parts on the wire.
    pub model_input: Vec<String>,
    /// Compaction trigger fraction — `None` → the engine's `COMPACT_AT`.
    pub compact_at: Option<f32>,
    /// Per-model wire settings from config (`maxTokens`, `samplingParams`,
    /// model-level `compat`). `None` → every adapter default applies.
    pub model_params: Option<agent_llm::ModelParams>,
    /// Parked follow-up prompts restored from `SessionMeta` — the
    /// composer's queue survives session switches and app restarts.
    pub queued_prompts: Vec<String>,
    /// Memory subsystem master switch — `false` opens no `MemoryStore`:
    /// no `<memory>` block refresh and no episode/fact writes.
    pub memory_enabled: bool,
    /// Model distillation switch — `false` keeps the deterministic
    /// distiller but drops the per-turn summarizer model call.
    pub memory_distill: bool,
}

/// The host-added tools every full registry carries on top of the kernel's
/// builtins. The child registry draws from the same list and then excludes
/// capabilities that only the parent may hold.
fn host_tools() -> Vec<agent_kernel::tools::registry::ToolSpec> {
    vec![
        agent_kernel::tools::batch::spec_with(batch_profile()),
        // The file tools and the shell are described against each other —
        // observe the workspace with the former, run commands with the latter.
        agent_coding::observe::read(agent_tools_fs::fs_read::spec()),
        // The patch tools cannot write `.blueprint/`: a node file has one
        // writer, `blueprint_edit`, which validates the fields first.
        agent_coding::blueprint::guard_patch(agent_tools_fs::fs_patch::spec()),
        agent_coding::blueprint::guard_patch(agent_tools_fs::apply_patch::spec()),
        agent_coding::observe::list(agent_tools_fs::list_dir::spec()),
        agent_coding::observe::grep(agent_tools_fs::grep::spec()),
        agent_tools_shell::test_runner::spec(),
        agent_coding::observe::bash(agent_tools_shell::bash::spec()),
        agent_tools_shell::services::spec(),
        agent_tools_shell::bg::spec(),
        agent_tools_web::web_fetch::spec(),
        agent_tools_web::web_fetch::spec_alias(),
        agent_tools_computer::screenshot::spec(),
        agent_tools_computer::computer::spec(),
        agent_coding::blueprint_tool::spec(),
        agent_coding::blueprint::edit::spec(),
        agent_coding::skill::spec(),
        // Read-only recall: the prompt carries persona, facts arrive per
        // message, episodes not at all — so "what did we decide about X" needs
        // a way to ask. Safe for a delegated child to hold.
        agent_coding::remember::search_spec(),
        // Serena's own file ops write too, and its target sits inside
        // `arguments` — the guard reads through the dispatcher.
        agent_coding::blueprint::guard_patch(agent_tools_serena::spec()),
    ]
}

/// `batch_execute` described with Husk's own observation tools, plus the
/// argument signatures a nameless batch item is recognised by.
fn batch_profile() -> agent_kernel::tools::batch::BatchProfile {
    const DESCRIPTION: &str = "The DEFAULT way to issue 2+ INDEPENDENT observation calls \
         (smart_read, list_dir, smart_grep, web_fetch…) — pack them into \
         ONE round trip instead of firing read-only tools back to back. \
         Args shape is always an object with a `calls` array, and every \
         item MUST carry a `tool` string: \
         {\"calls\":[{\"tool\":\"smart_read\",\"args\":{\"path\":\"src/lib.rs\"}},\
         {\"tool\":\"smart_grep\",\"args\":{\"pattern\":\"TODO\"}}]}. \
         Known-target rule: if the targets are already known (e.g. three \
         file paths, two ranges of one file, a dir listing plus a grep), \
         batch them now — never \
         read A → reason → read B when A and B were both known up front. \
         Only go sequential when the next call's arguments depend on this \
         call's result. Calls run at limited concurrency (max 4, max 4 \
         network calls); results return in the order you sent them, and \
         one call's failure doesn't fail the batch (check each \
         `── [i] ──` section). Forbidden by design: mutations \
         (write/patch/shell/test), delegate, todo, ask_question, goal \
         signals, and nested batch_execute.";
    agent_kernel::tools::batch::BatchProfile {
        description: Some(DESCRIPTION),
        example_call: Some(r#"{"tool":"smart_read","args":{"path":"src/lib.rs"}}"#),
        infer: vec![
            (&["url"], "web_fetch"),
            (&["pattern", "regex", "max_hits"], "smart_grep"),
            (&["command"], "bash"),
        ],
    }
}

/// Tools only the parent holds. `remember` writes the persona every later
/// parent turn reads; a delegated child works on untrusted content and must
/// not write that, so it is registered here and never in the child base.
fn parent_only_tools() -> Vec<agent_kernel::tools::registry::ToolSpec> {
    vec![agent_coding::remember::spec()]
}

/// The parent's base registry and the base a delegated child draws from
/// (no `delegate`, `remember`, `computer` or human-interaction tools).
pub fn base_registries() -> (ToolRegistry, ToolRegistry) {
    let mut full = ToolRegistry::with_builtins();
    for spec in host_tools() {
        full.register(spec);
    }
    let child = full.filtered(|s| {
        !agent_kernel::tools::delegate::PARENT_ONLY_TOOLS.contains(&s.name)
            && s.name != "computer"
            && s.class != ToolClass::HumanInteraction
    });
    for spec in parent_only_tools() {
        full.register(spec);
    }
    (full, child)
}

/// What opened the turn being run — the only difference between a user prompt
/// and a background wake-up once the shared bookkeeping starts.
enum TurnInput {
    /// The user's message, echoed to the UI and appended to history by the
    /// engine.
    Prompt(String),
    /// Host notices the actor drained from the engine's notice channel — a
    /// finished background job. The engine opens the turn on them, and no
    /// user message is invented.
    Notices(Vec<String>),
}

/// One live session: owns history + engine, consumes commands, emits events.
pub struct SessionActor {
    history: Vec<ChatMessage>,
    engine: Engine,
    /// The coding policy's `.blueprint/` switch — the host owns the flip, the
    /// kernel engine knows nothing of it.
    architecture: agent_coding::policy::ArchitectureSwitch,
    /// Model + provider key this session runs — stamped into the store's meta
    /// with every usage write so the stats pane can group spend by model.
    model: String,
    provider_name: String,
    state: AgentState,
    io: EngineIo,
    cmd_tx: mpsc::Sender<UiCommand>,
    /// Write tracking for undo/rewind + files-changed list.
    hunks: WriteTracker,
    /// What the root context has observed. Cleared with the history rows that
    /// held the results, so "you saw this already" is never said about a
    /// result the model no longer has.
    observations: Arc<agent_coding::observe::ObservationLedger>,
    /// Background notices into the model — a linked workspace granted mid-turn
    /// is announced on the next turn instead of waiting for a restart.
    notices: mpsc::Sender<String>,
    /// The roots this session may reach beyond its workspace — shared with
    /// the tool context, so a grant applies to the next call and to every
    /// delegated child without rebuilding anything.
    linked_roots: Arc<agent_capabilities::LinkedRoots>,
    /// Engine's decision slot — `ToolDecision` writes here mid-turn.
    decision_slot: Arc<std::sync::Mutex<Option<(u64, agent_kernel::tools::registry::ToolVerdict)>>>,
    /// `AnswerQuestion` resolves here mid-turn (same bypass pattern).
    ask_channel: Arc<agent_kernel::tools::registry::AskChannel>,
    /// Engine's permissions slot — permission mode updates apply immediately mid-turn.
    permissions_slot: Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>>,
    /// Child-engine configuration shared with `delegate` — rewritten on
    /// hot-swap so a live session's children track its current model.
    child_route: Arc<std::sync::RwLock<agent_kernel::tools::delegate::ChildRoute>>,
    /// The parent's own model settings, before `child_route` widens them —
    /// kept so clearing the subagent default restores the parent's model.
    child_base: agent_kernel::tools::delegate::ChildRoute,
    /// Global subagent default `(provider, model)`; `None` = children follow
    /// the parent's model.
    subagent_model: Option<(String, String)>,
    /// Global subagent thinking level; `None` = children follow this session's
    /// level. A manifest `thinking:` still wins over it.
    subagent_thinking: Option<String>,
    /// Engine's thinking-level slot — read by the manager to report the
    /// session's live level (defaults may differ once prefs change).
    thinking_slot: Arc<std::sync::RwLock<Option<String>>>,
    /// The session's live `(provider, model)` — the IPC layer writes it the
    /// moment a `SetModel` arrives, ahead of the pump (which waits behind a
    /// running turn); the actor keeps it in step on every switch it applies.
    model_slot: Arc<std::sync::RwLock<(String, String)>>,
    /// Forwards `UiCommand::Steer` texts into the engine's steer channel. The
    /// engine drains it between rounds; whatever it leaves behind when a turn
    /// ends is read back by the actor itself (`drain_queue` / `run`) and run
    /// as the next prompt.
    steer_tx: mpsc::Sender<String>,
    /// Cooperative cancel flag shared with the engine — `UiCommand::Cancel`
    /// and `cancel_writer()` set it; the engine polls it between chunks and
    /// before each tool dispatch.
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Actor-owned command receiver — `run()` drains it.
    cmd_rx: mpsc::Receiver<UiCommand>,
    shutdown: Arc<tokio::sync::Notify>,
    /// Workspace root (undo writes resolve against it).
    workspace_root: PathBuf,
    /// The host's app directories — config-relative and workspace-local state
    /// paths derive from these instead of a hardcoded product name.
    paths: agent_paths::AppPaths,
    /// Ordered hook chain (lifecycle interception).
    hooks: HookChain,
    /// Per-session loop guard, layered into every chain this actor installs —
    /// it is not a plugin, so a `reload_plugins` must not remove it.
    repeat_guard: Arc<crate::repeat_guard::RepeatGuard>,
    /// Live plugin router — the SAME `Arc` the engine reads per request,
    /// so a `reload_plugins` swap reaches `/x` dispatch too. `None` = no
    /// plugin manager installed for this workspace.
    plugins: Option<crate::plugins::RuntimePluginHandle>,
    /// Memory store + post-turn distiller (background task). `memory` is the
    /// actor's own view; `memory_handle` is the same slot the engine's tools
    /// read, so one switch covers recall, distillation and the `remember` tool.
    memory: Option<Arc<MemoryStore>>,
    memory_handle: agent_coding::remember::MemoryHandle,
    distiller: Option<Arc<TurnDistiller>>,
    /// What the distillation pass runs on — rebuilt with the session's model
    /// on every switch, so the memory pass uses the model in use.
    distill_ctx: DistillCtx,
    /// Whether the semantic distill pass is on (`SetMemory` flips it). The
    /// distiller is rebuilt from this on every model switch.
    memory_distill: bool,
    /// Where a distill call reports its tokens — one write per call into the
    /// session's store, so the spend is visible in 统计.
    distill_record: Option<DistillRecorder>,
    /// Recall rows this session already sent: `(history index of the prompt
    /// that follows it, fact id)`. A fact the model has already read is not
    /// worth re-sending, and a rewind drops the rows together with the index.
    injected_facts: Vec<(usize, u64)>,
    /// Persona entries this session is already carrying (system prompt plus
    /// recall rows). A mid-session `remember` reaches the model as the delta.
    injected_persona: std::collections::HashMap<String, String>,
    /// Stable session identity — sidebar ordering + persistence file name.
    session_id: i64,
    /// This session's conversation identity (`conversation_key`). Every
    /// provider the actor rebuilds advertises it, so a model switch or a
    /// fallback hop keeps the gateway's routing and prompt cache.
    conversation: String,
    /// Per-workspace session store, including recovery points for live turns.
    store: Option<Arc<agent_store::SessionStore>>,
    history_revision: u64,
    /// Unix seconds the running turn was submitted — its meta's `updated_at`
    /// until the turn ends. The sidebar orders sessions by that stamp; one
    /// taken per write would let concurrently running sessions trade places
    /// on every flush.
    turn_started_at: Option<u64>,
    checkpoint: Option<Arc<TurnCheckpoint>>,
    /// Per-session cache-write sink shared with the provider and the
    /// checkpoint relay (the shared `StreamChunk::Done` has no write slot).
    usage_observer: Option<Arc<agent_llm::UsageObserver>>,
    /// Turn-settle / history-rewrite signals for prompt-cache warming.
    warming: agent_llm::WarmingControl,
    /// Last completed turn's `(prompt, completion, cached, cache_write)`
    /// tokens — persisted into `SessionMeta` so a reopened session's header
    /// meter shows real numbers before the next `Usage` event.
    last_usage: Option<(u32, u32, u32, u32)>,
    /// Estimated composition of the request behind `last_usage`; cleared
    /// whenever `last_usage` stops describing a real request (compaction).
    last_breakdown: Option<agent_store::ContextBreakdown>,
    /// Parked follow-up prompts — the composer's queue. Lives here (not
    /// in the webview) so it survives session switches: the actor stays
    /// alive in `handles`/`parked`, drains at turn end no matter which
    /// session is on screen, and mirrors into `SessionMeta` for restarts.
    /// Shared with `SessionHandle` so `Enqueue`/`SetQueued` apply directly
    /// from IPC — the command pump blocks behind a running `run_turn`, and
    /// a pump-carried write would keep the queue panel hidden for the whole
    /// turn (the `QueuedPrompts` echo is what lights it up).
    queued: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    /// Skill backend — owns the cached prompt catalog and its change stamp, so
    /// a large tree is not re-read on every prompt.
    skills: agent_skills::SkillManager,
    /// This session's background jobs. `run()` mirrors every change to the UI
    /// as `husk:bg_jobs`; the manager queries and controls them directly.
    bg_jobs: Arc<agent_tools_shell::bg::BgRegistry>,
    /// What earlier processes of this session ran, read back from the record
    /// at spawn — the head of every snapshot the feed sends and saves.
    bg_restored: BgRestored,
    /// Poked by the background notifier — `run()` turns an idle session into
    /// a notice turn instead of waiting for the next prompt.
    wake_rx: mpsc::Receiver<()>,
    /// Notice turns run since the last user prompt. Capped at
    /// `MAX_WAKES_IN_ROW`: a woken model may start another job, and nothing
    /// else would stop that chain from billing an unattended session.
    wakes_in_row: u32,
}

/// The conversation identity every provider of this session advertises: the
/// install's routing salt, the persisted session id and the workspace. Stable
/// across a restart, a model switch and a fallback hop, which is what a gateway
/// needs to keep serving one conversation from the node that holds its prompt
/// cache. The provider hashes it before it reaches the wire, so neither the
/// salt nor the session id is ever sent.
///
/// Canonical, so `--workspace .`, an absolute path and a symlinked path name the
/// same conversation: the provider only sees the hash, and a different spelling
/// would quietly cold-start the gateway's cache.
pub fn conversation_key(session_id: i64, workspace_root: &Path) -> String {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    format!(
        "{}:{session_id}:{}",
        crate::prefs::routing_salt(),
        root.display()
    )
}

/// A provider instance without the session's usage observer. Background work
/// (distiller, child engines) must not overwrite the foreground counter.
fn unobserved_provider(
    cfg: &SessionConfig,
    conversation: &str,
) -> Option<Arc<dyn agent_llm::LlmProvider>> {
    let app = agent_llm::AppConfig::load(&cfg.paths.config_dir, None).ok()?;
    unobserved_provider_from(&app, &cfg.provider_name, conversation)
}

/// The same provider, built without the session's usage observer.
///
/// Every request the provider sends starts with `UsageObserver::reset` and
/// ends by publishing its cache-write count into that observer. The session's
/// counter is read asynchronously — the checkpoint relay annotates the `Usage`
/// event it forwards, and the actor reads it at turn end — so any *side*
/// request sharing the observer can overwrite the foreground round's figure.
/// Side traffic (the distiller, cache warming) therefore gets its own
/// instance.
pub(crate) fn unobserved_provider_from(
    app: &agent_llm::AppConfig,
    provider_name: &str,
    conversation: &str,
) -> Option<Arc<dyn agent_llm::LlmProvider>> {
    let provider = app.providers.get(provider_name)?;
    agent_llm::ProviderFactory::build_with_conversation(provider, None, Some(conversation)).ok()
}

/// Wrap the foreground session provider in prompt-cache warming when the
/// model declares a `promptCache` lifetime and prices to weigh refreshes by.
/// Anything else (unknown model, no lifetime, warming off) passes `provider`
/// through unchanged.
fn warmed_provider(
    provider: Arc<dyn agent_llm::LlmProvider>,
    replay: Arc<dyn agent_llm::LlmProvider>,
    model: &str,
    resolved: Option<&agent_llm::ModelConfig>,
    control: &agent_llm::WarmingControl,
) -> Arc<dyn agent_llm::LlmProvider> {
    let Some(d) = resolved else {
        return provider;
    };
    agent_llm::WarmingProvider::wrap(
        provider,
        replay,
        model,
        d.prompt_cache.as_ref(),
        d.cost.as_ref(),
        control,
    )
}

/// The session model's resolved config (overrides folded in), when the
/// provider config describes it.
fn resolved_model(cfg: &SessionConfig) -> Option<agent_llm::ModelConfig> {
    let app = agent_llm::AppConfig::load(&cfg.paths.config_dir, None).ok()?;
    let pcfg = app.providers.get(&cfg.provider_name)?;
    pcfg.find_model(&cfg.model)?;
    Some(pcfg.model_opts(&cfg.model))
}

impl SessionActor {
    /// Boot a session: scan workspace + git snapshot → render system prompt →
    /// return the actor plus the UI channel endpoints.
    pub fn spawn(
        cfg: SessionConfig,
    ) -> (
        Self,
        agent_kernel::channels::UiChannels<crate::command::UiCommand>,
    ) {
        Self::spawn_inner(cfg, None, 0)
    }

    /// Fresh session with a caller-assigned id (the SessionManager allocates
    /// ids via `SessionStore::next_id` so persistence + sidebar agree).
    pub fn spawn_with_id(
        cfg: SessionConfig,
        session_id: i64,
    ) -> (
        Self,
        agent_kernel::channels::UiChannels<crate::command::UiCommand>,
    ) {
        Self::spawn_inner(cfg, None, session_id)
    }

    /// Resume a persisted session — `history` is the prior `Vec<ChatMessage>`
    /// snapshot; the LLM context is exactly what the session left behind.
    /// `session_id` reuses the persisted id so the store/app keep tracking it.
    pub fn resume(
        cfg: SessionConfig,
        session_id: i64,
        history: Vec<ChatMessage>,
    ) -> (
        Self,
        agent_kernel::channels::UiChannels<crate::command::UiCommand>,
    ) {
        // Sanitize persisted history (a snapshot poisoned by an old bug
        // replays into every later request and crashes devin's upstream as
        // `invalid_argument`):
        //  1. strip `[call: …]` text-protocol echoes from assistant content;
        //  2. drop assistant rows that are empty AND carry no tool_calls —
        //     a blank turn replays as a meaningless message;
        //  3. drop orphan tool rows — a `tool` message whose `tool_call_id`
        //     never appeared on an assistant's `tool_calls` replays as a
        //     `function_call_output` with no matching call → invalid_argument.
        // Compare future writes with the bytes read from disk, before the
        // resume sanitizer repairs them. Repairs must not look like a write
        // from a competing window.
        let persisted_revision = agent_store::SessionStore::history_revision_of(&history);
        let mut history = history;
        for m in &mut history {
            if m.role == agent_llm::types::Role::Assistant {
                if let Some(text) = &mut m.content {
                    *text = strip_call_echo(text);
                    if text.trim().is_empty() {
                        m.content = None;
                    }
                }
            }
        }
        let mut known_call_ids = std::collections::HashSet::new();
        // 4. strip dangling `tool_calls` entries — a call whose output never
        //    landed (a malformed call skipped before the pairing fix, a
        //    crash mid-dispatch, a hand-edited snapshot) replays on strict
        //    backends as `function_call` with no `function_call_output` →
        //    deepseek rejects every later request with `No tool output
        //    found for tool call …`. Dropping the entry also orphan-drops
        //    nothing: outputs of *kept* calls are untouched below.
        let output_ids: std::collections::HashSet<String> = history
            .iter()
            .filter(|m| m.role == agent_llm::types::Role::Tool)
            .filter_map(|m| m.tool_call_id.clone())
            .collect();
        for m in &mut history {
            if m.role == agent_llm::types::Role::Assistant {
                if let Some(calls) = &mut m.tool_calls {
                    calls.retain(|c| output_ids.contains(&c.id));
                    if calls.is_empty() {
                        m.tool_calls = None;
                    }
                }
            }
        }
        history.retain(|m| {
            match m.role {
                agent_llm::types::Role::Assistant => {
                    for c in m.tool_calls.iter().flatten() {
                        known_call_ids.insert(c.id.clone());
                    }
                    // A crash may leave a reasoning-only streamed draft.
                    m.content.is_some()
                        || m.tool_calls.is_some()
                        || m.reasoning.as_ref().is_some_and(|s| !s.is_empty())
                }
                agent_llm::types::Role::Tool => m
                    .tool_call_id
                    .as_ref()
                    .map(|id| known_call_ids.contains(id))
                    .unwrap_or(false),
                _ => true,
            }
        });
        let (mut actor, channels) = Self::spawn_inner(cfg, Some(history), session_id);
        if actor
            .store
            .as_ref()
            .is_some_and(|s| s.has_history(session_id).ok() == Some(true))
        {
            actor.history_revision = persisted_revision;
            if let Some(checkpoint) = &actor.checkpoint {
                checkpoint.set_revision(persisted_revision);
            }
        }
        (actor, channels)
    }

    /// Shared spawn path — `prior_history` replaces the fresh system-prompt
    /// history when resuming; `session_id` is 0 for a fresh session (the
    /// caller assigns a real id via the store).
    fn spawn_inner(
        cfg: SessionConfig,
        prior_history: Option<Vec<ChatMessage>>,
        session_id: i64,
    ) -> (
        Self,
        agent_kernel::channels::UiChannels<crate::command::UiCommand>,
    ) {
        let mut channels = ui_channels();
        // One conversation, one routing key: everything this session builds —
        // foreground, fallback chain, background, children — advertises the
        // same identity, so a rebuild never cold-starts the provider cache.
        let conversation = conversation_key(session_id, &cfg.workspace_root);
        // Side traffic (the post-turn distiller, prompt-cache warming) runs on
        // its own provider instance — see `unobserved_provider_from`. It holds
        // no usage observer, so it can never move the foreground turn's
        // cache-write counter.
        let background_provider =
            unobserved_provider(&cfg, &conversation).unwrap_or_else(|| cfg.provider.clone());

        // Workspace skeleton + git snapshot.
        let workspace_tree = WorkspaceScanner::build_skeleton(&cfg.workspace_root)
            .map(|t| t.to_prompt_block())
            .unwrap_or_else(|e| format!("(workspace scan failed: {e})\n"));
        let git_status = git_snapshot(&cfg.workspace_root)
            .map(|s| s.to_prompt_block())
            .unwrap_or_else(|_| "(not a git repository)\n".to_string());

        let system_prompt = SYSTEM_PROMPT_TEMPLATE
            .replace("{{DATE}}", &chrono_lite_date())
            .replace(
                "{{WORKSPACE_ROOT}}",
                &cfg.workspace_root.display().to_string(),
            )
            .replace("{{PERMISSION_MODE}}", &cfg.permission_mode)
            .replace(
                "{{AGENT_MODE}}",
                crate::mode::AgentMode::from_str(&cfg.agent_mode).prompt_block(),
            )
            .replace("{{WORKSPACE_TREE}}", &workspace_tree)
            .replace("{{GIT_STATUS}}", &git_status);

        // `memory.db` lives in the host's data dir, partitioned by
        // `hash(canonical_root)`. `{{MEMORY_BLOCK}}` holds only the stable
        // half (persona + recent episodes) and is rendered once per spawn or
        // resume — refreshing it every turn would rewrite the system prompt
        // and miss the provider's prompt cache. Per-message facts are
        // recalled in `run_prompt` and sent beside the user message.
        // `memory_enabled` (default prefs) is the master switch: no store
        // handle → no block refresh and no distill writes. `memory_distill`
        // gates only the per-turn summarizer call — deterministic
        // episode/fact writes keep running.
        let mut skills = agent_skills::SkillManager::new(&cfg.workspace_root);
        let (memory, initial_memory_block, injected_persona) = {
            let store = if cfg.memory_enabled {
                open_memory_store(&cfg.workspace_root)
            } else {
                None
            };
            match store {
                Some(s) => {
                    let s = Arc::new(s);
                    // Only what the block actually carries counts as delivered:
                    // an entry the budget dropped never reached the model, and
                    // after this point the recall row is the only way to send
                    // it.
                    let (block, carried) = s
                        .stable_block_carried()
                        .unwrap_or_else(|_| (String::new(), Vec::new()));
                    (Some(s), block, carried.into_iter().collect())
                }
                None => (
                    None,
                    "(no memory)".to_string(),
                    std::collections::HashMap::new(),
                ),
            }
        };
        let system_prompt = set_memory_block(&system_prompt, &initial_memory_block)
            .replace("{{SKILLS_BLOCK}}", skills.catalog())
            .replace(
                "{{BLUEPRINT_BLOCK}}",
                &agent_coding::blueprint::prompt_block(&cfg.workspace_root, true),
            )
            .replace(
                "{{SUBAGENTS_BLOCK}}",
                &crate::subagents::catalog(&cfg.workspace_root, &cfg.paths),
            );

        // the host's user-level AGENTS.md then `<workspace>/AGENTS.md`
        // (project) append verbatim as dedicated sections, then the rule files
        // — custom rules live outside the template so they never fight its
        // contract.
        let mut system_prompt = system_prompt;
        append_prompt_tail(&mut system_prompt, &cfg.workspace_root, &cfg.paths);

        // Fresh session → just the rendered system prompt; resume → the
        // persisted history (its system prompt was rendered at that session's
        // spawn — memory/workspace baked in for that turn).
        let is_resume = prior_history.is_some();
        let mut history_revision = prior_history
            .as_deref()
            .map(agent_store::SessionStore::history_revision_of)
            .unwrap_or(0);
        let mut history = prior_history.unwrap_or_else(|| vec![ChatMessage::system(system_prompt)]);

        // A resumed session's system prompt was frozen at ITS spawn —
        // refresh the volatile regions so a reopened conversation keeps up:
        //   - `<!-- memory -->` markers let `set_memory_block` swap only the
        //     recall block (persona/facts/episodes change between sessions);
        //   - the trailing `## User instructions` and `## Rules` sections are
        //     re-appended from the CURRENT AGENTS.md files and rule files (they
        //     are always the tail of the prompt, so truncating at whichever
        //     marker comes first restores them exactly — a prompt that only
        //     has rules has no instructions marker at all).
        if is_resume {
            if let Some(sys) = history.get_mut(0) {
                if let Some(c) = sys.content.take() {
                    let mut c = set_memory_block(&c, &initial_memory_block);
                    // The mode and permission the session runs with are
                    // settings, not history: re-stamp them from this spawn's
                    // config so a reopened conversation cannot describe a
                    // registry it no longer dispatches against.
                    c = replace_block(
                        &c,
                        "mode",
                        crate::mode::AgentMode::from_str(&cfg.agent_mode).prompt_block(),
                    );
                    c = replace_block(&c, "permission", &permission_block(&cfg.permission_mode));
                    if let Some(idx) = prompt_tail_start(&c) {
                        c.truncate(idx);
                        append_prompt_tail(&mut c, &cfg.workspace_root, &cfg.paths);
                    }
                    sys.content = Some(c);
                }
            }
        }

        // Session persistence — the provider boundary also checkpoints the
        // current round and streamed draft before the whole turn completes.
        let store = match agent_store::SessionStore::open(&cfg.workspace_root) {
            Ok(store) => Some(Arc::new(store)),
            Err(error) => {
                let _ = channels.event_tx.send(UiEvent::SystemMessage(format!(
                    "会话存储不可用: {error}；todo 与历史持久化暂不可用，修复后重新打开会话"
                )));
                None
            }
        };
        if let Some(warning) = agent_store::session_storage_warning() {
            let _ = channels.event_tx.send(UiEvent::SystemMessage(warning));
        }
        // Linked workspaces — this session's reach beyond its own root. Read
        // through the workspace's hub so the prompt and the tool context agree
        // from the first call, and so a grant another window makes while this
        // session is starting is already in it. The hub is what every later
        // grant goes through, whichever side of the app asked for it.
        let links = crate::links::hub_for(&cfg.workspace_root, &cfg.paths);
        let linked_roots = Arc::new(agent_capabilities::LinkedRoots::default());
        linked_roots.set(links.effective(session_id));
        if let Some(sys) = history.get_mut(0) {
            if let Some(c) = sys.content.take() {
                sys.content = Some(replace_block(
                    &c,
                    "linked",
                    &linked_roots_block(&linked_roots.snapshot()),
                ));
            }
        }
        // The distiller and the spend it records are both built here: the
        // recorder needs the store handle, which exists only from this point.
        let distill_ctx = DistillCtx {
            provider: background_provider.clone(),
            model: cfg.model.clone(),
            params: cfg.model_params.clone().unwrap_or_default(),
        };
        let distill_record: Option<DistillRecorder> = store.as_ref().map(|s| {
            let store = s.clone();
            std::sync::Arc::new(move |prompt: u32, completion: u32| {
                // The journal write is blocking file IO, and this runs on the
                // task streaming the distillation: keep it off that worker, or
                // a slow disk holds the memory pass open.
                let store = store.clone();
                let write = move || {
                    let _ = store.add_distill_usage(session_id, prompt, completion);
                };
                match tokio::runtime::Handle::try_current() {
                    Ok(rt) => {
                        // Deliberately dropped: the write is fire-and-forget,
                        // and awaiting it would hold the distillation task.
                        drop(rt.spawn_blocking(write));
                    }
                    // No runtime to hand it to — a bare store test. There is no
                    // async worker here for the write to stall.
                    Err(_) => write(),
                }
            }) as DistillRecorder
        });
        let distiller = build_distiller(
            memory.as_ref(),
            cfg.memory_distill,
            &distill_ctx,
            distill_record.clone(),
        );
        // Forks/importers may supply prior history before any journal exists.
        // Existing journals still compare against the supplied revision, so
        // a concurrently updated persisted session cannot be overwritten.
        if store
            .as_ref()
            .is_some_and(|s| s.has_history(session_id).ok() == Some(false))
        {
            history_revision = 0;
        }

        let checkpoint = store.as_ref().map(|store| {
            let (forward, receiver) = agent_kernel::channels::UiSink::channel();
            let events = std::mem::replace(&mut channels.event_rx, receiver);
            TurnCheckpoint::new(
                store.clone(),
                history_revision,
                channels.event_tx.clone(),
                events,
                forward,
            )
        });

        // Cooperative cancel flag — hoisted above `ToolCtx` so delegated
        // subagents inherit it (a user cancel tears down in-flight child
        // runs instead of orphaning them).
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Engine steering channel — created before the tool context so the
        // background-job notifier can hold a sender from birth.
        let (steer_tx, steer_rx) = mpsc::channel::<String>(32);
        // Host notices (a background job finishing) get their own channel: a
        // notice carries no user intent, so the engine appends it as a system
        // row instead of a user steer — otherwise untrusted job output would
        // pose as the user's words and pollute the turn's steering record.
        let (notice_tx, notice_rx) = mpsc::channel::<String>(32);
        let (registry, child_full) = base_registries();
        // Subagent registries: a child never carries `delegate` (recursion
        // guard) or `HumanInteraction` tools (nobody can answer a child's
        // question — deny at the policy layer, not just via missing ui_tx);
        // `readonly` children drop every non-readonly tool too.
        //
        // `computer` is excluded by the same rule: nobody can approve a
        // child's click. The policy layer does NOT cover this case —
        // `for_subagent()` is `auto` with `headless` on, and `headless` only
        // turns `Ask` into `Deny`, while `auto` never asks for a
        // `Process`-class tool. Excluding it here is the guard, and the
        // child's `ToolCtx` carries a refusing desktop backend as the second
        // one. `screenshot` stays: looking at the screen is an observation a
        // delegated visual check legitimately needs.
        let child_ro = child_full.readonly_only();
        let mut tool_ctx = ToolCtx::new()
            .with_workspace(&cfg.workspace_root)
            .with_session(session_id);
        // The handle the prompt was rendered from. It is `insert_shared`, so a
        // delegated child reaches the same roots, and a later grant lands in
        // every context at once — they share the `Arc`.
        tool_ctx.install_linked_roots(linked_roots.clone());
        // The kernel asks before a refusal becomes a wall: a path outside every
        // root comes back as `OutOfScope`, and this answers it with a grant.
        // It is the same hub the manager's menu writes through, so one grant
        // path serves the tool, the card and the composer — and its refresh
        // reaches every other live session of this workspace.
        {
            let hub = links.clone();
            tool_ctx.install_scope_grant(Arc::new(move |action: &str, path: &str, access: &str| {
                let Some(layer) = crate::links::Layer::from_action(action) else {
                    return Err(format!("`{action}` 不是本会话支持的授权动作"));
                };
                hub.grant(session_id, path, access, layer).map(|_| ())
            }));
        }
        // This actor's handle is what the hub refreshes on every grant; the
        // notice channel is how the model hears about one it did not make.
        links.register(session_id, &linked_roots, &notice_tx);
        // The todo list persists beside this session's history. A session with
        // no store (the db failed to open) gets the kernel's in-memory list.
        if let Some(store) = store.clone() {
            crate::todo_store::SessionTodoStorage::new(store, session_id, &cfg.workspace_root)
                .install(&mut tool_ctx);
        }
        // This context's own goal/plan contract pair — the coding crate owns
        // the controller types, so installation is an explicit host step now.
        agent_coding::install_contract(&mut tool_ctx);
        // File-write tracking for undo and the files-changed list: the file tools
        // report applied writes through the observer installed here; the kernel
        // engine never sees a tracker.
        // `AllDirty` seeds the tracker with git-dirty files so a dirty-tree
        // warning can precede the first prompt.
        let hunks = WriteTracker::new(if cfg.track_dirty {
            TrackingMode::AllDirty
        } else {
            TrackingMode::AgentOnly
        });
        if cfg.track_dirty {
            if let Ok(snap) = git_snapshot(&cfg.workspace_root) {
                // status lines are porcelain XY-prefixed (` M a.rs`, `?? b.rs`)
                for line in &snap.status {
                    let p = line.get(3..).map(str::trim).unwrap_or("");
                    if !p.is_empty() {
                        hunks.lock().mark_dirty_at_start(PathBuf::from(p));
                    }
                }
            }
        }

        hunks.install(&mut tool_ctx);
        let observations = agent_coding::observe::ObservationLedger::install(&mut tool_ctx);
        tool_ctx.cancel = Some(cancel.clone());
        // The host's sandbox: the detected backend behind the live master switch.
        // Installed here, not in `ToolCtx::new`, so a context built elsewhere
        // (an SSH worker, a test) has no local sandbox unless it asks for one.
        let (detected, _loud) = agent_sandbox::detect_backend();
        tool_ctx.install_paths(cfg.paths.clone());
        tool_ctx.install_sandbox(Arc::new(
            crate::sandbox_prefs::SwitchableSandbox::new(Arc::from(detected))
                .with_deny_read(crate::sandbox_prefs::protected_paths(&cfg.paths)),
        ));
        // Background services belong to this session: its `service` tool lists
        // and stops only its own, and they are killed when the session drops.
        agent_tools_shell::services::install_registry(&mut tool_ctx);
        // Background jobs share that lifetime. A finished job pushes a notice
        // into the engine's notice channel (delivered between tool calls, or
        // at the next turn when the session is idle). The UI learns of it
        // from the job list (`husk:bg_jobs`, fed in `run()`), not from the
        // notice. The sink is installed per *context* on purpose: a delegated
        // child inherits the registry (this session can see and stop its
        // jobs) but not the sink, so a child's job never reports here.
        let bg_jobs = agent_tools_shell::bg::install_registry(&mut tool_ctx);
        let bg_restored: BgRestored = Arc::new(std::sync::Mutex::new(
            store
                .as_ref()
                .and_then(|s| s.load_bg_jobs(session_id))
                .map(restored_bg_jobs)
                .unwrap_or_default(),
        ));
        // An idle session wakes on a notice instead of holding it until the
        // next prompt — a finished build must reach the model on its own. The
        // notice goes first: whenever the wake lands, the text is already on
        // the channel for the actor to take.
        let (wake_tx, wake_rx) = mpsc::channel::<()>(1);
        {
            let notice = notice_tx.clone();
            let ui = channels.event_tx.clone();
            let wake = wake_tx.clone();
            // Said once per run of overflow, not once per lost notice: a burst
            // of finishes would otherwise print the same line a dozen times.
            let overflow_told = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            agent_tools_shell::bg::install_notifier(&mut tool_ctx, move |text| {
                // A full channel is 32 unread notices — tell the user rather
                // than lose the report silently; the job list still shows it.
                if notice.try_send(text).is_err() {
                    if !overflow_told.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        let _ = ui.send(UiEvent::SystemMessage(
                            "后台任务的完成通知过多，部分未送达模型".into(),
                        ));
                    }
                } else {
                    // The channel drained in between — a later overflow is news.
                    overflow_told.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                // One pending wake is enough: the actor drains the channel.
                let _ = wake.try_send(());
            });
        }
        let memory_handle = agent_coding::remember::MemoryHandle::default();
        memory_handle.set(memory.clone());
        memory_handle.install(&mut tool_ctx);
        // The child route starts from the session's own settings and is
        // rewritten by `apply_model_switch`, so children follow a hot-swap.
        // `child_base` is the parent's own model; the shared route widens it to
        // every configured provider and applies the subagent default on top.
        let child_base = agent_kernel::tools::delegate::ChildRoute {
            provider: background_provider.clone(),
            model: cfg.model.clone(),
            temperature: cfg.temperature,
            context_window: cfg.context_window.unwrap_or(256_000) as usize,
            thinking_level: cfg.thinking_level.clone(),
            thinking_level_map: cfg.thinking_level_map.clone(),
            model_params: cfg.model_params.clone().unwrap_or_default(),
            model_input: cfg.model_input.clone(),
            models: std::collections::HashMap::new(),
        };
        // Every frontend spawns through here, so the stored default is read
        // once at birth; `SetSubagentModel` carries later changes.
        let subagent_defaults = crate::prefs::try_load_default_preferences();
        let subagent_model = subagent_defaults.as_ref().and_then(|d| d.subagent_pair());
        let subagent_thinking = subagent_defaults
            .as_ref()
            .and_then(|d| d.subagent_thinking_level.clone());
        let child_route = Arc::new(std::sync::RwLock::new(routed_child(
            &child_base,
            &cfg.paths.config_dir,
            &cfg.provider_name,
            subagent_model.as_ref(),
            subagent_thinking.as_deref(),
            &conversation,
        )));
        tool_ctx.install_subagent(
            agent_kernel::tools::delegate::SubagentSpawner::new(
                child_route.clone(),
                Arc::new(child_full),
                Arc::new(child_ro),
            )
            .with_policy(agent_coding::policy::coding_policy_factory(
                cfg.workspace_root.clone(),
            ))
            .with_host(Arc::new(crate::subagents::HuskSubagents)),
        );
        // `ask_question` emits its card through the session's event channel;
        // fan-out tools (batch items) emit ToolCallStarted/Finished on ui_tx.
        tool_ctx.install_ask(Arc::new(agent_kernel::tools::registry::AskChannel::new(
            Some(channels.event_tx.clone()),
        )));
        tool_ctx.install_ui(channels.event_tx.clone());
        let ctx = Arc::new(tool_ctx);
        let registry = Arc::new(registry);
        let warming = agent_llm::WarmingControl::default();
        // Warming replays requests on its own schedule: same rule as the
        // distiller — the replays must not share the session's usage observer,
        // while the turn itself still reports through it.
        let warmed = warmed_provider(
            cfg.provider.clone(),
            background_provider.clone(),
            &cfg.model,
            resolved_model(&cfg).as_ref(),
            &warming,
        );
        let provider = checkpoint.as_ref().map_or_else(
            || warmed.clone(),
            |checkpoint| checkpoint.wrap(warmed.clone()),
        );
        let mut engine = Engine::new(
            provider,
            registry.clone(),
            ctx.clone(),
            cfg.model.clone(),
            cfg.temperature,
        );
        // No round cap. A single message can legitimately need hundreds of
        // tool round-trips — a repo-wide refactor, a review that reads forty
        // files, a fix that edits ten and re-runs the suite after each. The
        // kernel's limit ended that kind of turn by taking the tools away and
        // asking the model to summarize what it had so far, which for a coding
        // task means stopping mid-edit.
        //
        // What actually bounds a turn here, and bounds it honestly: the user's
        // cancel, the context window (the per-round sanitize/compaction pass
        // owns that), and the wake-up budget on the unattended paths. A count
        // of round-trips is not a signal that the agent is going wrong — a
        // loop that repeats the same call with the same arguments is, and that
        // is a different check.
        engine.set_max_tool_rounds(usize::MAX);
        let coding_policy =
            agent_coding::policy::CodingPolicy::from_ctx(&ctx, cfg.workspace_root.clone());
        debug_assert!(
            coding_policy.has_contract(),
            "install_contract must run on the ToolCtx before the coding policy is built"
        );
        let architecture = coding_policy.architecture_switch();
        engine.set_policy(Box::new(coding_policy));
        // Mode table — the host's mapping from a mode id to the registry that
        // id dispatches against. The engine carries no mode names; these are
        // the coding product's (mirroring `mode::AgentMode`).
        //
        // Plan mode = readonly set + the `submit_plan` contract — the model
        // declares its deliverable as structured data, not prose.
        let mut plan_reg = registry.readonly_only();
        plan_reg.register(agent_coding::plan::spec_submit());
        engine.add_mode(agent_kernel::engine::ModeEntry {
            id: agent_coding::MODE_PLAN.into(),
            registry: Arc::new(plan_reg),
            allow_plugins: false,
        });
        // Goal mode = full set + the completion contract — the model
        // declares `goal_complete`/`goal_blocked` to end its run.
        let mut goal_reg = (*registry).clone();
        goal_reg.register(agent_coding::goal::spec_complete());
        goal_reg.register(agent_coding::goal::spec_blocked());
        engine.add_mode(agent_kernel::engine::ModeEntry {
            id: agent_coding::MODE_GOAL.into(),
            registry: Arc::new(goal_reg),
            allow_plugins: true,
        });
        // Office mode = full set + the `office_*` document tools (officecli
        // wrappers). Same write surface as build — the difference is the
        // document-producing contract in the prompt, not a narrower toolbox.
        let mut office_reg = (*registry).clone();
        for spec in agent_tools_office::office::specs() {
            office_reg.register(spec);
        }
        for spec in agent_tools_web::assets::specs() {
            office_reg.register(spec);
        }
        engine.add_mode(agent_kernel::engine::ModeEntry {
            id: "office".into(),
            registry: Arc::new(office_reg),
            allow_plugins: true,
        });
        engine.set_permissions(
            agent_kernel::permissions::PermissionGate::from_mode_str(&cfg.permission_mode)
                .with_policy(
                    "computer",
                    Arc::new(agent_coding::computer_policy::ComputerPolicy),
                ),
        );
        engine.set_agent_mode(cfg.agent_mode.clone());
        engine.set_thinking_level_map(cfg.thinking_level_map.clone());
        engine.set_model_thinking_level(cfg.thinking_level.clone());
        engine.set_context_window(cfg.context_window.unwrap_or(256_000) as usize);
        engine.set_compact_at(
            cfg.compact_at
                .unwrap_or(agent_kernel::compaction::COMPACT_AT),
        );
        engine.set_model_input(cfg.model_input.clone());
        engine.set_model_params(cfg.model_params.clone().unwrap_or_default());
        if let Some(handle) = cfg.plugins.clone() {
            engine.set_plugins(crate::plugins::router_handle(handle));
        }
        let decision_slot = engine.decision_slot();
        let ask_channel = engine.ask_channel();
        let permissions_slot = engine.permissions_writer();
        let thinking_slot = engine.thinking_level_shared();
        // This session's loop guard, layered under the plugin chain whenever
        // one is installed — see `SessionActor::set_hooks`.
        let repeat_guard: Arc<crate::repeat_guard::RepeatGuard> = Arc::new(Default::default());
        if let Some(checkpoint) = &checkpoint {
            checkpoint.set_settings(
                permissions_slot.clone(),
                engine.agent_mode_writer(),
                thinking_slot.clone(),
            );
        }
        // Installed with or without a checkpoint: the guard is the session's
        // own, and a headless test spawns an actor with no plugins at all.
        let local: Vec<Arc<dyn agent_kernel::hooks::AgentHook>> = vec![repeat_guard.clone()];
        let layered = HookChain::layered(local, &HookChain::new());
        engine.set_hooks(match &checkpoint {
            Some(checkpoint) => checkpoint.hooks(layered),
            None => layered,
        });

        // Actor keeps the real cmd_rx — `run()` drains it into `handle()`.
        // The `channels.cmd_rx` left for the caller is a dead end; the
        // frontend sends on `command_sender()`.
        let cmd_rx = std::mem::replace(&mut channels.cmd_rx, mpsc::channel(1).1);
        let cmd_tx = channels.cmd_tx.clone();

        // The steering channel was created above (the bg notifier holds a
        // sender); `SessionActor` forwards `UiCommand::Steer` texts into it
        // mid-turn and the engine drains them between tool calls.
        // P0-C4: shared cancel flag — the UI writes it via `cancel_writer()`
        // (bypassing the command pump so a Cancel lands mid-turn); the engine
        // polls it between chunks and before each tool dispatch. Created above
        // `ToolCtx` so the subagent spawner already carries a clone.
        let io = EngineIo {
            ui_tx: channels.event_tx.clone(),
            steer_rx,
            notice_rx,
            cancel: cancel.clone(),
        };

        (
            Self {
                history,
                engine,
                architecture,
                model: cfg.model.clone(),
                provider_name: cfg.provider_name.clone(),
                state: AgentState::Idle,
                io,
                cmd_tx,
                hunks,
                observations,
                notices: notice_tx.clone(),
                linked_roots,
                decision_slot,
                ask_channel,
                permissions_slot,
                thinking_slot,
                model_slot: Arc::new(std::sync::RwLock::new((
                    cfg.provider_name.clone(),
                    cfg.model.clone(),
                ))),
                child_route,
                child_base,
                subagent_model,
                subagent_thinking,
                steer_tx,
                cancel,
                cmd_rx,
                shutdown: Arc::new(tokio::sync::Notify::new()),
                workspace_root: cfg.workspace_root.clone(),
                paths: cfg.paths,
                // Empty until `set_hooks` installs the plugin chain — the
                // manager does it before the actor starts running. The loop
                // guard is not part of that chain: it belongs to this session
                // and is layered back in on every install.
                hooks: HookChain::new(),
                repeat_guard,
                plugins: cfg.plugins.clone(),
                memory,
                memory_handle,
                distiller,
                distill_ctx,
                memory_distill: cfg.memory_distill,
                distill_record,
                injected_facts: Vec::new(),
                injected_persona,
                session_id,
                conversation,
                store,
                history_revision,
                turn_started_at: None,
                checkpoint,
                usage_observer: None,
                warming,
                last_usage: None,
                last_breakdown: None,
                queued: Arc::new(std::sync::Mutex::new(cfg.queued_prompts.into())),
                skills,
                bg_jobs,
                bg_restored,
                wake_rx,
                wakes_in_row: 0,
            },
            channels,
        )
    }

    /// The actor's command pump — `run()` consumes `UiCommand`s until the
    /// frontend's sender drops. A `Prompt` runs its turn inline.
    ///
    /// Mid-turn `ToolDecision`/`Cancel`/`Steer` do not flow through this pump —
    /// they would block behind the running turn. They arrive via the shared
    /// `decision_slot`, the `EngineIo::cancel` flag and `steer_tx` instead.
    pub async fn run(&mut self) {
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.start();
        }
        self.spawn_bg_feed();
        loop {
            let cmd = tokio::select! {
                biased;
                _ = self.shutdown.notified() => break,
                // A prompt outranks a wake: the user is waiting.
                cmd = self.cmd_rx.recv() => cmd,
                Some(()) = self.wake_rx.recv() => {
                    self.run_notice_wake().await;
                    self.drain_queue().await;
                    continue;
                }
                // The IPC layer routes `Steer` straight into the engine's
                // channel, never through this pump, so while nothing runs the
                // actor is that channel's only reader — without this arm a
                // steer arriving the instant a turn ended would wait for the
                // next prompt. Mid-turn the select is not polled at all: the
                // engine owns the channel then, and `drain_queue` picks up
                // whatever it left behind.
                Some(text) = self.io.steer_rx.recv() => {
                    self.handle_prompt(text).await;
                    self.drain_queue().await;
                    continue;
                }
            };
            let Some(cmd) = cmd else { break };
            if self
                .store
                .as_ref()
                .is_some_and(|s| s.is_deleted(self.session_id))
            {
                break;
            }
            self.handle(cmd).await;
        }
        // The registry drops with the actor and takes the log files with it: a
        // job still running now has printed more since its last change, and
        // this is the last moment its output can be read. A deleted session's
        // record is gone and must stay gone.
        if let Some(store) = &self.store {
            if !store.is_deleted(self.session_id) && !self.bg_jobs.jobs().is_empty() {
                let _ = store.save_bg_jobs(self.session_id, &bg_record(&self.bg_restored, &self.bg_jobs));
            }
        }
        // A closed or deleted session must not keep paying for refreshes.
        self.warming.cancel();
        self.io.ui_tx.close();
    }

    /// Mirror the job registry to the UI: one full snapshot per settled burst
    /// of changes. The task holds the registry weakly and the sink closes with
    /// the actor, so it ends with the session instead of keeping it alive.
    fn spawn_bg_feed(&self) {
        let reg = Arc::downgrade(&self.bg_jobs);
        let restored = self.bg_restored.clone();
        let mut feed = self.bg_jobs.subscribe();
        let ui = self.io.ui_tx.clone();
        let store = self.store.clone();
        let session_id = self.session_id;
        tokio::spawn(async move {
            while feed.changed().await.is_ok() {
                tokio::time::sleep(BG_FEED_SETTLE).await;
                feed.mark_unchanged();
                let Some(reg) = reg.upgrade() else { break };
                let jobs = bg_snapshot(&restored, &reg);
                // The pane outlives the run: the jobs die with this process, so
                // what the sidebar shows next time it starts is this record —
                // the earlier runs' entries included, or a restart's first job
                // would erase them. The record also keeps each job's log tail:
                // the log file goes with the process.
                if let Some(store) = &store {
                    let _ = store.save_bg_jobs(session_id, &bg_record(&restored, &reg));
                }
                let event = UiEvent::Custom {
                    kind: BG_JOBS.into(),
                    payload: serde_json::json!({ "jobs": jobs }),
                };
                if ui.send(event).is_err() {
                    break;
                }
            }
        });
    }

    /// The session's background-job registry — shared with the manager so
    /// the UI can list and control jobs without going through the pump.
    pub fn bg_registry(&self) -> Arc<agent_tools_shell::bg::BgRegistry> {
        self.bg_jobs.clone()
    }

    /// The entries read back from the job record — shared with the manager so
    /// the pane lists them and can drop one while the actor is up.
    pub fn bg_restored(&self) -> BgRestored {
        self.bg_restored.clone()
    }
    /// The session's linked workspaces — the same handle its tool context and
    /// every delegated child read, so `set` takes effect on the next call.
    pub fn linked_roots(&self) -> Arc<agent_capabilities::LinkedRoots> {
        self.linked_roots.clone()
    }

    /// Tell the model something between turns — the same channel background
    /// jobs report on. Nothing reaches the wire until the next turn, which is
    /// what a permission change wants: the tools are already live.
    pub fn notice_writer(&self) -> mpsc::Sender<String> {
        self.notices.clone()
    }
    /// May an undo write to `path`? The workspace, or a linked root that
    /// granted writes — the same boundary the patch tools hold, so a file the
    /// agent changed in a linked workspace can be rolled back too.
    fn undo_reaches(&self, path: &std::path::Path) -> bool {
        if path.starts_with(&self.workspace_root) {
            return true;
        }
        self.linked_roots.snapshot().iter().any(|r| {
            r.access == agent_capabilities::RootAccess::ReadWrite && path.starts_with(&r.path)
        })
    }

    pub fn shutdown_writer(&self) -> Arc<tokio::sync::Notify> {
        self.shutdown.clone()
    }

    /// Attach the host's cache-write sink after construction. The provider was
    /// built with the same `Arc`, and the checkpoint relay reads it when a
    /// `Usage` event passes through (so the live top bar can price/write-count
    /// Anthropic cache writes without changing the shared wire types).
    pub fn set_usage_observer(&mut self, observer: Arc<agent_llm::UsageObserver>) {
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.set_usage_observer(observer.clone());
        }
        self.usage_observer = Some(observer);
    }

    /// The frontend's command sender — the bridge clones this.
    /// `Prompt`/`Steer`/`SetModel`/`UndoLastTurn` go here.
    pub fn command_sender(&self) -> mpsc::Sender<UiCommand> {
        self.cmd_tx.clone()
    }

    /// Install the lifecycle hook chain — the manager calls this right after
    /// the actor is built and before its thread starts, so the first turn
    /// already runs under the plugins' hooks. The chain is shared, so a
    /// later `reload_plugins` reaches this session without a restart.
    pub fn set_hooks(&mut self, chain: HookChain) {
        // The loop guard is this session's own hook, not a plugin: it goes in
        // as the local layer so a reload (which replaces the shared chain)
        // cannot take it away.
        let local: Vec<Arc<dyn agent_kernel::hooks::AgentHook>> =
            vec![self.repeat_guard.clone()];
        let layered = HookChain::layered(local, &chain);
        let engine_chain = self.checkpoint.as_ref().map_or_else(
            || layered.clone(),
            |checkpoint| checkpoint.hooks(layered.clone()),
        );
        self.hooks = chain;
        self.engine.set_hooks(engine_chain);
    }

    /// Resolved provider keys for the egress masker. Installed before the
    /// actor runs so the first request is already scrubbed.
    pub fn set_secrets(&mut self, secrets: Vec<String>) {
        self.engine.set_secrets(secrets);
    }

    /// Degrade chain for this session's primary provider.
    pub fn set_fallbacks(
        &mut self,
        primary: &str,
        chain: Vec<(String, Arc<dyn agent_llm::LlmProvider>)>,
    ) {
        let chain = chain
            .into_iter()
            .map(|(name, provider)| {
                let provider = self.checkpoint.as_ref().map_or_else(
                    || provider.clone(),
                    |checkpoint| checkpoint.wrap(provider.clone()),
                );
                (name, provider)
            })
            .collect();
        self.engine.set_fallbacks(primary, chain);
    }

    /// The session's `.blueprint/` switch. Forwarded to the coding policy (which
    /// gates the turn-end check and baseline), and the system prompt's
    /// blueprint region is re-rendered at once so the index never shows for
    /// a session that turned the layer off.
    pub fn set_blueprint_enabled(&mut self, enabled: bool) {
        self.architecture.set(enabled);
        let block = agent_coding::blueprint::prompt_block(&self.workspace_root, enabled);
        self.rewrite_prompt_block("blueprint", &block);
    }

    /// Swap one `<!-- name -->` region of the rendered system prompt.
    ///
    /// The prompt is rendered once per spawn and heads the provider's prompt
    /// cache, so a setting the user flips mid-session rewrites its own region
    /// instead of rebuilding the whole text. Marker-located: matching the old
    /// block by content would silently no-op on a session resumed from a build
    /// that worded it differently.
    fn rewrite_prompt_block(&mut self, name: &str, block: &str) {
        let Some(sys) = self.history.first_mut() else {
            return;
        };
        if sys.role != agent_llm::Role::System || sys.notice.is_some() {
            return;
        }
        if let Some(content) = sys.content.take() {
            sys.content = Some(replace_block(&content, name, block));
        }
    }

    /// The shared tool-decision slot — the bridge writes `ToolDecision`
    /// **here directly**, bypassing the command channel so a mid-turn
    /// approval isn't queued behind the running `run_turn`.
    pub fn decision_writer(&self) -> Arc<std::sync::Mutex<Option<(u64, agent_kernel::tools::registry::ToolVerdict)>>> {
        self.decision_slot.clone()
    }

    /// `AnswerQuestion` resolves the parked `ask_question` oneshot —
    /// bypasses the pump for the same reason `ToolDecision` does.
    pub fn ask_channel(&self) -> Arc<agent_kernel::tools::registry::AskChannel> {
        self.ask_channel.clone()
    }

    /// The shared permission gate slot — updates apply immediately mid-turn.
    pub fn permissions_writer(
        &self,
    ) -> Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>> {
        self.permissions_slot.clone()
    }

    /// The shared agent-mode slot — `SetAgentMode` writes here mid-turn so
    /// the engine's next sampling round already sees the swapped registry.
    pub fn agent_mode_writer(&self) -> Arc<std::sync::RwLock<String>> {
        self.engine.agent_mode_writer()
    }

    /// The shared thinking-level slot — read-only mirror of the engine's
    /// level so the manager can report the session's actual value.
    pub fn thinking_writer(&self) -> Arc<std::sync::RwLock<Option<String>>> {
        self.thinking_slot.clone()
    }

    /// The shared `(provider, model)` slot — see `model_slot`.
    pub fn model_writer(&self) -> Arc<std::sync::RwLock<(String, String)>> {
        self.model_slot.clone()
    }

    /// The mid-turn steering sender — `Steer` texts go here when a turn is
    /// already in flight (the engine drains between rounds). A steer that
    /// arrives with no round left to join is not lost: the actor runs it as
    /// the next prompt.
    pub fn steer_writer(&self) -> mpsc::Sender<String> {
        self.steer_tx.clone()
    }

    /// The shared queued-prompt deque — the IPC layer writes `Enqueue` /
    /// `SetQueued` ops here directly so the list (and its `QueuedPrompts`
    /// echo) updates instantly even mid-turn; a `DrainQueue` marker down
    /// `cmd_tx` then schedules the drain for when the pump is free.
    pub fn queue_writer(&self) -> Arc<std::sync::Mutex<std::collections::VecDeque<String>>> {
        self.queued.clone()
    }

    /// The shared cancel flag — the UI writes `true` here **directly** to
    /// abort the in-flight turn, bypassing the command pump (a `UiCommand::
    /// Cancel` queued behind `run_turn` would never be seen in time).
    /// `run_prompt` clears it at turn start.
    pub fn cancel_writer(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.cancel.clone()
    }

    /// Current state-machine position.
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Read-only view of history length (for compaction pressure checks).
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Shared Prompt/Steer-between-turns path — extracted so `Steer` doesn't
    /// need a recursive `handle` call (async recursion requires boxing).
    async fn run_prompt(&mut self, text: String) {
        info!("user prompt ({} chars)", text.len());
        // The user is back: the wake-up budget starts over.
        self.wakes_in_row = 0;
        let turn = self.begin_turn().await;
        let _ = self.io.ui_tx.send(UiEvent::UserPrompt(text.clone()));

        // Recalled memory rides a hidden user row right before the message,
        // never the system prompt: the system message is the head of the
        // provider's prompt cache, and a per-question block there would turn
        // every request into a full cache miss. Persisted with the turn, so
        // later requests replay the same bytes and keep the prefix warm.
        //
        // It carries a delta, not a dump: facts the model has not read yet
        // (re-sending one is pure token spend) plus any persona written since
        // the system prompt was rendered — a mid-session `remember` would
        // otherwise only be visible after a restart.
        let seen: std::collections::HashSet<u64> =
            self.injected_facts.iter().map(|(_, id)| *id).collect();
        let (block, ids, persona) = match self.memory.clone() {
            Some(m) => {
                // Recall reads the whole fact table, and with the semantic
                // model loaded it also runs inference — seconds of CPU that
                // must not hold a runtime worker, because this is an async
                // task and every other session shares its pool.
                let persona_seen = self.injected_persona.clone();
                let prompt = text.clone();
                tokio::task::spawn_blocking(move || {
                    let (facts, ids) =
                        m.facts_block_excluding(&prompt, &seen).unwrap_or_default();
                    let (persona, rows) = m.persona_delta_block(&persona_seen).unwrap_or_default();
                    (format!("{facts}{persona}"), ids, rows)
                })
                .await
                .unwrap_or_default()
            }
            None => (String::new(), Vec::new(), Vec::new()),
        };
        if !block.trim().is_empty() {
            // The row lands at `at - 1` and the prompt it belongs to at `at`:
            // a rewind that truncates history at the prompt drops both.
            let at = self.history.len() + 1;
            self.history.push(ChatMessage::user_hidden(format!(
                "{MEMORY_CONTEXT_OPEN}\n{}\n</memory-context>",
                block.trim_end()
            )));
            self.injected_facts.extend(ids.into_iter().map(|id| (at, id)));
        }
        self.injected_persona.extend(persona);

        self.run_engine_turn(TurnInput::Prompt(text), turn).await;
    }

    /// A background job finished while the session sat idle: wake the model on
    /// the completion notices rather than holding them until the next prompt.
    /// Nothing happens when a running turn already drained them, or when the
    /// session was deleted meanwhile.
    async fn run_notice_wake(&mut self) {
        if self.state.is_active() {
            return;
        }
        if self
            .store
            .as_ref()
            .is_some_and(|s| s.is_deleted(self.session_id))
        {
            return;
        }
        // Past the cap the notices stay on the channel — the next prompt's
        // turn drains them — so a job chain the model keeps restarting
        // cannot run on unattended. Said once, when the cap is reached.
        if self.wakes_in_row >= MAX_WAKES_IN_ROW {
            if self.wakes_in_row == MAX_WAKES_IN_ROW {
                self.wakes_in_row += 1;
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(
                    "后台任务已连续唤醒模型多次，后续结果将在你下次发言时送达".into(),
                ));
            }
            return;
        }
        // The notifier sends the notice before the wake, so whatever is queued
        // here belongs to this wake-up. Drained here rather than in the engine
        // so an empty set means "a turn already took it" and starts nothing.
        let mut notices = Vec::new();
        while let Ok(text) = self.io.notice_rx.try_recv() {
            notices.push(text);
        }
        if notices.is_empty() {
            return;
        }
        info!(notices = notices.len(), "waking on background notices");
        self.wakes_in_row += 1;
        let turn = self.begin_turn().await;
        self.run_engine_turn(TurnInput::Notices(notices), turn)
            .await;
    }

    /// Everything a turn does before the model is asked: the cancel reset, the
    /// opening state, the prompt blocks that may have changed since the last
    /// turn, and the write tracker's turn mark. Returns that mark.
    async fn begin_turn(&mut self) -> u32 {
        // Clear the cancel flag so a stale Cancel from a previous turn can't
        // abort this one before it starts.
        self.cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.state = AgentState::ScanningWorkspace;
        // The session's own writes feed the same transition slot the engine
        // emits into, so a state both sides announce is one hook event.
        self.hooks.observe(&self.state).await;
        let turn = self.hunks.lock().begin_turn();

        // Refresh the skills catalog when a skill was added, removed, or
        // edited. `SkillManager::refresh` renders only when the tree's mtime
        // stamp moved, so the common turn skips the scan entirely.
        if let Some(block) = self.skills.refresh() {
            let block = block.to_string();
            if let Some(sys) = self.history.get_mut(0) {
                if let Some(content) = sys.content.take() {
                    sys.content = Some(replace_block(&content, "skills", &block));
                }
            }
        }

        // Subagent catalog — cheap to rebuild (a handful of small manifests) and
        // the settings UI can add one while the session is open.
        if let Some(sys) = self.history.get_mut(0) {
            if let Some(content) = sys.content.take() {
                let block = crate::subagents::catalog(&self.workspace_root, &self.paths);
                sys.content = Some(replace_block(&content, "subagents", &block));
            }
        }

        // Blueprint index — re-rendered every turn so a `.blueprint/` edit
        // (by the agent last turn, or the user in an editor) reaches the
        // next request. An unchanged directory renders byte-identical text,
        // so the cached prompt prefix is untouched. Pre-marker prompts
        // (sessions persisted before the layer existed) carry no region and
        // stay as they are.
        if let Some(sys) = self.history.get_mut(0) {
            if let Some(content) = sys.content.take() {
                let block = agent_coding::blueprint::prompt_block(
                    &self.workspace_root,
                    self.architecture.get(),
                );
                sys.content = Some(replace_block(&content, "blueprint", &block));
            }
        }
        turn
    }

    /// Run the model and settle the turn — shared by a user prompt and a
    /// background wake-up.
    async fn run_engine_turn(&mut self, input: TurnInput, turn: u32) {
        // A wake-up carries no user intent: it is not a task the distiller
        // can learn a correction from, so only a prompt is distilled.
        let was_prompt = matches!(&input, TurnInput::Prompt(_));
        let outcome_text = match &input {
            TurnInput::Prompt(text) => text.clone(),
            // Nothing was typed: the notices are the turn's subject, so the
            // checkpoint names them.
            TurnInput::Notices(notices) => notices.first().cloned().unwrap_or_default(),
        };
        self.turn_started_at = Some(unix_now());
        if let (Some(checkpoint), Some(meta)) = (&self.checkpoint, self.session_meta(&outcome_text))
        {
            checkpoint.begin(&self.history, &outcome_text, meta);
        }
        let result = match input {
            TurnInput::Prompt(text) => {
                self.engine
                    .run_turn(&mut self.io, &mut self.history, text)
                    .await
            }
            TurnInput::Notices(notices) => {
                self.engine
                    .run_notice_turn(&mut self.io, &mut self.history, notices)
                    .await
            }
        };
        // The turn is over either way: `streaming` warming stops, `idle`
        // switches to the idle horizon.
        self.warming.settle();
        match result {
            // `run_turn` already emitted the terminal `StateChanged` on every
            // exit path (Finished at the assistant-message boundary, Failed at
            // each early return) — re-emitting here doubles the event and the
            // frontend's turn-end notification.
            Ok(outcome) => {
                self.state = AgentState::Finished;
                self.hooks.observe(&self.state).await;
                // The provider published cache *writes* for the final round;
                // the shared turn outcome only carries prompt/completion/cached.
                let cache_write = self
                    .usage_observer
                    .as_ref()
                    .map(|observer| observer.cache_write())
                    .unwrap_or(0);
                // Some wires report an all-zero Usage on a degraded/failed
                // round. Persisting that would erase the last real meter and
                // the reopened header would read 0; keep the last good reading
                // until a round actually reports tokens.
                if let Some(reading) = measured_usage(outcome.usage, cache_write) {
                    self.last_usage = Some(reading);
                    self.last_breakdown = self
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.last_breakdown());
                }
                info!(tool_calls = outcome.tool_calls_run, turn, "turn finished");
                if was_prompt {
                    let turn_files = self.turn_files(turn);
                    let reply = self.last_reply();
                    self.queue_distill(TurnRecord {
                        task: outcome_text.clone(),
                        outcome: "success".into(),
                        files: turn_files,
                        correction: denied_tools(&outcome.denials),
                        steered_with: steered_with(&outcome.steers),
                        reply,
                    });
                }
            }
            Err(e) => {
                self.state = AgentState::Failed(e.clone());
                self.hooks.observe(&self.state).await;
                warn!("turn failed: {e}");
                // `run_turn` emits its own display line before every Err —
                // a SystemMessage for cancel, an Error for transport and
                // in-stream failures — plus the terminal `StateChanged(Failed)`.
                // Re-emitting here prints the reminder twice (the cancelled-
                // turn double-line bug).
                if was_prompt {
                    let turn_files = self.turn_files(turn);
                    self.queue_distill(TurnRecord {
                        task: outcome_text.clone(),
                        outcome: "failed".into(),
                        files: turn_files,
                        correction: denied_tools(self.engine.denials_this_turn()),
                        // The engine keeps the turn's steer record across the error
                        // return — a steer followed by a cancel is exactly the
                        // evidence the model pass reads the next time it runs.
                        steered_with: steered_with(self.engine.steers_this_turn()),
                        // The turn produced no answer worth reading back.
                        reply: None,
                    });
                }
            }
        }
        // The turn is done — drop the screenshots it staged. They fed the
        // model this turn; on disk they're just leaked PNGs now (a resumed
        // session would re-shoot rather than trust a stale frame anyway).
        agent_tools_computer::screenshot::cleanup_screenshots(&self.workspace_root, &self.paths);

        // A `computer` grant covers the task it was given for. Ending it here —
        // in the shared host session — means
        // the Tauri overlay, the TUI and the CLI all see the same lifecycle,
        // and a grant can never carry into an unrelated next prompt.
        let had_grant = self
            .permissions_slot
            .write()
            .map(|mut g| g.revoke_grants("computer"))
            .unwrap_or(false);
        if had_grant {
            let _ = self.io.ui_tx.send(UiEvent::SessionToolWhitelisted {
                tool_name: "computer".into(),
                stopped: true,
            });
        }

        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.drain().await;
            checkpoint.end();
        }
        // Commit the engine's authoritative final history over the recovery
        // copy, replacing interrupted-tool placeholders and streamed drafts.
        self.persist_turn(&outcome_text);
        self.turn_started_at = None;
    }

    /// Snapshot `history` to the session store + update sidebar metadata.
    /// `title` derives from the first user prompt; `preview` from the last
    /// agent reply (or "running…" when mid-turn, per product spec).
    fn persist_turn(&mut self, last_task: &str) {
        let Some(store) = &self.store else { return };
        // Sanitize before persisting — strip any `[call:…]` text-protocol
        // echo a provider leaked into assistant content (the streaming
        // CallStripper catches it live, but this is the last-line defense so
        // a poisoned snapshot can't re-teach the model on resume).
        let sanitized: Vec<ChatMessage> = self
            .history
            .iter()
            .map(|m| {
                if m.role == agent_llm::types::Role::Assistant {
                    let mut m = m.clone();
                    if let Some(c) = &m.content {
                        m.content = Some(strip_call_echo(c));
                    }
                    m
                } else {
                    m.clone()
                }
            })
            .collect();

        let Some(meta) = self.session_meta(last_task) else {
            return;
        };
        let expected = self
            .checkpoint
            .as_ref()
            .map_or(self.history_revision, |c| c.revision());
        match store.commit_turn_checked(&sanitized, meta, expected) {
            Ok(revision) => {
                self.history_revision = revision;
                if let Some(checkpoint) = &self.checkpoint {
                    checkpoint.set_revision(revision);
                }
                self.emit_counts();
            }
            Err(error) => {
                let _ = self
                    .io
                    .ui_tx
                    .send(UiEvent::SystemMessage(format!("保存会话失败: {error}")));
            }
        }
    }

    fn session_meta(&self, last_task: &str) -> Option<agent_store::SessionMeta> {
        let store = self.store.as_ref()?;
        let id = self.session_id;
        // Title = first user prompt (truncated); preview = last agent text.
        let title = self
            .history
            .iter()
            .find(|m| m.role == agent_llm::types::Role::User && m.notice.is_none())
            .and_then(|m| m.content.clone())
            .map(|c| c.chars().take(40).collect())
            .unwrap_or_else(|| last_task.chars().take(40).collect());
        let running = self.state.is_active();
        let preview = if running {
            "⟳ running…".to_string()
        } else {
            self.history
                .iter()
                .rev()
                .find(|m| m.role == agent_llm::types::Role::Assistant)
                .and_then(|m| m.content.clone())
                .map(|c| c.chars().take(60).collect())
                .unwrap_or_else(|| "(no reply)".into())
        };
        let old = store.meta(id);
        // A turn's whole life carries one stamp — its submission — so the
        // sidebar order a turn sets never shifts while it streams or when it
        // settles. Outside a turn (`/clear`, `/compact`) the write itself is
        // the activity.
        let updated_at = self.turn_started_at.unwrap_or_else(unix_now);
        // Usage rides along in the sidebar meta: fresh value when this turn
        // produced one, otherwise the previously persisted value survives
        // (a failed turn must not erase the last good reading; a resumed
        // actor's `last_usage` starts empty until its first turn lands).
        let usage = self
            .last_usage
            .map(
                |(prompt, completion, cached, cache_write)| agent_store::SessionUsage {
                    prompt,
                    completion,
                    context_window: self.engine.context_window() as u32,
                    cached,
                    cache_write,
                    breakdown: self.last_breakdown,
                },
            )
            .or_else(|| old.as_ref().and_then(|m| m.usage));
        // The existing meta's pin flag — a turn-boundary upsert must not
        // clobber a pin the user set mid-turn.
        let pinned = old.as_ref().map(|m| m.pinned).unwrap_or(false);
        let (runs, tasks) = history_counts(&self.history);
        // Stamp the live composer settings alongside the usage — the model a
        // turn ran under is the spend's attribution, and the same fields are
        // what `spawn_actor` restores on reopen. These come from the engine's
        // live slots (updated by `apply_model_switch` mid-session), not the
        // spawn-time snapshot.
        let (model, provider) = (self.model.clone(), self.provider_name.clone());
        Some(agent_store::SessionMeta {
            id,
            title,
            preview,
            updated_at,
            pinned,
            usage,
            model: (!model.is_empty()).then_some(model),
            provider: (!provider.is_empty()).then_some(provider),
            permission_mode: Some(self.current_permission_mode()),
            agent_mode: Some(self.engine.agent_mode()),
            thinking_level: self.engine.thinking_level(),
            queued_prompts: self.queued.lock().unwrap().iter().cloned().collect(),
            runs: Some(runs),
            tasks: Some(tasks),
            ..Default::default()
        })
    }

    /// The permission mode the engine is currently dispatching under —
    /// `model_info` labels and `SessionMeta` share this canonical string.
    fn current_permission_mode(&self) -> String {
        self.permissions_slot
            .read()
            .map(|g| g.mode().as_str().to_string())
            .unwrap_or_else(|_| "default".into())
    }

    /// Broadcast the persisted run/task counts. The header treats this as
    /// authoritative — a `/clear`, compaction or retry shrinks the history,
    /// and the view must follow it down, not keep a stale maximum.
    fn emit_counts(&self) {
        let (runs, tasks) = history_counts(&self.history);
        // The turn count rides along: it is the one figure the view cannot
        // derive from live events, since a prompt echo and a steer echo look
        // the same on the wire.
        let turns = self
            .history
            .iter()
            .filter(|m| agent_store::counts_as_turn(m))
            .count();
        let _ = self.io.ui_tx.send(UiEvent::Custom {
            kind: SESSION_COUNTS.into(),
            payload: serde_json::json!({ "runs": runs, "tasks": tasks, "turns": turns }),
        });
    }

    /// Broadcast the current queue to the session's view + mirror it into
    /// `SessionMeta` — the two copies keep a switched-away view and a
    /// reopened session in sync with the actor's deque.
    fn emit_queued(&mut self) {
        let items: Vec<String> = self.queued.lock().unwrap().iter().cloned().collect();
        let _ = self.io.ui_tx.send(UiEvent::QueuedPrompts {
            items: items.clone(),
        });
        if let Some(store) = &self.store {
            if let Err(error) = store.update_queue(self.session_id, items) {
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(format!(
                    "保存待执行任务失败: {error}"
                )));
            }
        }
    }

    /// `UiCommand::SetQueued` — the composer replaces the whole parked
    /// list (enqueue/edit/remove/reorder are all list ops client-side).
    fn set_queued(&mut self, items: Vec<String>) {
        *self.queued.lock().unwrap() = items.into();
        self.emit_queued();
    }

    /// Send parked follow-ups — each pops off the front and runs as a
    /// normal prompt (slash intercept included), one turn at a time, for
    /// as long as the session is idle. Called after any path that can end
    /// a turn; a mid-turn call is a no-op (`is_active` guard).
    ///
    /// A steer that outlived its turn — it landed while the last round was
    /// already sampling, so the engine's between-round drain never saw it —
    /// is still in the channel here and goes first: the user typed it before
    /// anything parked behind it. Left there it would sit until the next
    /// prompt's turn, which is what made 插队 look like it did nothing.
    async fn drain_queue(&mut self) {
        while !self.state.is_active() {
            if self
                .store
                .as_ref()
                .is_some_and(|s| s.is_deleted(self.session_id))
            {
                self.queued.lock().unwrap().clear();
                break;
            }
            let next = match self.io.steer_rx.try_recv() {
                Ok(text) => text,
                Err(_) => {
                    let Some(next) = self.queued.lock().unwrap().pop_front() else {
                        break;
                    };
                    self.emit_queued();
                    next
                }
            };
            self.handle_prompt(next).await;
        }
    }

    /// Persist ONLY this session's composer settings to its own meta row —
    /// called the moment a `SetModel`/`SetPermissionMode`/`SetAgentMode`/
    /// `SetThinkingLevel` lands, so a reopened session restores the choice even
    /// if no turn has run yet (the turn-boundary `persist_turn` also writes
    /// them, but that only fires after a prompt). Other meta fields are read
    /// back and preserved untouched.
    fn persist_settings(&mut self) {
        let Some(store) = &self.store else { return };
        let id = self.session_id;
        let old = store.meta(id);
        // A settings change is not conversation activity: a config reload
        // reaches every live actor at once, and stamping them all "now" would
        // reshuffle the sidebar by whichever actor wrote last.
        let now = old.as_ref().map(|m| m.updated_at).unwrap_or_else(unix_now);
        let (model, provider) = (self.model.clone(), self.provider_name.clone());
        let result = store.update_settings(agent_store::SessionMeta {
            id,
            title: old
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "新会话".into()),
            preview: old.as_ref().map(|m| m.preview.clone()).unwrap_or_default(),
            updated_at: now,
            pinned: old.as_ref().map(|m| m.pinned).unwrap_or(false),
            // Same rule as a turn-boundary write: the actor's own reading wins
            // once a turn has produced one — a model switch trims its cache
            // half — while a resumed actor that has not run yet keeps the
            // stored value.
            usage: self
                .last_usage
                .map(
                    |(prompt, completion, cached, cache_write)| agent_store::SessionUsage {
                        prompt,
                        completion,
                        context_window: self.engine.context_window() as u32,
                        cached,
                        cache_write,
                        breakdown: self.last_breakdown,
                    },
                )
                .or_else(|| old.as_ref().and_then(|m| m.usage)),
            model: (!model.is_empty()).then_some(model),
            provider: (!provider.is_empty()).then_some(provider),
            permission_mode: Some(self.current_permission_mode()),
            agent_mode: Some(self.engine.agent_mode()),
            thinking_level: self.engine.thinking_level(),
            queued_prompts: self.queued.lock().unwrap().iter().cloned().collect(),
            runs: old.as_ref().and_then(|m| m.runs),
            tasks: old.as_ref().and_then(|m| m.tasks),
            ..Default::default()
        });
        if let Err(error) = result {
            let _ = self
                .io
                .ui_tx
                .send(UiEvent::SystemMessage(format!("保存会话设置失败: {error}")));
        }
    }

    /// Session identity — the sidebar/store key.
    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Read-only history snapshot (persistence + multi-session handoff).
    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    /// Hand the turn's record to the background distiller — never blocks
    /// the turn's completion path.
    fn queue_distill(&mut self, record: TurnRecord) {
        if let Some(d) = &self.distiller {
            d.spawn_distill(record);
        }
    }

    /// Paths the given turn's tools wrote, as the distiller wants them.
    fn turn_files(&self, turn: u32) -> Vec<String> {
        self.hunks
            .lock()
            .files_in_turn(turn)
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// The turn's final assistant text, capped — the distiller's evidence of
    /// what actually happened. Without it the memory pass only ever saw the
    /// user's ask, and had no way to tell "the project's convention is X" from
    /// "the user reported a bug".
    fn last_reply(&self) -> Option<String> {
        self.history
            .iter()
            .rev()
            .filter(|m| m.role == agent_llm::types::Role::Assistant)
            .filter_map(|m| m.content.as_deref())
            .find(|text| !text.trim().is_empty())
            .map(|text| text.chars().take(REPLY_CAP).collect())
    }

    /// Drop the recall bookkeeping for rows at or past `len`. A rewind
    /// (`/retry`, `/clear`, `/compact`) takes the recall rows with the prompt,
    /// so those facts become recallable again. The observation ledger goes
    /// whole: the tool results it vouches for may have been in the cut.
    fn forget_after(&mut self, len: usize) {
        self.injected_facts.retain(|(at, _)| *at < len);
        self.observations.clear();
    }

    /// Drop the recall rows exhausted turns left behind — see
    /// [`prune_recall_rows_in`].
    fn prune_recall_rows(&mut self, keep: usize) {
        prune_recall_rows_in(&mut self.history, &mut self.injected_facts, keep);
    }

    /// Reverse-apply the last turn's writes via the write tracker — restores
    /// each touched file to its pre-turn state (or deletes it if created).
    async fn undo_last_turn(&mut self) {
        self.warming.cancel();
        // The tracker lock is held only while planning — never across an await.
        let (last, ops, skipped) = {
            let mut hunks = self.hunks.lock();
            let last = hunks.current_turn();
            // Verify-on-undo: nothing feeds the watcher path in production, so
            // compare the turn's files against the agent's last recorded write
            // before building the plan — otherwise a user edit made after the
            // agent's write is silently overwritten.
            hunks.mark_external_if_drifted(last, |path| std::fs::read(path).ok());
            // P0-C2: undo_plan now refuses when a target file was modified
            // externally after the agent's last write — undoing would silently
            // clobber the user's own edits. Fall back to partial undo so the
            // rest still reverts, and surface which files were skipped.
            let (ops, skipped) = match hunks.undo_plan(last) {
                Ok(ops) => (ops, Vec::new()),
                Err(_) => hunks.undo_plan_partial(last),
            };
            (last, ops, skipped)
        };
        let _ = last;
        if ops.is_empty() && skipped.is_empty() {
            let line = "没有可回滚的修改".to_string();
            let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
            self.history.push(ChatMessage::notice(line));
            return;
        }
        for path in &skipped {
            let line = format!(
                "已跳过 `{}` — 在 agent 写入后被外部修改（回滚会覆盖你的改动）",
                path.display()
            );
            let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
            self.history.push(ChatMessage::notice(line));
        }
        let mut reverted = 0usize;
        for op in ops {
            // Safety: never undo outside the workspace or a linked workspace
            // that granted writes — the tracker records resolved paths, but
            // guard here so a bad origin can't delete arbitrary files.
            if !self.undo_reaches(&op.path) {
                warn!(path = %op.path.display(), "undo skipped: outside the granted roots");
                continue;
            }
            match &op.restore_to {
                Some(content) => {
                    if tokio::fs::write(&op.path, content).await.is_ok() {
                        reverted += 1;
                    }
                }
                None => {
                    if tokio::fs::remove_file(&op.path).await.is_ok() {
                        reverted += 1;
                    }
                }
            }
        }
        let line = format!("已回滚第 {last} 轮修改的 {reverted} 个文件");
        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
        self.history.push(ChatMessage::notice(line));
    }

    /// Point the engine at `provider`/`model` — rebuild the provider, then
    /// re-resolve everything config declares per model (context window, input
    /// modalities, thinking-level map, and the per-model wire settings).
    ///
    /// `modelOverrides` are folded in via `ProviderConfig::model_opts`, so an
    /// override declared for this id applies to a hot-swap exactly as it does
    /// at session spawn. A model the config doesn't describe resets to the
    /// built-in defaults instead of leaking the previous model's caps.
    fn apply_model_switch(&mut self, provider: &str, model: &str) {
        let model_changed = self.model != model || self.provider_name != provider;
        // What the distillation pass will run on after this switch. It mirrors
        // the engine exactly: a buildable provider moves provider, model and
        // params together; otherwise only the model name moves, as
        // `engine.set_model` does below, and the old params stay with the old
        // provider instead of being reset under it.
        let mut next_distill: Option<DistillCtx> = None;
        if let Ok(cfg) = agent_llm::AppConfig::load(&self.paths.config_dir, None) {
            // Every provider's key is scrubbed, not just the active one: a
            // prompt can carry any configured key, and a switch must not leave
            // the previous provider's mask in place.
            self.engine.set_secrets(cfg.resolved_secrets());
            if let Some(pcfg) = cfg.providers.get(provider) {
                // Same rule as spawn: an unbuildable provider is replaced by one
                // that fails with the reason. Keeping the previous provider here
                // would answer under the new model's limits and thinking map.
                let (built, _unavailable) = crate::session_manager::choose_session_provider(
                    &cfg,
                    provider,
                    self.usage_observer.as_ref(),
                    Some(&self.conversation),
                );
                // The old model's warm entry is no use to the new one.
                self.warming.cancel();
                // Warming is side traffic: on its own instance, so a replay
                // cannot overwrite the foreground round's cache-write figure.
                let side = unobserved_provider_from(&cfg, provider, &self.conversation)
                    .unwrap_or_else(|| built.clone());
                let resolved = pcfg
                    .find_model(model)
                    .is_some()
                    .then(|| pcfg.model_opts(model));
                let warmed = warmed_provider(
                    built.clone(),
                    side.clone(),
                    model,
                    resolved.as_ref(),
                    &self.warming,
                );
                let parent_provider = self.checkpoint.as_ref().map_or_else(
                    || warmed.clone(),
                    |checkpoint| checkpoint.wrap(warmed.clone()),
                );
                self.engine.set_provider(parent_provider);
                self.set_fallbacks(
                    provider,
                    crate::session_manager::build_fallback_chain(
                        &cfg,
                        provider,
                        self.usage_observer.as_ref(),
                        Some(&self.conversation),
                    ),
                );
                // Post-turn distillation follows the model in use: same side
                // instance (no usage observer), so the memory pass reads the
                // session's own dialect and never moves its cache counter.
                next_distill = Some(DistillCtx {
                    provider: side.clone(),
                    model: model.to_string(),
                    params: resolved
                        .as_ref()
                        .map(agent_llm::ProviderFactory::model_params_from)
                        .unwrap_or(agent_llm::ModelParams::EMPTY),
                });
                // Children are routed from this base, so it must match what the
                // parent just got — but without the foreground usage observer
                // (the same instance `side` already is).
                let route = &mut self.child_base;
                route.provider = side;
                match resolved {
                    Some(d) => {
                        let map = d.thinking_level_map.clone();
                        let window = d.context_window.unwrap_or(256_000) as usize;
                        let input = d.input.clone();
                        let params = agent_llm::ProviderFactory::model_params_from(&d);
                        self.engine.set_thinking_level_map(map.clone());
                        self.engine.set_context_window(window);
                        self.engine.set_model_input(input.clone());
                        self.engine.set_model_params(params.clone());
                        route.thinking_level_map = map;
                        route.context_window = window;
                        route.model_input = input;
                        route.model_params = params;
                    }
                    None => {
                        self.engine.set_thinking_level_map(None);
                        self.engine.set_context_window(256_000);
                        self.engine.set_model_input(Vec::new());
                        self.engine.set_model_params(agent_llm::ModelParams::EMPTY);
                        route.thinking_level_map = None;
                        route.context_window = 256_000;
                        route.model_input = Vec::new();
                        route.model_params = agent_llm::ModelParams::EMPTY;
                    }
                }
            }
        }
        // The parent's model changes unconditionally, so the children's base must too.
        self.child_base.model = model.to_string();
        self.engine.set_model(model.to_string());
        // Keep the actor's own identity fields in step — they feed both the
        // usage attribution on `persist_turn` and the `SessionMeta` row a
        // reopened session restores from. Letting them lag behind the engine
        // recorded a stale model against the spend.
        self.model = model.to_string();
        self.provider_name = provider.to_string();
        self.last_usage = cache_reading_after_switch(self.last_usage, model_changed);
        // The distiller is rebuilt with the session, not left behind: a
        // memory pass running on the previous model is a second, invisible
        // bill — and the one thing the settings 统计 pane then cannot explain.
        if let Some(ctx) = next_distill {
            self.distill_ctx = ctx;
        } else {
            // The engine's own rule when the config cannot build the provider:
            // the model name moves, provider and params stay as they were.
            self.distill_ctx.model = model.to_string();
        }
        self.distiller = build_distiller(
            self.memory.as_ref(),
            self.memory_distill,
            &self.distill_ctx,
            self.distill_record.clone(),
        );
        if let Ok(mut slot) = self.model_slot.write() {
            *slot = (provider.to_string(), model.to_string());
        }
        self.refresh_child_route();
    }

    /// Set the global subagent default `(provider, model)` — `None` lets
    /// children follow this session's model again. Applies from the next
    /// `delegate` call; a child already running keeps its model.
    pub fn set_subagent_model(&mut self, model: Option<(String, String)>) {
        self.subagent_model = model;
        self.refresh_child_route();
    }

    /// Global thinking level for delegated children — `None` follows this
    /// session's own level, and a manifest `thinking:` still wins. Applies
    /// from the next `delegate` call; a child already running keeps its level.
    pub fn set_subagent_thinking(&mut self, level: Option<String>) {
        self.subagent_thinking = level.filter(|l| !l.trim().is_empty());
        self.refresh_child_route();
    }

    /// Rebuild the shared child route from the parent's base, the current
    /// config and the subagent default.
    fn refresh_child_route(&mut self) {
        let route = routed_child(
            &self.child_base,
            &self.paths.config_dir,
            &self.provider_name,
            self.subagent_model.as_ref(),
            self.subagent_thinking.as_deref(),
            &self.conversation,
        );
        *self.child_route.write().expect("child route lock poisoned") = route;
    }

    /// Execute a `ControlOp` from a slash command — session state changes
    /// with no LLM round-trip.
    async fn run_control(&mut self, op: ControlOp) {
        match op {
            ControlOp::ClearHistory => {
                self.history.truncate(1); // keep the system prompt
                self.forget_after(1);
                self.warming.cancel();
                let line = "会话历史已清空".to_string();
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
                self.persist_turn("已清空历史");
            }
            ControlOp::Compact => {
                // `/compact` bypasses the threshold AND the suppressor —
                // the user asked for it now. (The old arm only *reported* a
                // threshold and never compacted, which read as "the button
                // does nothing".)
                // Publish the in-flight phase around the pass: the summary
                // sample can take a while on a big history, and without a
                // state the UI showed nothing at all until the card landed.
                // Restore the pre-command phase afterwards (Idle/Finished —
                // a manual compact never runs inside a turn).
                let prev_state = self.state.clone();
                let _ = self
                    .io
                    .ui_tx
                    .send(UiEvent::StateChanged(AgentState::Compacting));
                // The summary rewrites the history the warm entry was keyed on.
                self.warming.cancel();
                // Stale recall rows are the cheapest thing to lose, and a
                // compaction is the one moment a rewrite is free: the facts they
                // carried are already in the transcript.
                self.prune_recall_rows(2);
                let result = self
                    .engine
                    .compact_now(&mut self.io, &mut self.history)
                    .await;
                let _ = self.io.ui_tx.send(UiEvent::StateChanged(prev_state));
                match result {
                    Ok(agent_kernel::engine::CompactOutcome::Compacted(outcome)) => {
                        // The engine already emitted the card event and
                        // pushed the persisted display row. Re-base the
                        // persisted meter on the post-splice figure, then
                        // snapshot NOW — without this the raw-JSON viewer
                        // and a reopened session both replayed the
                        // pre-compaction history, so compaction read as a
                        // no-op until the next turn happened to land.
                        // Cached prompt tokens describe the pre-compaction
                        // request body — zero them with it, or the reopened
                        // meter shows a cache reading for a context that no
                        // longer exists.
                        self.last_usage = Some(match self.last_usage {
                            Some((_, completion, _, _)) => {
                                (outcome.after_tokens as u32, completion, 0, 0)
                            }
                            None => (outcome.after_tokens as u32, 0, 0, 0),
                        });
                        // The split described the pre-compaction request.
                        self.last_breakdown = None;
                        // The summariser dropped the recall rows it folded in.
                        let len = self.history.len();
                        self.forget_after(len);
                        self.persist_turn("已压缩上下文");
                    }
                    Ok(agent_kernel::engine::CompactOutcome::NothingToDo) => {
                        // Not a failure: the history already fits the
                        // verbatim suffix budget. Report it neutrally so a
                        // second `/compact` doesn't read as an error.
                        let line = "当前上下文已足够小，无需压缩".to_string();
                        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                        self.history.push(ChatMessage::notice(line));
                    }
                    Err(e) => {
                        let line = format!("上下文压缩失败: {e}");
                        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                        self.history.push(ChatMessage::notice(line));
                    }
                }
            }
            ControlOp::UndoLastTurn => self.undo_last_turn().await,
            ControlOp::SetModel { provider, model } => {
                info!(%provider, %model, "slash hot-swap");
                self.apply_model_switch(&provider, &model);
                self.persist_settings();
                let line = format!("已切换模型至: {model} ({provider}) — 下一轮生效");
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
        }
    }

    /// `/x` → plugin `tool:<name>` — dispatch through the live plugin
    /// router. The slash remainder is typed text, not JSON: a bare word
    /// becomes `{"text": args}` (the common one-arg-tool convention), a
    /// JSON object is used verbatim, and empty means `{}`.
    async fn run_plugin_command(&mut self, tool: &str, name: &str, args: &str) {
        let Some(mgr) = self
            .plugins
            .as_ref()
            .and_then(|h| h.read().ok().and_then(|g| g.clone()))
        else {
            let line = "plugin router unavailable — no plugins loaded".to_string();
            let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
            self.history.push(ChatMessage::notice(line));
            return;
        };
        let trimmed = args.trim();
        let parsed: serde_json::Value = if trimmed.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::json!({ "text": trimmed }))
        };
        let line = match mgr.dispatch_tool_call(tool, parsed).await {
            // Frame the result — a bare SystemMessage that echoes the args
            // reads as "nothing happened" (e.g. a slugifier whose output
            // equals its input). `/{name}` attributes it to the command.
            // An empty result still gets labelled — `→ ` alone looks like a
            // render bug instead of "the tool returned nothing".
            Ok(out) if out.trim().is_empty() => format!("`/{name}` → (空输出)"),
            Ok(out) => format!("`/{name}` → {out}"),
            Err(e) => format!("`/{name}` plugin call failed: {e}"),
        };
        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
        self.history.push(ChatMessage::notice(line));
    }

    /// `UiCommand::Prompt` / `UiCommand::Retry` shared path — slash-command
    /// intercept + the `on_user_input` hook chain, then `run_prompt`. Kept
    /// out of the `match` so `Retry` can re-enter it without recursion.
    async fn handle_prompt(&mut self, text: String) {
        if self
            .store
            .as_ref()
            .is_some_and(|s| s.is_deleted(self.session_id))
        {
            return;
        }
        // `/x` slash commands intercept before the ReAct loop — zero
        // tokens. Unknown `/x` falls through as a prompt.
        let workspace_root = self.workspace_root.clone();
        // Snapshot the enabled plugins' declared commands — the live handle
        // is shared, so a mid-session `reload_plugins` reaches `/x` dispatch.
        let plugin_commands = self
            .plugins
            .as_ref()
            .and_then(|h| h.read().ok().and_then(|g| g.clone()))
            .map(|m| m.commands())
            .unwrap_or_default();
        let cmd_result = {
            let linked = self.linked_roots.snapshot();
            let mut ctx = CommandCtx {
                history: &mut self.history,
                permission_mode: "default",
                workspace_root: &workspace_root,
                linked_roots: &linked,
                memory: self.memory.as_deref(),
                plugin_commands: &plugin_commands,
                ui_tx: &self.io.ui_tx,
                blueprint_enabled: self.architecture.get(),
            };
            CommandRegistry::try_run(&text, &mut ctx).await
        }; // ctx dropped here — the history/ui_tx borrows end
        match cmd_result {
            Some(CommandResult::Reply(r)) => {
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(r.clone()));
                self.history.push(ChatMessage::notice(r));
            }
            Some(CommandResult::Control(op)) => {
                self.run_control(op).await;
            }
            Some(CommandResult::PluginTool { tool, name, args }) => {
                self.run_plugin_command(&tool, &name, &args).await;
            }
            Some(CommandResult::FeedToAgent(prompt)) => {
                self.run_prompt(prompt).await;
            }
            None => {
                // Hook chain: on_user_input may block/rewrite.
                match self.hooks.run_on_user_input(&text).await {
                    agent_kernel::hooks::HookAction::BlockTurn(reason) => {
                        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(reason.clone()));
                        self.history.push(ChatMessage::notice(reason));
                    }
                    agent_kernel::hooks::HookAction::InjectSystemNote(note) => {
                        self.history.push(ChatMessage::system(note));
                        self.run_prompt(text).await;
                    }
                    agent_kernel::hooks::HookAction::MutateMessages(msgs) => {
                        self.history = msgs;
                        let len = self.history.len();
                        self.forget_after(len);
                        self.run_prompt(text).await;
                    }
                    agent_kernel::hooks::HookAction::Continue => {
                        self.run_prompt(text).await;
                    }
                }
            }
        }
    }

    /// Consume a `UiCommand`. `Prompt` runs a turn to completion; `Steer`
    /// lands mid-turn via the engine's drain; the rest are no-ops until their
    /// owning stages land.
    pub async fn handle(&mut self, cmd: UiCommand) {
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.start();
        }
        self.handle_command(cmd).await;
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.drain().await;
        }
    }

    async fn handle_command(&mut self, cmd: UiCommand) {
        match cmd {
            UiCommand::Prompt { text } => {
                self.handle_prompt(text).await;
                self.drain_queue().await;
            }
            // Routed around this pump by the IPC layer, the same as `Steer`. Queued
            // here it would wait behind the turn it is meant to interrupt, so it is
            // never handled on this path.
            UiCommand::SteerChild { .. } => {}
            UiCommand::Retry => {
                // Regenerate the last turn: rewind `history` to just before
                // its user prompt and re-run the text through the full
                // Prompt path (slash intercept + hooks), so a retried `/x`
                // behaves exactly like a hand-typed resend. `TurnRetry`
                // tells the UI to drop the old turn's items before the
                // fresh `UserPrompt` echo lands. Idle-only — a live turn
                // owns the prompt slot and a pending approval/question is
                // mid-flight state a rewind can't safely discard.
                if self.state.is_active() {
                    return;
                }
                let idx = self.history.iter().rposition(|m| {
                    // Same rule the store counts turns by: a hidden recall row
                    // and a mid-turn steer are the turn's furniture, not its
                    // opening prompt, so neither can be the rewind target.
                    agent_store::counts_as_turn(m)
                });
                if let Some(idx) = idx {
                    let text = self.history[idx].content.clone().unwrap_or_default();
                    if text.is_empty() {
                        return;
                    }
                    info!(history_idx = idx, "retry — rewinding to last user prompt");
                    // The prompt's recall row goes with it — the resend
                    // recalls afresh, and two rows would both reach the model.
                    let idx = if idx > 0 && is_memory_context(&self.history[idx - 1]) {
                        idx - 1
                    } else {
                        idx
                    };
                    self.history.truncate(idx);
                    self.forget_after(idx);
                    self.warming.cancel();
                    let _ = self.io.ui_tx.send(UiEvent::TurnRetry);
                    self.handle_prompt(text).await;
                    self.drain_queue().await;
                }
            }
            UiCommand::Steer { text } => {
                info!(len = text.len(), "steer command");
                if self.state.is_active() {
                    // Mid-turn: forward into the engine's steer channel —
                    // drain_steering picks it up between tool calls.
                    let _ = self.steer_tx.try_send(text);
                } else {
                    // Between turns a Steer is a fresh Prompt.
                    self.run_prompt(text).await;
                    self.drain_queue().await;
                }
            }
            UiCommand::SetQueued { items } => {
                self.set_queued(items);
            }
            UiCommand::SetCompactAt { fraction } => {
                // Workspace-wide default change from the settings page —
                // applied to the live engine so the new trigger takes effect
                // on the next round instead of waiting for a respawn.
                info!(fraction, "compaction ratio change requested");
                self.engine.set_compact_at(fraction);
            }
            UiCommand::Enqueue { text } => {
                // Direct cmd_tx path (tests, internal callers). The app's IPC
                // layer pushes to the shared deque itself and sends
                // `DrainQueue` — see `queue_writer`.
                self.queued.lock().unwrap().push_back(text);
                self.emit_queued();
                // Idle + enqueue = send now — covers the race where the UI
                // still showed streaming when the user hit queue but the
                // turn had already finished.
                self.drain_queue().await;
            }
            UiCommand::DrainQueue => {
                // The deque already changed via the shared endpoint — this
                // only schedules the drain for when the pump is free.
                self.drain_queue().await;
            }
            UiCommand::SetModel { provider, model } => {
                info!(%provider, %model, "hot-swap requested");
                self.apply_model_switch(&provider, &model);
                self.persist_settings();
                let line = format!("已切换模型至: {model} ({provider})");
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            UiCommand::ReloadModel => {
                // A config write re-reads `config.toml` into every live actor,
                // so an edited model takes effect on the next turn instead of
                // on the next launch. The session reloads ITS OWN pair —
                // carrying the manager's mirror once re-pointed every actor
                // at the active session's model. Silent: no history notice.
                let (provider, model) = (self.provider_name.clone(), self.model.clone());
                info!(%provider, %model, "model config reload");
                self.apply_model_switch(&provider, &model);
                self.persist_settings();
            }
            UiCommand::SetSubagentModel { provider, model } => {
                let pair = (!provider.is_empty() && !model.is_empty()).then_some((provider, model));
                self.set_subagent_model(pair);
            }
            UiCommand::SetSubagentThinking { level } => {
                self.set_subagent_thinking(Some(level));
            }
            UiCommand::SetThinkingLevel { level } => {
                info!(%level, "thinking level change requested");
                self.engine.set_model_thinking_level(if level.is_empty() {
                    None
                } else {
                    Some(level.clone())
                });
                self.persist_settings();
                let actual = self.engine.thinking_level().unwrap_or_else(|| "off".into());
                let line = format!("已设置思考推理强度: {actual}");
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            UiCommand::SetPermissionMode { mode } => {
                info!(%mode, "permission mode switch");
                // Switch the mode IN the live gate so a "本会话允许" grant
                // survives it (rebuilding the gate wiped the rules).
                let slot = self.engine.permissions_writer();
                let parsed = agent_kernel::permissions::PermissionMode::from_str(&mode);
                if let Ok(mut gate) = slot.write() {
                    gate.set_mode(parsed);
                }
                self.persist_settings();
                let line = format!("权限模式已切换为: {}", parsed.label());
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
                // The prompt carries the gate's mode too — a stale copy would
                // tell the model it is judged by rules the gate no longer has.
                self.rewrite_prompt_block("permission", &permission_block(parsed.as_str()));
            }
            UiCommand::SetBlueprintEnabled { enabled } => {
                self.set_blueprint_enabled(enabled);
            }
            UiCommand::SetAgentMode { mode } => {
                let new = crate::mode::AgentMode::from_str(&mode);
                self.engine.set_agent_mode(new.as_str());
                self.persist_settings();
                // Rewrite the mode block inside the rendered system prompt —
                // the contract text must match the registry the next round
                // actually dispatches against. Located by marker, not by the
                // old text: a session resumed from an older build carries
                // wording this build would never match.
                self.rewrite_prompt_block("mode", new.prompt_block());
                let label = match new {
                    crate::mode::AgentMode::Build => "构建",
                    crate::mode::AgentMode::Plan => "计划",
                    crate::mode::AgentMode::Goal => "目标",
                    crate::mode::AgentMode::Office => "办公",
                };
                let line = format!("模式已切换为: {label}");
                let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            UiCommand::Cancel => {
                // P0-C4: set the shared flag the engine polls between chunks
                // and before each tool dispatch. This pump path covers the
                // between-turns case; mid-turn cancels arrive via
                // `cancel_writer()`, which isn't queued behind `run_turn`.
                self.cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                // Wake a parked ask_question too — its exec polls the flag,
                // but dropping the oneshot fails it fast either way.
                self.ask_channel.clear();
                self.state = AgentState::Finished;
                self.hooks.observe(&self.state).await;
                let _ = self
                    .io
                    .ui_tx
                    .send(UiEvent::SystemMessage("已被用户中断".into()));
                self.history.push(ChatMessage::notice("已被用户中断"));
            }
            UiCommand::AnswerQuestion { request_id, answer } => {
                // Fallback path — commands.rs normally resolves the channel
                // directly; this arm covers test-driven actors.
                self.ask_channel.answer(request_id, answer);
            }
            UiCommand::ToolDecision {
                request_id,
                approved,
                action,
            } => {
                // Direct write to the engine's shared decision slot — reaches
                // the wait even when handle() runs mid-turn via run()'s
                // serial pump (the turn future polls the slot, not this
                // channel). The request_id correlates the verdict to a
                // specific ApprovalRequested (P1-a).
                *self.decision_slot.lock().unwrap() = Some((
                    request_id,
                    agent_kernel::tools::registry::ToolVerdict { approved, action },
                ));
            }
            UiCommand::ApproveSessionTool { tool_name } => {
                // Computer-use (and friends) opt into per-session
                // whitelisting so a single approval unblocks the rest of
                // the turn's actions. The flag lives on the gate — dies
                // with the actor, never persisted.
                if let Ok(mut gate) = self.permissions_slot.write() {
                    gate.allow_tool_for_session(&tool_name);
                }
                let _ = self.io.ui_tx.send(UiEvent::SessionToolWhitelisted {
                    tool_name,
                    stopped: false,
                });
            }
            UiCommand::RevokeSessionTool { tool_name } => {
                // The overlay's "停止" path — the whitelist drops the
                // entry so the NEXT `computer` dispatch goes back to Ask.
                // A running call isn't retro-cancelled; the next one
                // re-prompts, which is exactly what the user asked for
                // when they clicked stop.
                if let Ok(mut gate) = self.permissions_slot.write() {
                    gate.revoke_tool_for_session(&tool_name);
                }
                let _ = self.io.ui_tx.send(UiEvent::SessionToolWhitelisted {
                    tool_name,
                    stopped: true,
                });
            }
            UiCommand::UndoLastTurn => {
                self.undo_last_turn().await;
            }
            UiCommand::SetMemory { enabled, distill } => {
                self.memory_distill = distill;
                if !enabled {
                    // Off means fully off — no recall block, no writes, and no
                    // `remember` either: the tools read the shared handle.
                    self.memory = None;
                    self.memory_handle.set(None);
                    self.distiller = None;
                    return;
                }
                if self.memory.is_none() {
                    let Some(s) = open_memory_store(&self.workspace_root) else {
                        let line = "记忆功能打开失败：无法打开 memory.db".to_string();
                        let _ = self.io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                        self.history.push(ChatMessage::notice_error(line));
                        return;
                    };
                    self.memory = Some(Arc::new(s));
                    self.memory_handle.set(self.memory.clone());
                }
                self.distiller = build_distiller(
                    self.memory.as_ref(),
                    self.memory_distill,
                    &self.distill_ctx,
                    self.distill_record.clone(),
                );
            }
        }
    }
}

/// Unix seconds, 0 when the clock reads before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `chrono`-free date stamp for the prompt — good enough for "today" context
/// without pulling a date crate into the kernel's dep graph.
fn chrono_lite_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // days since epoch → civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Remove `[call: name(args)]` / `[call: ()]` text-protocol tool-call echoes
/// from a whole assistant message (the non-streaming form of the adapter's
/// CallStripper — applied to persisted history on resume so a poisoned
/// snapshot can't re-teach the model). Drops each `[call:…]` run plus one
/// leading/trailing newline so no blank-line artifact remains.
pub(crate) fn strip_call_echo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[call:") {
        // Emit text before the call, trimming a directly-preceding newline.
        let before = &rest[..start];
        out.push_str(before.strip_suffix('\n').unwrap_or(before));
        let after = &rest[start..];
        // Skip the `[call:…]` run to the closing `]` (or end of input).
        let end = after.find(']').map(|i| i + 1).unwrap_or(after.len());
        rest = &after[end..];
        // Swallow one newline right after the call line.
        if let Some(stripped) = rest.strip_prefix('\n') {
            rest = stripped;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Header counts follow the CURRENT history: assistant rows are model
    /// runs, tool rows are task steps; notices and user rows count as neither.
    #[test]
    fn history_counts_tally_assistant_and_tool_rows() {
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("go"),
            ChatMessage::assistant("calling"),
            ChatMessage::tool_result("c1", "ok"),
            ChatMessage::tool_result("c2", "ok"),
            ChatMessage::assistant("done"),
            ChatMessage::notice("note"),
        ];
        assert_eq!(history_counts(&history), (2, 2));
        assert_eq!(history_counts(&history[..2]), (0, 0));
    }

    /// An all-zero Usage is not a measurement and must not replace the last
    /// real meter; any non-zero field (cache write included) is.
    #[test]
    fn all_zero_usage_is_not_a_reading() {
        assert_eq!(measured_usage(None, 0), None);
        assert_eq!(measured_usage(Some((0, 0, 0)), 0), None);
        assert_eq!(measured_usage(Some((0, 0, 0)), 5), Some((0, 0, 0, 5)));
        assert_eq!(measured_usage(Some((10, 2, 8)), 0), Some((10, 2, 8, 0)));
    }

    /// Parent and child bases share the host tools; the child only differs by
    /// the parent's restricted capabilities. Guards against a host tool landing
    /// in one registry and not the other.
    #[test]
    fn base_registries_carry_the_same_host_tools() {
        let (full, child) = base_registries();
        for spec in host_tools() {
            assert!(
                full.spec(spec.name).is_some(),
                "{} missing in parent",
                spec.name
            );
            if spec.name != "computer" && spec.class != ToolClass::HumanInteraction {
                assert!(
                    child.spec(spec.name).is_some(),
                    "{} missing in child",
                    spec.name
                );
            }
        }
        for stripped in ["delegate", "remember", "computer"] {
            assert!(full.spec(stripped).is_some());
            assert!(
                child.spec(stripped).is_none(),
                "child must not hold {stripped}"
            );
        }
    }

    /// The bundled template must keep the markers — without them the per-turn
    /// refresh silently degrades to the legacy literal swap (this is exactly
    /// the bug these tests guard).
    #[test]
    fn template_carries_the_memory_markers() {
        assert!(SYSTEM_PROMPT_TEMPLATE.contains("<!-- memory -->"));
        assert!(SYSTEM_PROMPT_TEMPLATE.contains("<!-- /memory -->"));
    }

    /// Prompt-cache layout: every session- or turn-dependent placeholder sits
    /// after the fixed instructions, so the instructions stay a byte-stable
    /// prefix across sessions and across mode/permission changes.
    #[test]
    fn template_keeps_volatile_regions_after_the_fixed_prefix() {
        let split = SYSTEM_PROMPT_TEMPLATE
            .find("## Session context")
            .expect("session context section");
        let (fixed, session) = SYSTEM_PROMPT_TEMPLATE.split_at(split);
        assert!(!fixed.contains("{{"), "placeholder in the fixed prefix");
        for ph in [
            "{{DATE}}",
            "{{WORKSPACE_ROOT}}",
            "{{PERMISSION_MODE}}",
            "{{WORKSPACE_TREE}}",
            "{{GIT_STATUS}}",
            "{{SKILLS_BLOCK}}",
            "{{SUBAGENTS_BLOCK}}",
            "{{BLUEPRINT_BLOCK}}",
            "{{MEMORY_BLOCK}}",
            "{{AGENT_MODE}}",
        ] {
            assert!(session.contains(ph), "{ph} missing");
        }
        // The regions a live session rewrites carry markers: content-matched
        // replacement goes stale on a prompt resumed from an older build.
        for marker in [
            ("<!-- permission -->", "<!-- /permission -->"),
            ("<!-- blueprint -->", "<!-- /blueprint -->"),
            ("<!-- memory -->", "<!-- /memory -->"),
            ("<!-- mode -->", "<!-- /mode -->"),
        ] {
            assert!(session.contains(marker.0), "{} missing", marker.0);
            assert!(session.contains(marker.1), "{} missing", marker.1);
        }
        // The mode block is last, and it is what a mode switch rewrites.
        assert!(session.contains("<!-- mode -->\n{{AGENT_MODE}}\n<!-- /mode -->"));
        assert!(session.trim_end().ends_with("<!-- /mode -->"));
    }

    /// A live mode or permission switch must not depend on the wording it
    /// replaces: a resumed session carries whatever the build that spawned it
    /// wrote. `replace_block` locates its region by marker for exactly that.
    #[test]
    fn rewrites_regions_by_marker_not_by_content() {
        let prompt = "a\n<!-- mode -->\nMode: build — old wording\n<!-- /mode -->\nb";
        let out = replace_block(prompt, "mode", "Mode: plan — new");
        assert!(out.contains("Mode: plan — new"), "{out}");
        assert!(!out.contains("old wording"), "{out}");
        assert!(out.starts_with("a\n<!-- mode -->"), "{out}");
        assert!(out.ends_with("<!-- /mode -->\nb"), "{out}");
        // No marker → untouched, never a half-mangled prompt.
        assert_eq!(replace_block("no markers", "mode", "x"), "no markers");
    }

    /// The permission region renders the gate's canonical wire label, so the
    /// prompt and the gate name the same mode.
    #[test]
    fn permission_block_names_the_wire_label() {
        let block = permission_block(agent_kernel::permissions::PermissionMode::Bypass.as_str());
        assert!(
            block.starts_with("- Permission mode: bypassPermissions"),
            "{block}"
        );
    }

    #[test]
    fn memory_context_rows_are_recognised() {
        let row =
            ChatMessage::user_hidden(format!("{MEMORY_CONTEXT_OPEN}\nfact\n</memory-context>"));
        assert!(is_memory_context(&row));
        assert!(!is_memory_context(&ChatMessage::user("<memory-context>")));
    }

    /// A compaction drops the recall rows of turns that are already history,
    /// with their bookkeeping, and leaves the newest one alone.
    #[test]
    fn pruning_recall_rows_keeps_the_newest_and_reindexes_the_bookkeeping() {
        let mut history = vec![
            ChatMessage::system("s"),
            ChatMessage::user_hidden(format!("{MEMORY_CONTEXT_OPEN}\na")),
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user_hidden(format!("{MEMORY_CONTEXT_OPEN}\nb")),
            ChatMessage::user("q2"),
            ChatMessage::assistant("a2"),
            ChatMessage::user_hidden(format!("{MEMORY_CONTEXT_OPEN}\nc")),
            ChatMessage::user("q3"),
        ];
        let mut facts = vec![(2, 10), (5, 20), (8, 30)];

        prune_recall_rows_in(&mut history, &mut facts, 1);

        assert_eq!(history.iter().filter(|m| is_memory_context(m)).count(), 1);
        // The surviving row's prompt moved from 8 to 6 — the fact went with it.
        assert_eq!(facts, vec![(6, 30)]);
        assert_eq!(history[6].content.as_deref(), Some("q3"));
    }

    /// Spawn and per-turn both route through this: the region is swapped
    /// verbatim, the rest of the prompt is untouched, and the placeholder
    /// never survives.
    #[test]
    fn refreshes_the_marked_region() {
        let prompt = "head\n<!-- memory -->\n{{MEMORY_BLOCK}}\n<!-- /memory -->\ntail";
        let first = set_memory_block(prompt, "── recent episodes ──\n• did a thing [ok]");
        assert!(!first.contains("{{MEMORY_BLOCK}}"), "{first}");
        assert!(first.contains("• did a thing [ok]"), "{first}");
        assert!(first.starts_with("head\n<!-- memory -->"), "{first}");
        assert!(first.ends_with("<!-- /memory -->\ntail"), "{first}");

        // A later turn replaces the previous block, not appends.
        let second = set_memory_block(
            &first,
            "── relevant facts ──\n• newer fact (confidence 0.90)",
        );
        assert!(!second.contains("did a thing"), "{second}");
        assert!(second.contains("• newer fact"), "{second}");
    }

    /// A prompt persisted before the markers existed only has the
    /// store-failure literal — the old swap still applies.
    #[test]
    fn falls_back_to_the_legacy_literal() {
        let legacy = "head\n```\n(no memory)\n```\ntail";
        let out = set_memory_block(legacy, "── persona ──\nlang: zh");
        assert!(out.contains("lang: zh"), "{out}");
        assert!(!out.contains("(no memory)"), "{out}");
        assert!(out.starts_with("head"), "{out}");
        assert!(out.ends_with("tail"), "{out}");
    }

    /// Neither markers nor the literal: leave it alone (never corrupt a
    /// replayed prompt).
    #[test]
    fn leaves_unrelated_prompts_alone() {
        let other = "head\nrecalled memory lives here\ntail";
        assert_eq!(set_memory_block(other, "block"), other);
    }

    /// A steered turn reaches the distiller as one correction string; an
    /// unsteered turn stays `None` (no fact is seeded).
    #[test]
    fn steers_join_into_one_correction_string() {
        assert_eq!(steered_with(&[]), None);
        assert_eq!(
            steered_with(&["use X".to_string(), "never Y".to_string()]),
            Some("use X; never Y".to_string())
        );
        // Whitespace-only steers are dropped, not joined as "a; ; b".
        assert_eq!(
            steered_with(&["use X".to_string(), "   ".to_string()]),
            Some("use X".to_string())
        );
    }

    /// Denied tools reach the distiller as the correction string; a turn
    /// with no approval-card denials stays `None`.
    #[test]
    fn denials_join_into_correction_string() {
        assert_eq!(denied_tools(&[]), None);
        assert_eq!(
            denied_tools(&["apply_patch".to_string(), "shell".to_string()]),
            Some("denied tool(s): apply_patch, shell".to_string())
        );
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
