//! `batch_execute` — parallel execution for independent read-only calls.
//!
//! Eligibility is by `ToolSpec::class`, not by name: only `Observation` tools
//! qualify (`readonly` alone is not enough), and network items are capped lower
//! than local ones. Results keep input order, per-item failures are reported
//! rather than fatal, and cancellation is batch-scoped so tools that poll
//! `ctx.cancel` actually abort. No permission gate runs: only auto-allowed
//! specs are eligible by construction.


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
    /// state, and control signals are rejected per-item. `name`/`tool_name`
    /// are accepted because models reach for them.
    #[serde(alias = "name", alias = "tool_name")]
    tool: String,
    /// Arguments for that tool, same shape as a direct call. `arguments` is
    /// accepted as an alias (the MCP spelling).
    #[serde(default, alias = "arguments")]
    args: serde_json::Value,
    /// Argument keys written inline instead of under `args`
    /// (`{"tool":"smart_read","path":"…"}`) — absorbed by [`BatchCall::normalized`].
    #[serde(flatten)]
    inline: serde_json::Map<String, serde_json::Value>,
}

impl BatchCall {
    /// Fold inline keys into `args`: the flattened form is the second thing
    /// every model tries, and rejecting it costs a whole turn.
    fn normalized(mut self) -> Self {
        if self.inline.is_empty() {
            return self;
        }
        let inline = std::mem::take(&mut self.inline);
        match &mut self.args {
            serde_json::Value::Null => self.args = serde_json::Value::Object(inline),
            serde_json::Value::Object(map) => {
                for (key, value) in inline {
                    // An explicit `args` entry wins over an inline duplicate.
                    map.entry(key).or_insert(value);
                }
            }
            _ => {}
        }
        self
    }
}

/// One `calls[]` entry after lenient parsing.
#[derive(Debug)]
enum ParsedItem {
    Call(BatchCall),
    /// Unparseable — reported as that item's rejection instead of failing the
    /// whole batch, and named from whatever the item did carry.
    Bad { name: String, reason: String },
}

