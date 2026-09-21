//! `batch_execute` — bounded parallel **observation** batch. The model
//! packs up to 16 independent readonly calls into one round trip; the
//! batch runs them at limited concurrency, preserves input order in the
//! result, and reports per-item status (partial failure ≠ batch failure).
//!
//! Deliberately NOT an arbitrary parallel executor — eligibility keys
//! off `ToolSpec::class`, not name lists:
//!
//! - **Observation-only.** `class == ToolClass::Observation` is the whole
//!   rule: mutations (`WorkspaceMutation`/`Process`), session state
//!   (`SessionMutation`), control signals (`Control`), human-blocking
//!   calls (`HumanInteraction`), and orchestration (`Orchestration` —
//!   `delegate`, `batch_execute` itself) are all rejected by class, so a
//!   new tool can't slip in just by being `readonly`.
//! - **Network sub-cap.** `spec.network` items (web_fetch) are
//!   Observation but capped lower — a batch can't become a
//!   16-way SSRF/rate-limit amplifier.
//! - **Mode-scoped.** Dispatch resolves through the engine's live
//!   `active_registry` slot — plan mode's readonly registry applies to
//!   batches the same as single calls.
//! - **Real cancellation.** Items run under a batch-scoped flag that a
//!   watchdog sets on deadline OR parent cancel — tools that poll
//!   `ctx.cancel` (grep, web_fetch, delegate) actually abort in-flight
//!   work instead of merely being dropped by `timeout`.
//! - **Visible.** Each item emits ToolCallStarted/Finished through
//!   `ctx.ui_tx` — the UI shows 16 capsules, not a silent 30s batch.
//!   Skipped where no channel exists (headless).
//! - **No gate needed.** Only auto-allowed Observation specs ever run,
//!   so the batch skips the permission gate by construction. Mutation
//!   batching (transaction/rollback/conflict detection) is a different
//!   feature — not this tool.

