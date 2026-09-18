//! `smart_test_runner` — failure-only filter over test output.
//!
//! Contract (native-tools.md §B): stream-filter the sandboxed test process —
//! drop `passed`/progress noise, keep only failed test names + assertion
//! locations + panic backtrace frames. 20k-line log → ≤300 tok diagnostic.
//!
//! Stage 3 runs the command via `tokio::process` (the same path `bash`
//! uses); the Stage-8 sandbox backend will wrap this unchanged — the tool's
//! contract is the *filter*, not the spawn.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Failure-diagnostic budget (≤~300 tokens ≈ 1.2 KB of dense signal).
const MAX_DIAGNOSTIC_CHARS: usize = 4 * 1024;
const TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct TestRunnerArgs {
    /// Test command, e.g. `cargo test -p agent-kernel`, `pytest`, `vitest run`.
    command: String,
    /// Optional sub-filter: only keep output lines mentioning this (e.g. a
    /// test name) — applied to the failure sections, not the whole log.
    focus: Option<String>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "smart_test_runner",
        schema: schema_for::<TestRunnerArgs>(
            "Run a test command and return ONLY the failures: failed test \
             names, assertion diffs, panic backtrace frames. Passing noise is \
             dropped — a 20k-line log returns ~300 tokens of signal. Use for \
             `cargo test`, `pytest`, `vitest`/`jest`, `go test`.",
        ),
        readonly: false, // test runs can write caches/artifacts; gate via permissions
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: TestRunnerArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("smart_test_runner args: {e}")))?;

    let out = tokio::time::timeout(
        TIMEOUT,
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&parsed.command)
            .current_dir(&*ctx.workspace_root)
            .output(),
    )
    .await
    .map_err(|_| ToolError::Failed(format!("test run exceeded {}s", TIMEOUT.as_secs())))?
    .map_err(ToolError::Io)?;

    let mut raw = String::new();
    raw.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        raw.push_str(&String::from_utf8_lossy(&out.stderr));
    }

    let distilled = distill(&raw, parsed.focus.as_deref());
    let status = if out.status.success() { "PASS" } else { "FAIL" };
    let mut content = format!("── {status} `{}` ──\n", parsed.command);
    content.push_str(&distilled);
    if distilled.len() >= MAX_DIAGNOSTIC_CHARS {
        content.push_str("\n… [diagnostic truncated — rerun with a narrower test filter] …\n");
    }
    Ok(ToolResult::text(content))
}

/// Failure-only distillation. Framework-agnostic: keep lines that look like
/// failures/assertions/panics, plus the first summary block.
#[doc(hidden)]
pub fn test_distill_pub(raw: &str, focus: Option<&str>) -> String {
    distill(raw, focus)
}

fn distill(raw: &str, focus: Option<&str>) -> String {
    const KEEP_PATTERNS: &[&str] = &[
        "FAILED", "failed", "failures:", "error[", "error:", "panicked",
        "assertion failed", "assertion `", "FAILED:", "Error:", "FAIL ",
        "expected", "Expected", "left:", "right:", "diff <", "thread '",
        "not ok", "✗", "×", "Cannot find", "TypeError", "ReferenceError",
        "File \"", "line ", "at ",
    ];
    const DROP_PATTERNS: &[&str] = &[
        "... ok", "test result: ok", "Compiling", "Finished", "Running ",
        "Downloading", "Downloaded", "npm warn", "PASS ", "✓ ",
    ];

    let mut kept: Vec<&str> = Vec::new();
    let mut last_kept_idx = 0i64;
    let lines: Vec<&str> = raw.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        // Context glue: keep a line if it (a) matches a failure pattern, or
        // (b) directly follows one (assertion continuations, backtrace).
        let keep = KEEP_PATTERNS.iter().any(|p| l.contains(p))
            && !DROP_PATTERNS.iter().any(|p| l.contains(p))
            || (i as i64 - last_kept_idx <= 2 && kept.last().is_some_and(|_| {
                l.starts_with("at ") || l.starts_with('|') || l.starts_with("File ")
            }));
        if let Some(f) = focus {
            if keep && !f.is_empty() && !l.contains(f) && KEEP_PATTERNS.iter().any(|p| l.contains(p)) {
                // focus filter: non-focused failures drop unless they're the summary
                if !l.contains("test result") && !l.contains("failures:") {
                    continue;
                }
            }
        }
        if keep {
            kept.push(line);
            last_kept_idx = i as i64;
        }
    }

    if kept.is_empty() {
        // Nothing matched — either clean pass or unrecognized output. Return
        // the last 20 lines as a fallback summary rather than silence.
        let tail: Vec<&str> = lines.iter().rev().take(20).copied().collect();
        let t: Vec<&str> = tail.into_iter().rev().collect();
        if t.is_empty() {
            return "(no output)\n".into();
        }
        return t.join("\n") + "\n";
    }

    let mut out = kept.join("\n");
    out.push('\n');
    if out.len() > MAX_DIAGNOSTIC_CHARS {
        // UTF-8-safe truncate — a byte cut mid-char would panic (P2).
        crate::tools::util::truncate(&mut out, MAX_DIAGNOSTIC_CHARS);
    }
    out
}
