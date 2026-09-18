//! `apply_patch` — codex-style patch: `*** Add File`, `*** Delete File`,
//! `*** Update File` with `@@` context hunks, all inside one
//! `*** Begin Patch`/`*** End Patch` envelope.
//!
//! Like `fuzzy_patch` the tool STAGES the change: it returns one
//! `PendingWrite` per file op and never touches the filesystem, so the
//! engine can show the full multi-file diff on the approval card and only
//! commit after the permission decision.
//!
//! Grammar (a tolerant subset of the codex apply_patch format):
//! ```text
//! *** Begin Patch
//! *** Add File: path/to/new.rs
//! +line one
//! +line two
//! *** Delete File: path/old.rs
//! *** Update File: path/existing.rs
//! @@ optional context marker text
//!  context line to keep
//! -line to remove
//! +line to add
//! *** End Patch
//! ```

use std::sync::Arc;

use futures::FutureExt;
use serde::Deserialize;
use similar::TextDiff;

use super::registry::{schema_for, Args, PendingWrite, ToolCtx, ToolError, ToolResult, ToolSpec, WriteOp};

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct ApplyPatchArgs {
    /// The full patch text, `*** Begin Patch` … `*** End Patch`, containing
    /// one or more `*** Add File` / `*** Delete File` / `*** Update File`
    /// sections.
    patch: String,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "apply_patch",
        schema: schema_for::<ApplyPatchArgs>(
            "Apply a multi-file patch — codex-style `*** Add File` /\n\
             `*** Delete File` / `*** Update File` sections inside\n\
             `*** Begin Patch`/`*** End Patch`. Use this to CREATE new\n\
             files or folders (parents are made automatically), DELETE\n\
             files, or update several files in one call. `fuzzy_patch`\n\
             remains the right tool for a single in-place search/replace.",
        ),
        readonly: false,
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

/// One parsed file operation.
enum FileOp {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, hunks: Vec<Hunk> },
}

/// One `@@` hunk inside an `*** Update File` section: the leading-char
/// lines (` ` context, `-` remove, `+` add). `context` is the `@@ …`
/// locator text — codex uses it to disambiguate a `before` pattern that
/// matches several spots; `eof` marks a `*** End of File` append hunk.
#[derive(Debug)]
struct Hunk {
    context: Option<String>,
    lines: Vec<String>,
    eof: bool,
}

impl Hunk {
    fn new(context: Option<String>) -> Self {
        Self { context, lines: Vec::new(), eof: false }
    }
    fn implicit(line: String) -> Self {
        Self { context: None, lines: vec![line], eof: false }
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: ApplyPatchArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("apply_patch args: {e}")))?;

    let ops = parse_patch(&parsed.patch)?;
    if ops.is_empty() {
        return Err(ToolError::Args(
            "patch contained no file operations — expected `*** Add File:` / \
             `*** Delete File:` / `*** Update File:` sections".into(),
        ));
    }

    let mut summary = String::new();
    let mut writes: Vec<PendingWrite> = Vec::new();
    let mut any_fuzzy = false;

    for op in &ops {
        match op {
            FileOp::Add { path, content } => {
                let abs = ctx.resolve(path)?;
                // Diff from empty → everything is an add.
                let diff = unified_diff("", content);
                summary.push_str(&format!("added {path}\n\n{diff}\n"));
                writes.push(PendingWrite {
                    path: abs,
                    content: content.clone().into_bytes(),
                    op: WriteOp::Write,
                    auto_mkdir: true,
                });
            }
            FileOp::Delete { path } => {
                let abs = ctx.resolve(path)?;
                let old = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                let diff = unified_diff(&old, "");
                summary.push_str(&format!("deleted {path}\n\n{diff}\n"));
                writes.push(PendingWrite {
                    path: abs,
                    content: Vec::new(),
                    op: WriteOp::Delete,
                    auto_mkdir: false,
                });
            }
            FileOp::Update { path, hunks } => {
                let abs = ctx.resolve(path)?;
                let old = tokio::fs::read_to_string(&abs).await?;
                let (new, fuzzy) = apply_hunks(path, &old, hunks)?;
                any_fuzzy |= fuzzy;
                let diff = unified_diff(&old, &new);
                summary.push_str(&format!("updated {path}\n\n{diff}\n"));
                writes.push(PendingWrite {
                    path: abs,
                    content: new.into_bytes(),
                    op: WriteOp::Write,
                    auto_mkdir: false,
                });
            }
        }
    }

    Ok(ToolResult {
        content: summary.trim_end().to_string(),
        ui_type: Some("diff"),
        fuzzy: any_fuzzy,
        pending_write: writes,
    })
}