use super::registry::{Args, ToolClass, ToolCtx, ToolError, ToolResult, ToolSpec};
use futures::{FutureExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_BATCH_CALLS: usize = 16;
const MAX_BATCH_CONCURRENCY: usize = 4;
/// Network reads share the observation pool but get a tighter ceiling —
/// 16 concurrent fetches is an SSRF/rate-limit amplifier, not a feature.
const MAX_NETWORK_CALLS: usize = 4;
const MAX_BATCH_WALL: Duration = Duration::from_secs(30);
const MAX_BATCH_OUTPUT: usize = 5 * 1024 * 1024;

/// The batch's own resource envelope — deadline is a BATCH deadline,
/// not a per-item timeout; every item sees the remaining slice at start.
struct BatchBudget {
    deadline: Instant,
    /// Set by the watchdog when the deadline lands or the parent turn is
    /// cancelled — propagates into every item's ctx so blocking work
    /// (spawn_blocking walks, in-flight fetches) actually stops.
    cancel: Arc<AtomicBool>,
}

#[derive(Debug)]
enum ItemStatus {
    Ok,
    Error,
    Timeout,
    /// Failed eligibility up front (non-observation class / unknown tool).
    Rejected,
    Cancelled,
}

struct ItemOut {
    tool: String,
    status: ItemStatus,
    body: String,
    duration_ms: u64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchCall {
    /// Tool name — must be an observation tool (read_file, list_dir,
    /// smart_grep, web_fetch…). Mutations, delegation, questions, session
    /// state, and control signals are rejected per-item.
    tool: String,
    /// Arguments for that tool, same shape as a direct call.
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchArgs {
    /// Up to 16 independent calls — results come back in THIS order.
    calls: Vec<BatchCall>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "batch_execute",
        schema: super::registry::schema_for::<BatchArgs>(
            "Run up to 16 INDEPENDENT observation calls (smart_read, list_dir, \
             smart_grep, web_fetch…) as ONE round trip — use it whenever you \
             would otherwise fire several read-only tools back to back. Calls \
             run at limited concurrency (max 4, max 4 network calls); results \
             return in the order you sent them, and one call's failure doesn't \
             fail the batch (check each `── [i] ──` section). Forbidden by \
             design: mutations (write/patch/shell/test), delegate, todo, \
             ask_question, goal signals, and nested batch_execute.",
        ),
        readonly: true, // orchestrator — only ever runs auto-allowed Observation specs
        class: ToolClass::Orchestration,
        network: false,
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

/// Batch eligibility — class is the whole story; no name lists.
fn is_batchable(spec: &ToolSpec) -> bool {
    spec.class == ToolClass::Observation
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: BatchArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(e.to_string()))?;
    if parsed.calls.is_empty() {
        return Err(ToolError::Args("`calls` must not be empty".into()));
    }
    if parsed.calls.len() > MAX_BATCH_CALLS {
        return Err(ToolError::Args(format!(
            "at most {MAX_BATCH_CALLS} calls per batch — split the rest"
        )));
    }

    let registry = ctx
        .active_registry
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| ToolError::Failed("registry unavailable".into()))?;

    // Network sub-cap — count up front so over-capacity items get a
    // per-item rejection instead of silently queueing.
    let mut network_used = 0usize;
    let mut eligible_flags: Vec<Result<Arc<ToolSpec>, String>> =
        Vec::with_capacity(parsed.calls.len());
    for call in &parsed.calls {
        match registry.spec(&call.tool) {
            None => eligible_flags.push(Err("unknown tool".into())),
            Some(spec) if !is_batchable(&spec) => {
                eligible_flags.push(Err(
                    "not batchable — only Observation tools run in a batch".into(),
                ))
            }
            Some(spec) if spec.network => {
                network_used += 1;
                if network_used > MAX_NETWORK_CALLS {
                    eligible_flags.push(Err("network call limit for this batch (4)".into()));
                } else {
                    eligible_flags.push(Ok(spec));
                }
            }
            Some(spec) => eligible_flags.push(Ok(spec)),
        }
    }

    // Batch-scoped cancellation — a watchdog sets the flag on deadline OR
    // parent cancel; every item's ctx carries it so blocking work aborts
    // for real instead of being abandoned by `timeout`.
    let budget = BatchBudget {
        deadline: Instant::now() + MAX_BATCH_WALL,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    {
        let flag = budget.cancel.clone();
        let parent = ctx.cancel.clone();
        let deadline = budget.deadline;
        tokio::spawn(async move {
            loop {
                if Instant::now() >= deadline
                    || parent
                        .as_ref()
                        .is_some_and(|c| c.load(Ordering::Relaxed))
                {
                    flag.store(true, Ordering::Relaxed);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
    }

    let started = Instant::now();
    let streams = parsed
        .calls
        .into_iter()
        .zip(eligible_flags)
        .enumerate()
        .map(|(i, (call, flag))| {
            let ctx = ctx.clone();
            let batch_cancel = budget.cancel.clone();
            let deadline = budget.deadline;
            let tool = call.tool.clone();
            async move {
                let t0 = Instant::now();
                let out = match flag {
                    Err(reason) => ItemOut {
                        tool,
                        status: ItemStatus::Rejected,
                        body: reason,
                        duration_ms: 0,
                    },
                    Ok(spec) => {
                        // Item ctx carries the batch flag — tools that poll
                        // cancel abort their own blocking work too.
                        let item_ctx =
                            Arc::new(ctx.with_cancel(batch_cancel.clone()));
                        if let Some(ui) = &ctx.ui_tx {
                            let _ = ui.try_send(agent_ipc::events::UiEvent::ToolCallStarted {
                                name: tool.clone(),
                                args_preview: call
                                    .args
                                    .get("path")
                                    .or_else(|| call.args.get("pattern"))
                                    .or_else(|| call.args.get("url"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .chars()
                                    .take(120)
                                    .collect(),
                            });
                        }
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        let run = (spec.exec)(call.args, item_ctx);
                        let out = match tokio::time::timeout(remaining, run).await {
                            Err(_) => ItemOut {
                                tool: tool.clone(),
                                status: if batch_cancel.load(Ordering::Relaxed) {
                                    ItemStatus::Cancelled
                                } else {
                                    ItemStatus::Timeout
                                },
                                body: "batch wall-time budget exhausted".into(),
                                duration_ms: t0.elapsed().as_millis() as u64,
                            },
                            Ok(Ok(res)) => {
                                // Readonly tools never stage writes — a
                                // pending_write here would be dropped by
                                // design anyway (observations don't write).
                                ItemOut {
                                    tool: tool.clone(),
                                    status: ItemStatus::Ok,
                                    body: res.content,
                                    duration_ms: t0.elapsed().as_millis() as u64,
                                }
                            }
                            Ok(Err(e)) => ItemOut {
                                tool: tool.clone(),
                                status: ItemStatus::Error,
                                body: e.to_string(),
                                duration_ms: t0.elapsed().as_millis() as u64,
                            },
                        };
                        if let Some(ui) = &ctx.ui_tx {
                            let (ok, content) = match &out.status {
                                ItemStatus::Ok => (true, out.body.clone()),
                                _ => (false, out.body.clone()),
                            };
                            let _ = ui.try_send(agent_ipc::events::UiEvent::ToolCallFinished {
                                name: tool,
                                ok,
                                content,
                                ui_type: None,
                            });
                        }
                        out
                    }
                };
                (i, out)
            }
        });

    // `buffered` = bounded concurrency + INPUT order preserved — the model
    // maps `── [i] ──` sections back to its own call list.
    let results: Vec<ItemOut> = futures::stream::iter(streams)
        .buffered(MAX_BATCH_CONCURRENCY)
        .map(|(_, out)| out)
        .collect()
        .await;

    // Sectioned model-facing text — one block per call, order preserved,
    // aggregate bytes capped. Status + duration ride in every header.
    let ok = results.iter().filter(|r| matches!(r.status, ItemStatus::Ok)).count();
    let errors = results.len() - ok;
    let mut out = format!(
        "[batch_execute — {} calls · {} ok · {} failed · {:.1}s]\n",
        results.len(),
        ok,
        errors,
        started.elapsed().as_secs_f32()
    );
    let mut bytes = out.len();
    let mut capped = false;
    for (i, r) in results.iter().enumerate() {
        let status = match &r.status {
            ItemStatus::Ok => format!("ok · {}ms", r.duration_ms),
            ItemStatus::Error => "error".into(),
            ItemStatus::Timeout => "timeout".into(),
            ItemStatus::Rejected => "rejected".into(),
            ItemStatus::Cancelled => "cancelled".into(),
        };
        let section = if capped {
            format!(
                "\n── [{i}] {} ({status}) ── [output dropped — batch output cap reached]\n",
                r.tool
            )
        } else {
            let room = MAX_BATCH_OUTPUT.saturating_sub(bytes + 96);
            let body = if r.body.len() > room {
                capped = true;
                &r.body[..r.body.floor_char_boundary(room)]
            } else {
                r.body.as_str()
            };
            format!("\n── [{i}] {} ({status}) ──\n{body}\n", r.tool)
        };
        bytes += section.len();
        out.push_str(&section);
    }
    if capped {
        out.push_str("\n⚠ batch output cap reached — later sections were truncated or dropped\n");
    }
    Ok(ToolResult::text(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_matrix() {
        let reg = crate::tools::registry::ToolRegistry::with_builtins();
        for (name, want) in [
            ("smart_read", true),
            ("list_dir", true),
            ("smart_grep", true),
            ("web_fetch", true),
            ("bash", false),
            ("apply_patch", false),
            ("fuzzy_patch", false),
            ("smart_test_runner", false),
            ("serena", false),
            ("delegate", false),
            ("ask_question", false),
            ("todo", false),
            ("batch_execute", false),
        ] {
            let spec = reg.spec(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(is_batchable(&spec), want, "{name}");
        }
    }

    #[test]
    fn network_flag_only_on_fetch() {
        let reg = crate::tools::registry::ToolRegistry::with_builtins();
        for (name, want) in [
            ("web_fetch", true),
            ("smart_read", false),
            ("list_dir", false),
            ("smart_grep", false),
            ("batch_execute", false),
        ] {
            let spec = reg.spec(name).unwrap();
            assert_eq!(spec.network, want, "{name}");
        }
    }
}
