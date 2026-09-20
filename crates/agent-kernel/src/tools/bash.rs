//! `bash` — sandboxed shell execution.
//!
//! Stage 8: runs through `ctx.sandbox` (`SandboxBackend::run_command`) —
//! bwrap on Linux, loud `none` fallback elsewhere. The audit gate runs
//! *before* dispatch and stamps the risk tier on the result; the permission
//! layer (Stage 6) already gated the call through `AwaitingToolConfirmation`.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Shell output cap per truncation table (~5k tok).
const MAX_OUTPUT_CHARS: usize = 20 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct BashArgs {
    /// Shell command to run inside the workspace.
    command: String,
    /// Timeout in seconds (default 120, max 600).
    timeout_secs: Option<u64>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "bash",
        schema: schema_for::<BashArgs>(
            "Run a shell command in the workspace. Output is truncated to \
             20 KB (head 30% + tail 70%). Prefer dedicated tools \
             (smart_read/smart_grep/fuzzy_patch/apply_patch) for file IO —
             never `cat`/`echo`/`sed` to create or edit files; use apply_patch
             to create new files/folders, fuzzy_patch for in-place edits.
             bash is for builds, tests, and commands those don't cover.",
        ),
        readonly: false,
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: BashArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("bash args: {e}")))?;
    let timeout = parsed
        .timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TIMEOUT)
        .min(MAX_TIMEOUT);

    // ---- Stage 8: audit before spawn → risk tier on the result ----
    // Scoped to the workspace so out-of-scope writes flag ScopeViolation.
    let verdict = agent_sandbox::audit_command_scoped(
        &parsed.command,
        Some(ctx.workspace_root.as_ref()),
    );

    // Policy → SandboxPlan → SandboxConfig. The plan derives network/fs
    // limits from the audit's capabilities (Network/PackageInstall flips
    // allow_network; High+ risk forces a CoW snapshot). The L2 backend still
    // consumes SandboxConfig — the plan is its structured source.
    let plan = agent_sandbox::plan::SandboxPlan::from_audit(
        &verdict,
        ctx.workspace_root.as_ref().to_path_buf(),
        std::env::temp_dir().join(format!("husk-{:x}", std::process::id())),
    );
    let cfg = agent_sandbox::SandboxConfig {
        workspace_dir: ctx.workspace_root.as_ref().to_path_buf(),
        allow_network: !matches!(plan.network, agent_sandbox::plan::NetworkPolicy::Deny),
        max_memory_mb: plan.processes.max_memory_mb,
        max_processes: plan.processes.max_processes,
        timeout_secs: timeout.as_secs(),
        snapshot: plan.snapshot,
        environment: plan.environment.clone(),
        ..Default::default()
    };

    let output = ctx
        .sandbox
        .run_command(&parsed.command, &[], &cfg)
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;

    let mut raw = output.stdout;
    if !output.stderr.is_empty() {
        if !raw.is_empty() {
            raw.push_str("\n── stderr ──\n");
        }
        raw.push_str(&output.stderr);
    }

    let mut content = format!("── exit {} `{}` ──\n", output.status, parsed.command);
    if verdict.level > agent_sandbox::AuditLevel::Normal {
        content.push_str(&format!(
            "[audit {:?}: {}]\n", verdict.level, verdict.reasons.join("; ")));
    }
    if output.is_timeout {
        content.push_str(&format!("[timeout {}s — tree killed]\n", timeout.as_secs()));
    }
    content.push_str(&fold_output(&raw));
    Ok(ToolResult::text(content))
}

/// Head 30% + `… [truncated N bytes] …` + tail 70% fold (errors land in tail).
fn fold_output(raw: &str) -> String {
    if raw.len() <= MAX_OUTPUT_CHARS {
        return raw.to_string();
    }
    let head = MAX_OUTPUT_CHARS * 3 / 10;
    let tail = MAX_OUTPUT_CHARS - head;
    let omitted = raw.len() - head - tail;
    let mut out = String::with_capacity(MAX_OUTPUT_CHARS + 80);
    // UTF-8-safe: byte-level head/tail clamps to char boundaries so CJK or
    // emoji in command output can't panic on a non-boundary index (P2).
    out.push_str(crate::tools::util::head(raw, head));
    out.push_str(&format!("\n… [truncated {omitted} bytes] …\n"));
    out.push_str(crate::tools::util::tail(raw, tail));
    out
}