/// Split the patch envelope into file operations.
fn parse_patch(patch: &str) -> Result<Vec<FileOp>, ToolError> {
    let mut ops = Vec::new();
    let mut lines = patch.lines().peekable();

    // Skip to `*** Begin Patch` (tolerate a bare patch that omits the
    // envelope — the model sometimes drops the wrapper).
    while let Some(l) = lines.peek() {
        if l.trim() == "*** Begin Patch" {
            lines.next();
            break;
        }
        if l.trim().starts_with("*** Add File:")
            || l.trim().starts_with("*** Delete File:")
            || l.trim().starts_with("*** Update File:")
        {
            break; // bare patch — start parsing here
        }
        lines.next();
    }

    let mut cur_path: Option<String> = None;
    let mut cur_op: Option<&'static str> = None; // "add" | "delete" | "update"
    let mut add_lines: Vec<String> = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut cur_hunk: Option<Hunk> = None;

    // Flush the in-progress op into `ops`.
    macro_rules! flush {
        () => {
            if let Some(h) = cur_hunk.take() {
                hunks.push(h);
            }
            match cur_op.take() {
                Some("add") => {
                    if let Some(p) = cur_path.take() {
                        ops.push(FileOp::Add { path: p, content: add_lines.join("\n") });
                    }
                }
                Some("delete") => {
                    if let Some(p) = cur_path.take() {
                        ops.push(FileOp::Delete { path: p });
                    }
                }
                Some("update") => {
                    if let Some(p) = cur_path.take() {
                        ops.push(FileOp::Update { path: p, hunks: std::mem::take(&mut hunks) });
                    }
                }
                _ => {}
            }
            add_lines.clear();
            hunks.clear();
        };
    }

    for raw in lines {
        let l = raw.trim_end();
        if l == "*** End Patch" {
            break;
        }
        if let Some(rest) = l.strip_prefix("*** ") {
            // `*** End of File` marks the CURRENT hunk as an EOF append —
            // it does NOT flush the in-progress file op (no path of its
            // own), unlike Add/Delete/Update which each start a fresh op.
            if rest.starts_with("End of File") {
                if let Some(h) = cur_hunk.as_mut() {
                    h.eof = true;
                } else {
                    cur_hunk = Some(Hunk { context: None, lines: Vec::new(), eof: true });
                }
                continue;
            }
            flush!();
            if let Some(p) = rest.strip_prefix("Add File:") {
                cur_op = Some("add");
                cur_path = Some(p.trim().to_string());
            } else if let Some(p) = rest.strip_prefix("Delete File:") {
                cur_op = Some("delete");
                cur_path = Some(p.trim().to_string());
            } else if let Some(p) = rest.strip_prefix("Update File:") {
                cur_op = Some("update");
                cur_path = Some(p.trim().to_string());
            }
            // Move markers etc. are ignored.
            continue;
        }
        match cur_op {
            Some("add") => {
                // Add lines are `+`-prefixed; a bare line is tolerated too.
                add_lines.push(l.strip_prefix('+').unwrap_or(l).to_string());
            }
            Some("update") => {
                if l.starts_with("@@") {
                    if let Some(h) = cur_hunk.take() {
                        hunks.push(h);
                    }
                    // `@@ optional context marker` — keep the locator text;
                    // apply_hunks uses it to disambiguate a `before` that
                    // matches more than one spot.
                    let marker = l[2..].trim();
                    cur_hunk = Some(Hunk::new(
                        (!marker.is_empty()).then(|| marker.to_string()),
                    ));
                } else if l.starts_with(' ') || l.starts_with('-') || l.starts_with('+') {
                    if let Some(h) = cur_hunk.as_mut() {
                        h.lines.push(l.to_string());
                    } else {
                        // No `@@` header — start an implicit hunk so a
                        // context-free update still parses.
                        cur_hunk = Some(Hunk::implicit(l.to_string()));
                    }
                }
            }
            _ => {} // delete carries no body; pre-op junk ignored
        }
    }
    flush!();
    Ok(ops)
}

