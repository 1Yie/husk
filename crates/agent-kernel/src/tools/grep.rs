//! `smart_grep` — in-process regex search via `grep-searcher`.
//!
//! Truncation table: 5 MiB stdout cap, 20 s timeout, count matches then
//! truncate lines. Scope-annotation ([inside impl X]) is a Phase-2 upgrade
//! once tree-sitter grammars land; Phase 1 emits `path:line │ text`.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::SearcherBuilder;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Stdout cap per truncation table.
const MAX_OUTPUT: usize = 5 * 1024 * 1024;
/// Match cap before folding (keeps the prompt small — a match *count* plus
/// first N lines is more useful than a wall of text).
const MAX_HITS: usize = 300;
/// Search wall-clock budget.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct GrepArgs {
    /// Regex or literal pattern.
    pattern: String,
    /// Subtree to search (default workspace root).
    path: Option<String>,
    /// File-glob filter, e.g. `*.rs` (default: all non-ignored files).
    glob: Option<String>,
    /// Treat `pattern` as a literal string, not regex.
    literal: Option<bool>,
    /// Case-insensitive matching.
    ignore_case: Option<bool>,
    /// Max hits to list (default 300).
    max_hits: Option<usize>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "smart_grep",
        schema: schema_for::<GrepArgs>(
            "Regex/literal search across the workspace (ripgrep-engine, in-process). \
             Returns `path:line │ match` lines plus a total count; respects \
             .gitignore. Prefer this over list_dir for locating code.",
        ),
        readonly: true,
        exec: |args, ctx| exec(args, ctx).boxed(),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: GrepArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("smart_grep args: {e}")))?;
    let base = ctx.resolve(parsed.path.as_deref().unwrap_or("."))?;
    let max_hits = parsed.max_hits.unwrap_or(MAX_HITS);

    let mut builder = grep_regex::RegexMatcherBuilder::new();
    if parsed.ignore_case.unwrap_or(false) {
        builder.case_insensitive(true);
    }
    if parsed.literal.unwrap_or(false) {
        builder.fixed_strings(true);
    }
    let matcher = builder
        .build(&parsed.pattern)
        .map_err(|e| ToolError::Args(format!("bad pattern: {e}")))?;

    let glob = parsed.glob.clone();
    let ws = ctx.workspace_root.clone();

    // grep-searcher is blocking IO — run on the blocking pool under a timeout.
    let handle = tokio::task::spawn_blocking(move || {
        search_sync(&base, &ws, &matcher, glob.as_deref(), max_hits)
    });
    match tokio::time::timeout(TIMEOUT, handle).await {
        Ok(Ok(res)) => res,
        Ok(Err(join_err)) => Err(ToolError::Failed(format!("search task: {join_err}"))),
        Err(_) => Err(ToolError::Failed(format!(
            "search exceeded {}s — narrow `path` or `glob`",
            TIMEOUT.as_secs()
        ))),
    }
}

fn search_sync(
    base: &std::path::Path,
    workspace: &std::path::Path,
    matcher: &RegexMatcher,
    glob: Option<&str>,
    max_hits: usize,
) -> Result<ToolResult, ToolError> {
    let mut out = String::new();
    let mut total_hits = 0usize;
    let mut bytes_written = 0usize;

    let mut walker = ignore::WalkBuilder::new(base);
    walker
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b));

    'walk: for result in walker.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if let Some(g) = glob {
            if !glob_match(g, &entry.path().to_string_lossy()) {
                continue;
            }
        }

        let rel = entry
            .path()
            .strip_prefix(workspace)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/")
            .to_string();

        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .build();
        let mut hits = Vec::new();
        let sink = UTF8(|line_no, line| {
            hits.push((line_no, line.trim_end().to_string()));
            Ok(total_hits < max_hits) // stop collecting past cap
        });
        let _ = searcher.search_path(matcher, entry.path(), sink);

        for (line_no, text) in hits {
            total_hits += 1;
            if total_hits <= max_hits {
                let line = format!("{rel}:{line_no} │ {text}\n");
                bytes_written += line.len();
                if bytes_written > MAX_OUTPUT {
                    out.push_str("… [5 MiB output cap reached] …\n");
                    break 'walk;
                }
                out.push_str(&line);
            }
        }
    }

    if total_hits == 0 {
        out.push_str("no matches\n");
    } else {
        let summary = format!("── {total_hits} match(es)");
        let summary = if total_hits > max_hits {
            format!("{summary}, first {max_hits} shown ──\n")
        } else {
            format!("{summary} ──\n")
        };
        out.push_str(&summary);
    }
    Ok(ToolResult::text(out))
}

/// Tiny `*.rs`-style glob — supports `*`/`?` on the filename part.
fn glob_match(glob: &str, path: &str) -> bool {
    let fname = path.rsplit('/').next().unwrap_or(path);
    if glob.contains('/') {
        wildcard_match(glob, path)
    } else {
        wildcard_match(glob, fname)
    }
}

fn wildcard_match(pat: &str, s: &str) -> bool {
    // Classic backtracking wildcard: `*` = any run, `?` = one char.
    let (p, s) = (pat.as_bytes(), s.as_bytes());
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star_p, mut star_s) = (usize::MAX, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_p = pi;
            star_s = si;
            pi += 1;
        } else if star_p != usize::MAX {
            pi = star_p + 1;
            star_s += 1;
            si = star_s;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}
