//! Observation routing — the runtime half of "look with the file tools, run
//! with the shell".
//!
//! What each tool is for lives with the tool, in `agent-tools-fs` and
//! `agent-tools-shell`: their descriptions name the shell command each one
//! replaces, and `smart_grep` carries `context` natively so a hit and its
//! surroundings no longer need two tools. What a description cannot say is
//! anything about *this* conversation. This module adds exactly that, after
//! the result and without ever changing it:
//!
//! - [`ObservationLedger`] remembers digests of what each call returned, so a
//!   call repeated verbatim is marked as identical to its earlier output, a
//!   file whose `content_hash` moved is marked stale, and a failure repeated
//!   unchanged is marked as such;
//! - a `bash` command that only read, searched or listed workspace files gets
//!   a routing note — the command itself still runs untouched.
//!
//! Nothing is refused and nothing is withheld: every call runs the real tool
//! and returns its whole output, so a stale ledger can at worst add a note.

mod ledger;
mod shell;

pub use ledger::ObservationLedger;
pub use shell::{shell_observation, ObservationKind, ShellObservation};

use std::fmt::Write as _;
use std::sync::Arc;

use agent_capabilities::WorkspaceCtx;
use agent_kernel::tools::registry::{Args, ExecFn, ToolCtx, ToolError, ToolResult, ToolSpec};
use futures::FutureExt;

/// `smart_read`, annotated when a read repeats or its file changed.
pub fn read(inner: ToolSpec) -> ToolSpec {
    let pass = inner.exec.clone();
    let exec: ExecFn = Arc::new(move |args: Args, ctx: Arc<ToolCtx>| {
        let pass = pass.clone();
        async move {
            let key = call_key("smart_read", &args);
            let result = pass(args, ctx.clone()).await;
            let mut out = settle(&ctx, &key, result)?;
            note_file_change(&ctx, &mut out.content);
            Ok(out)
        }
        .boxed()
    });
    ToolSpec { exec, ..inner }
}

/// `smart_grep`, annotated when a search repeats verbatim.
pub fn grep(inner: ToolSpec) -> ToolSpec {
    let pass = inner.exec.clone();
    let exec: ExecFn = Arc::new(move |args: Args, ctx: Arc<ToolCtx>| {
        let pass = pass.clone();
        async move {
            let key = call_key("smart_grep", &args);
            let result = pass(args, ctx.clone()).await;
            settle(&ctx, &key, result)
        }
        .boxed()
    });
    ToolSpec { exec, ..inner }
}

/// `list_dir`, annotated when a listing repeats verbatim.
pub fn list(inner: ToolSpec) -> ToolSpec {
    let pass = inner.exec.clone();
    let exec: ExecFn = Arc::new(move |args: Args, ctx: Arc<ToolCtx>| {
        let pass = pass.clone();
        async move {
            let key = call_key("list_dir", &args);
            let result = pass(args, ctx.clone()).await;
            settle(&ctx, &key, result)
        }
        .boxed()
    });
    ToolSpec { exec, ..inner }
}

/// `bash` with a note after a command that only looked at workspace files.
/// The command runs untouched and its exit status is the tool's.
pub fn bash(inner: ToolSpec) -> ToolSpec {
    let pass = inner.exec.clone();
    let exec: ExecFn = Arc::new(move |args: Args, ctx: Arc<ToolCtx>| {
        let pass = pass.clone();
        async move {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let persistent = args.get("persistent").and_then(|v| v.as_bool()) == Some(true);
            let mut out = pass(args, ctx.clone()).await?;
            if persistent {
                return Ok(out);
            }
            let Ok(root) = ctx.root().map(|r| r.to_path_buf()) else {
                return Ok(out);
            };
            let Some(observation) = shell_observation(&command, &root) else {
                return Ok(out);
            };
            let ledger = ctx.ext.get::<ObservationLedger>();
            let repeat = ledger
                .as_ref()
                .is_some_and(|l| l.record_ok(&format!("bash {command}"), &out.content));
            let unchanged = observation.file.as_ref().and_then(|file| {
                let known = ledger.as_ref()?.file_hash(file)?;
                let now = ledger::content_hash(&std::fs::read_to_string(file).ok()?);
                (known == now).then(|| {
                    file.strip_prefix(&root)
                        .unwrap_or(file)
                        .display()
                        .to_string()
                })
            });
            out.content.push('\n');
            out.content
                .push_str(&routing_note(&observation, unchanged.as_deref(), repeat));
            Ok(out)
        }
        .boxed()
    });
    ToolSpec { exec, ..inner }
}