/// Apply `*** Update File` hunks to `old`. Each hunk's `-`/` ` lines are
/// the "before" pattern, `+`/` ` the "after". Reuses a fuzzy/whitespace
/// match so updates land even with drifted indentation.
///
/// Errors name the file, the hunk index, and a preview of the `before`
/// pattern so a failed patch is diagnosable without re-reading the tool
/// call (native-tools.md: errors must teach the fix).
fn apply_hunks(path: &str, old: &str, hunks: &[Hunk]) -> Result<(String, bool), ToolError> {
    let mut src = old.replace("\r\n", "\n");
    let mut fuzzy = false;
    for (idx, h) in hunks.iter().enumerate() {
        let hunk_desc = || format!("{path} hunk {}/{}", idx + 1, hunks.len());
        // Build before/after strings from the hunk lines.
        let mut before = String::new();
        let mut after = String::new();
        for l in &h.lines {
            match l.chars().next() {
                // `-` removes → before only. `+` adds → after only.
                // ` `/bare context → both. Each branch appends its own
                // `\n` so a `-` line can't leak a blank into `after`.
                Some('-') => {
                    before.push_str(&l[1..]);
                    before.push('\n');
                }
                Some('+') => {
                    after.push_str(&l[1..]);
                    after.push('\n');
                }
                _ => {
                    let ctx_line = l.strip_prefix(' ').unwrap_or(l);
                    before.push_str(ctx_line);
                    before.push('\n');
                    after.push_str(ctx_line);
                    after.push('\n');
                }
            }
        }
        let before = before.trim_end_matches('\n');
        let after = after.trim_end_matches('\n');

        // `*** End of File` — append `after` to the tail (no `before`
        // locator needed; codex uses it for trailing appends).
        if h.eof {
            if !src.is_empty() && !src.ends_with('\n') {
                src.push('\n');
            }
            src.push_str(after);
            if !after.is_empty() && !after.ends_with('\n') {
                src.push('\n');
            }
            continue;
        }

        if before.is_empty() {
            return Err(ToolError::Args(format!(
                "{} had no context/remove lines — \
                 each hunk needs ` ` or `-` lines to locate the change",
                hunk_desc()
            )));
        }
        let hits: Vec<usize> = src.match_indices(before).map(|(i, _)| i).collect();
        match hits.len() {
            1 => {
                // match_indices gave the byte offset — splice in place,
                // skipping replacen's second scan of the whole file.
                src.replace_range(hits[0]..hits[0] + before.len(), after);
            }
            n if n > 1 => {
                // Ambiguous `before` — the `@@ context` locator picks the
                // occurrence AT/AFTER the marker (codex semantics), else
                // report the ambiguity so the model adds context.
                let pick = h.context.as_deref().and_then(|marker| {
                    src.find(marker).and_then(|mpos| {
                        hits.iter().copied().find(|&i| i >= mpos)
                    })
                });
                match pick {
                    Some(i) => {
                        src.replace_range(i..i + before.len(), after);
                    }
                    None => {
                        return Err(ToolError::Failed(format!(
                            "{} matched {n} locations — add more context \
                             lines or an `@@ marker`. Pattern starts: {:?}",
                            hunk_desc(),
                            preview(before)
                        )));
                    }
                }
            }
            _ => {
                // Whitespace-tolerant fallback via the shared fuzzy helper.
                match super::fs_patch::apply(&src, before, after) {
                    Ok(res) => {
                        src = res.patched_content;
                        fuzzy |= res.matched_fuzzily;
                    }
                    Err(e) => {
                        return Err(ToolError::Failed(format!(
                            "{}: {e}. Pattern starts: {:?}",
                            hunk_desc(),
                            preview(before)
                        )))
                    }
                }
            }
        }
    }
    Ok((src, fuzzy))
}

