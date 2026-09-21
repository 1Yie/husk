//! `batch_execute` — bounded parallel **observation** batch. The model
//! packs up to 16 independent readonly calls into one round trip; the
//! batch runs them at limited concurrency, preserves input order in the
//! result, and reports per-item success/error (partial failure ≠ batch
//! failure).
//!
//! Deliberately NOT an arbitrary parallel executor:
//!
//! - **Observation-only.** A call is eligible iff its spec is `readonly`
//!   AND the name isn't in `EXCLUDED` — `readonly` tools that still don't
//!   belong in a batch: `delegate` (subagent scheduler), `ask_question`
//!   (blocks on a human), `goal_complete`/`goal_blocked` (turn-control
//!   signals), `todo` (session-state write — racing load→mutate→save),
//!   `batch_execute` itself (no nesting).
//! - **Mode-scoped.** Dispatch resolves through the engine's live
//!   `active_registry` slot — plan mode's readonly registry applies to
//!   batches the same as single calls.
//! - **Budgeted.** A batch has its own wall clock (30s); each item gets
//!   the *remaining* budget, so a slow call can't let the batch run 16×
//!   its own timeout. Aggregate output is capped (5 MiB) — the model
//!   shouldn't get an unbounded concatenation.
//! - **No gate needed.** Only auto-allowed readonly specs ever run, so
//!   the batch skips the permission gate by construction.
//!
//! Mutation batching (transaction/rollback/conflict detection) is a
//! different feature — not this tool.

use super::registry::{Args, ToolCtx, ToolError, ToolResult, ToolSpec};
use futures::{FutureExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_BATCH_CALLS: usize = 16;
const MAX_BATCH_CONCURRENCY: usize = 4;
const MAX_BATCH_WALL: Duration = Duration::from_secs(30);
const MAX_BATCH_OUTPUT: usize = 5 * 1024 * 1024;

/// `readonly: true` tools that still can't batch — the flag alone isn't
/// the whole story; these are control signals, human-blocking calls,
/// session-state writes, or the batch tool itself.
const EXCLUDED: &[&str] = &[
    "batch_execute",
    "delegate",
    "ask_question",
    "todo",
    "goal_complete",
    "goal_blocked",
];

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchCall {
    /// Tool name — must be a readonly observation tool.
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
            "Run up to 16 INDEPENDENT readonly observation calls (read_file, list_dir, \
             smart_grep, web_fetch, git_status…) as ONE round trip — use it whenever you \
             would otherwise fire several observation tools back to back. Calls run at \
             limited concurrency; results return in the order you sent them, and one call's \
             failure doesn't fail the batch (check each `── [i] ──` section). Forbidden: \
             mutations (write/patch/shell/test), delegate, todo, ask_question, goal signals, \
             and nested batch_execute — those are rejected per-item.",
        ),
        readonly: true, // orchestrator — only ever runs auto-allowed readonly specs
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

/// Batch eligibility — the `readonly` flag covers capability, `EXCLUDED`
/// covers semantics (control signals, human-blocking, session state,
/// nesting). Both must pass.
fn is_batchable(spec: &ToolSpec) -> bool {
    spec.readonly && !EXCLUDED.contains(&spec.name)
}

struct ItemOut {
    tool: String,
    result: Result<String, String>,
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

    // Validate EVERY call up front — an ineligible call becomes an error
    // item, it doesn't sink the batch (partial-failure semantics).
    let deadline = Instant::now() + MAX_BATCH_WALL;
    let streams = parsed.calls.into_iter().enumerate().map(|(i, call)| {
        let ctx = ctx.clone();
        let spec = registry.spec(call.tool.as_str());
        let eligible = spec.is_some() && is_batchable(spec.unwrap());
        let spec = spec.cloned();
        let tool = call.tool.clone();
        async move {
            let out = match (spec, eligible) {
                (None, _) => ItemOut {
                    tool,
                    result: Err("unknown tool".into()),
                },
                (Some(_), false) => ItemOut {
                    tool,
                    result: Err("not batchable — mutations, delegation, questions, \
                        session state, and control signals run as single calls".into()),
                },
                (Some(spec), true) => {
                    if ctx
                        .cancel
                        .as_ref()
                        .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                    {
                        ItemOut { tool, result: Err("cancelled".into()) }
                    } else {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        let run = (spec.exec)(call.args, ctx);
                        match tokio::time::timeout(remaining, run).await {
                            Err(_) => ItemOut {
                                tool,
                                result: Err("batch wall-time budget exhausted".into()),
                            },
                            Ok(Ok(res)) => ItemOut { tool, result: Ok(res.content) },
                            Ok(Err(e)) => ItemOut {
                                tool,
                                result: Err(e.to_string()),
                            },
                        }
                    }
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
    // aggregate bytes capped.
    let ok = results.iter().filter(|r| r.result.is_ok()).count();
    let errors = results.len() - ok;
    let mut out = format!(
        "[batch_execute — {} calls · {} ok · {} error]\n",
        results.len(),
        ok,
        errors
    );
    let mut bytes = out.len();
    let mut capped = false;
    for (i, r) in results.iter().enumerate() {
        let (status, body) = match &r.result {
            Ok(c) => ("ok", c.as_str()),
            Err(e) => ("error", e.as_str()),
        };
        let section = if capped {
            format!("\n── [{i}] {} ({status}) ── [output dropped — batch output cap reached]\n", r.tool)
        } else {
            let room = MAX_BATCH_OUTPUT.saturating_sub(bytes + 96);
            let body = if body.len() > room {
                capped = true;
                &body[..body.floor_char_boundary(room)]
            } else {
                body
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
            ("delegate", false),
            ("ask_question", false),
            ("todo", false),
            ("batch_execute", false),
        ] {
            let spec = reg.spec(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(is_batchable(spec), want, "{name}");
        }
    }
}