/// The documented shape, quoted back at the model when an item makes no sense.
const SHAPE_HINT: &str = "`{calls: [{tool, args}]}` — e.g. \
{\"calls\":[{\"tool\":\"smart_read\",\"args\":{\"path\":\"src/lib.rs\"}}]} \
(`args` may be omitted and its keys written inline next to `tool`)";

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Parse `{calls: […]}` leniently: the strict derive first, and anything it
/// rejects becomes a per-item failure that quotes what arrived.
fn parse_items(args: serde_json::Value) -> Result<Vec<ParsedItem>, ToolError> {
    let raw = args.get("calls").cloned().ok_or_else(|| {
        ToolError::Args(format!(
            "batch_execute args: missing field `calls`\nExpected {SHAPE_HINT}"
        ))
    })?;
    let items = raw.as_array().cloned().ok_or_else(|| {
        ToolError::Args(format!(
            "batch_execute args: `calls` must be an array, got {}\nExpected {SHAPE_HINT}",
            json_kind(&raw)
        ))
    })?;
    Ok(items
        .into_iter()
        .map(|item| match serde_json::from_value::<BatchCall>(item.clone()) {
            Ok(call) => ParsedItem::Call(call.normalized()),
            Err(e) => {
                let name = ["tool", "name", "tool_name"]
                    .iter()
                    .find_map(|k| item.get(k).and_then(|v| v.as_str()))
                    .unwrap_or("(unnamed)")
                    .to_string();
                let mut shown = item.to_string();
                if shown.len() > 160 {
                    shown.truncate(shown.floor_char_boundary(160));
                    shown.push('…');
                }
                ParsedItem::Bad {
                    name,
                    reason: format!(
                        "item could not be parsed ({e}); got {shown} — expected \
                         {{tool, args}} with `tool` naming an Observation tool"
                    ),
                }
            }
        })
        .collect())
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchArgs {
    /// Read only by `schema_for` — runtime parsing goes through `parse_items`,
    /// which is lenient about the shapes models actually emit.
    #[allow(dead_code)]

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
    let items = parse_items(args)?;
    if items.is_empty() {
        return Err(ToolError::Args("`calls` must not be empty".into()));
    }
    if items.len() > MAX_BATCH_CALLS {
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
        Vec::with_capacity(items.len());
    for item in &items {
        let ParsedItem::Call(call) = item else {
            let ParsedItem::Bad { reason, .. } = item else {
                unreachable!()
            };
            eligible_flags.push(Err(reason.clone()));
            continue;
        };
        match registry.spec(&call.tool) {
            None => eligible_flags.push(Err(format!(
                "unknown tool `{}` — not in the active registry",
                call.tool
            ))),
            Some(spec) if !is_batchable(&spec) => {
                eligible_flags.push(Err(format!(
                    "`{}` is not batchable — only Observation tools run in a batch",
                    call.tool
                )))
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
    let streams = items
        .into_iter()
        .zip(eligible_flags)
        .enumerate()
        .map(|(i, (item, flag))| {
            let ctx = ctx.clone();
            let batch_cancel = budget.cancel.clone();
            let deadline = budget.deadline;
            let (tool, call_args) = match item {
                ParsedItem::Call(call) => (call.tool, call.args),
                ParsedItem::Bad { name, .. } => (name, serde_json::Value::Null),
            };
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
                            let _ = ui.send(agent_ipc::events::UiEvent::ToolCallStarted {
                                name: tool.clone(),
                                // Nested — the UI renders this under the
                                // batch_execute capsule, not top-level.
                                parent: Some("batch_execute".into()),
                                args_preview: call_args
                                    .get("path")
                                    .or_else(|| call_args.get("pattern"))
                                    .or_else(|| call_args.get("url"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or_default()
                                    .chars()
                                    .take(120)
                                    .collect(),
                            });
                        }
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        let run = (spec.exec)(call_args, item_ctx);
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
                            let _ = ui.send(agent_ipc::events::UiEvent::ToolCallFinished {
                                name: tool,
                                ok,
                                content,
                                ui_type: None,
                                parent: Some("batch_execute".into()),
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
            // A loaded skill's instructions must not arrive inside a batch blob.
            ("skill", false),
            ("delegate", false),
            ("ask_question", false),
            ("todo", false),
            ("batch_execute", false),
        ] {
            let spec = reg.spec(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(is_batchable(&spec), want, "{name}");
        }
    }

    /// Every shape a model reaches for must land as a usable call — the old
    /// strict derive failed the whole batch with a bare `missing field \`tool\``.
    #[test]
    fn item_shapes_the_model_writes_all_parse() {
        let items = parse_items(serde_json::json!({"calls": [
            {"tool": "smart_read", "args": {"path": "a.rs"}},
            {"name": "list_dir", "arguments": {"path": "."}},
            {"tool_name": "smart_grep", "pattern": "TODO"},
            {"tool": "smart_read", "path": "b.rs", "start": 3}
        ]}))
        .unwrap();
        assert_eq!(items.len(), 4);
        for item in &items {
            let ParsedItem::Call(call) = item else {
                panic!("unparsed: {item:?}")
            };
            assert!(!call.tool.is_empty());
            assert!(
                call.args.is_object(),
                "{} args not normalized: {:?}",
                call.tool,
                call.args
            );
        }
        let ParsedItem::Call(grep) = &items[2] else { unreachable!() };
        assert_eq!(grep.args["pattern"], "TODO", "inline keys must survive");
    }

    #[test]
    fn a_bad_item_is_rejected_alone_with_the_expected_shape() {
        let items = parse_items(serde_json::json!({"calls": [
            {"tool": "list_dir", "args": {"path": "."}},
            {"path": "orphan.rs"},
            7
        ]}))
        .unwrap();
        assert!(matches!(items[0], ParsedItem::Call(_)));
        let ParsedItem::Bad { name, reason } = &items[1] else {
            panic!("expected a rejected item")
        };
        assert_eq!(name, "(unnamed)");
        assert!(reason.contains("tool"), "{reason}");
        assert!(matches!(items[2], ParsedItem::Bad { .. }));
    }

    #[test]
    fn a_missing_or_non_array_calls_field_names_the_shape() {
        let e = format!("{}", parse_items(serde_json::json!({})).unwrap_err());
        assert!(e.contains("missing field `calls`"), "{e}");
        assert!(e.contains("{calls:"), "{e}");
        let e = format!(
            "{}",
            parse_items(serde_json::json!({"calls": {}})).unwrap_err()
        );
        assert!(e.contains("must be an array"), "{e}");
    }

    /// End-to-end through the real dispatch: good items run, the shapeless one
    /// is rejected alone, and the batch still reports every section.
    #[tokio::test]
    async fn a_real_batch_runs_and_isolates_a_shapeless_item() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let mut ctx = ToolCtx::new(dir.path());
        ctx.active_registry = std::sync::RwLock::new(Some(Arc::new(
            crate::tools::registry::ToolRegistry::with_builtins(),
        )));
        let out = exec(
            serde_json::json!({"calls": [
                {"tool": "list_dir", "args": {"path": "."}},
                {"name": "smart_read", "arguments": {"path": "a.rs"}},
                {"path": "orphan.rs"}
            ]}),
            Arc::new(ctx),
        )
        .await
        .expect("a bad item must not fail the batch");
        let text = out.content;
        assert!(text.contains("list_dir (ok"), "{text}");
        assert!(text.contains("smart_read (ok"), "{text}");
        assert!(text.contains("rejected"), "{text}");
        assert!(text.contains("could not be parsed"), "{text}");
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