/// First ~60 chars of a pattern, single-line, for error messages.
fn preview(s: &str) -> String {
    const MAX: usize = 60;
    let one_line = s.replace('\n', "⏎");
    match one_line.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}…", &one_line[..i]),
        None => one_line,
    }
}

fn unified_diff(old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header("a/file", "b/file")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_delete_update() {
        let patch = "\
*** Begin Patch
*** Add File: src/new.rs
+fn new() {}
*** Delete File: src/old.rs
*** Update File: src/lib.rs
@@
 context
-old_call()
+new_call()
*** End Patch";
        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn update_hunks_patch_source() {
        let old = "fn a() {}\nfn b() {}\n";
        let hunks = vec![Hunk {
            lines: vec![" fn a() {}".into(), "-fn b() {}".into(), "+fn c() {}".into()],
            context: None,
            eof: false,
        }];
        let (new, _) = apply_hunks("src/lib.rs", old, &hunks).unwrap();
        assert!(new.contains("fn c()"));
        assert!(!new.contains("fn b()"));
    }

    #[test]
    fn update_error_names_file_hunk_and_pattern() {
        let old = "fn a() {}\nfn b() {}\n";
        let hunks = vec![
            Hunk {
                lines: vec![" fn a() {}".into()],
                context: None,
                eof: false,
            },
            Hunk {
                lines: vec!["-fn missing() {}".into(), "+fn c() {}".into()],
                context: None,
                eof: false,
            },
        ];
        let err = apply_hunks("src/lib.rs", old, &hunks).unwrap_err().to_string();
        assert!(err.contains("src/lib.rs"), "missing path: {err}");
        assert!(err.contains("hunk 2/2"), "missing hunk index: {err}");
        assert!(err.contains("fn missing()"), "missing pattern preview: {err}");
    }

    #[test]
    fn ambiguous_hunk_error_names_location() {
        let old = "x\nx\n";
        let hunks = vec![Hunk {
            lines: vec!["-x".into(), "+y".into()],
            context: None,
            eof: false,
        }];
        let err = apply_hunks("f.txt", old, &hunks).unwrap_err().to_string();
        assert!(err.contains("f.txt hunk 1/1"), "missing location: {err}");
        assert!(err.contains("matched 2 locations"), "missing count: {err}");
    }

    #[test]
    fn context_marker_disambiguates_repeated_before() {
        // `before` ("x") appears twice — the `@@ second` marker picks the
        // occurrence at/after the marker instead of erroring.
        let old = "first\nx\nsecond\nx\n";
        let hunks = vec![Hunk {
            context: Some("second".into()),
            lines: vec!["-x".into(), "+y".into()],
            eof: false,
        }];
        let (new, _) = apply_hunks("f.txt", old, &hunks).unwrap();
        assert_eq!(new, "first\nx\nsecond\ny\n");
    }

    #[test]
    fn end_of_file_appends() {
        let old = "fn a() {}\n";
        let hunks = vec![Hunk {
            context: None,
            lines: vec!["+fn tail() {}".into()],
            eof: true,
        }];
        let (new, _) = apply_hunks("f.rs", old, &hunks).unwrap();
        assert_eq!(new, "fn a() {}\nfn tail() {}\n");
    }
}