/// `serde_json` maps are ordered by key, so equal arguments give equal keys
/// however the model ordered them.
fn call_key(tool: &str, args: &Args) -> String {
    format!("{tool} {args}")
}

/// Record the outcome in the ledger and annotate it: a repeated identical
/// output, a repeated identical failure, a path the file tools cannot reach.
fn settle(
    ctx: &ToolCtx,
    key: &str,
    result: Result<ToolResult, ToolError>,
) -> Result<ToolResult, ToolError> {
    let ledger = ctx.ext.get::<ObservationLedger>();
    match result {
        Ok(mut out) => {
            if ledger.is_some_and(|l| l.record_ok(key, &out.content)) {
                out.content.push_str(
                    "\n[note: identical to the output this same call returned earlier — nothing \
                     changed since. If that result is still in your context, work from it \
                     instead of observing again.]",
                );
            }
            Ok(out)
        }
        Err(e) => {
            // An out-of-scope path is a *question* the engine answers with a
            // grant card — flattening it into `Failed` here would hide the
            // variant and the card would never appear.
            if let ToolError::OutOfScope { .. } = &e {
                return Err(e);
            }
            let mut message = e.to_string();
            if matches!(e, ToolError::PathEscape(_)) {
                message.push_str(
                    "\n[note: the file tools reach the workspace and the workspaces linked to \
                     it. To work in another directory, call `link_workspace` on its project root \
                     (the user approves the grant), then run this call again; `bash` reads it \
                     once but leaves nothing behind for the next edit.]",
                );
            }
            if ledger.is_some_and(|l| l.record_err(key, &e.to_string())) {
                message.push_str(
                    "\n[note: this exact call already failed with this exact error. Change the \
                     arguments or the approach — retrying it unchanged will fail again.]",
                );
            }
            Err(ToolError::Failed(message))
        }
    }
}

/// After a `smart_read`: if the file's hash moved since the last read, say so
/// — every excerpt of it seen before is stale.
fn note_file_change(ctx: &ToolCtx, content: &mut String) {
    let Some(ledger) = ctx.ext.get::<ObservationLedger>() else {
        return;
    };
    let Some((path, hash)) = read_header(content).map(|(p, h)| (p.to_string(), h.to_string()))
    else {
        return;
    };
    let Ok(resolved) = ctx.resolve(&path) else {
        return;
    };
    if let Some(old) = ledger.record_file(resolved, &hash) {
        let _ = write!(
            content,
            "\n[note: this file changed since your previous read (content_hash {old} → {hash}) — \
             line numbers and text from earlier reads of it are stale.]"
        );
    }
}

/// `smart_read`'s first line: `<path>  (content_hash: <hash>)`.
fn read_header(content: &str) -> Option<(&str, &str)> {
    let first = content.lines().next()?;
    let (path, rest) = first.rsplit_once("  (content_hash: ")?;
    Some((path, rest.strip_suffix(')')?))
}

fn routing_note(observation: &ShellObservation, unchanged: Option<&str>, repeat: bool) -> String {
    if repeat {
        return "[routing: identical to the output this same command returned earlier — nothing \
                changed since. Work from that result.]"
            .into();
    }
    if let Some(file) = unchanged {
        return format!(
            "[routing: `{file}` is unchanged since your smart_read of it (same content_hash). If \
             that read covered these lines, reuse it; for other lines use smart_read `range`.]"
        );
    }
    let tools: Vec<&str> = observation
        .kinds
        .iter()
        .map(|k| match k {
            ObservationKind::Read => "smart_read (`range`/`search`)",
            ObservationKind::Search => "smart_grep (`context` for surrounding lines)",
            ObservationKind::List => "list_dir",
        })
        .collect();
    format!(
        "[routing: this command only looked at workspace files — {} does that with line \
         numbers and a content_hash, and batch_execute runs several in one call. Keep bash for \
         builds, tests, git and what the file tools cannot reach.]",
        tools.join(" / ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_read_header_yields_path_and_hash() {
        assert_eq!(
            read_header("src/a b.rs  (content_hash: 00ff)\n   1 │ x"),
            Some(("src/a b.rs", "00ff"))
        );
        assert_eq!(read_header("no header"), None);
    }

    #[test]
    fn equal_arguments_give_equal_keys_in_any_order() {
        let a: Args = serde_json::from_str(r#"{"path":"a.rs","start":1,"end":9}"#).unwrap();
        let b: Args = serde_json::from_str(r#"{"end":9,"start":1,"path":"a.rs"}"#).unwrap();
        assert_eq!(call_key("smart_read", &a), call_key("smart_read", &b));
    }
}
