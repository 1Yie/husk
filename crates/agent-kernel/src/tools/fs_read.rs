//! `smart_read` — structured file reads that never `cat`.
//!
//! Three modes (native-tools.md): `outline` (tree-sitter skeleton — Stage 3
//! ships a regex-based signature extractor; tree-sitter grammars land with
//! Phase 2), `range` (numbered lines + `content_hash`), `search` (matching
//! lines with numbers).
//!
//! Every response carries `content_hash` (xxh3 of the read window) so
//! `fuzzy_patch` can detect drift between read and write.

use std::sync::Arc;

use futures::FutureExt;
use serde::Deserialize;
use xxhash_rust::xxh3::xxh3_64;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// `range` mode hard cap per the truncation table (1000 lines).
const MAX_RANGE_LINES: usize = 1000;
/// `search` mode cap on matching lines returned.
const MAX_SEARCH_HITS: usize = 200;

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct SmartReadArgs {
    /// Workspace-relative path to the file.
    path: String,
    /// `outline` | `range` | `search` (default `range` when start/end given).
    mode: Option<String>,
    /// 1-based first line for `range` mode.
    start: Option<usize>,
    /// 1-based last line for `range` mode (inclusive).
    end: Option<usize>,
    /// Substring or regex pattern for `search` mode.
    pattern: Option<String>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "smart_read",
        schema: schema_for::<SmartReadArgs>(
            "Read a file structurally: `outline` returns a signature skeleton \
             (~100 tokens for large files), `range` returns numbered lines with \
             a content_hash for patch anchoring, `search` returns matching \
             lines with numbers. Prefer outline→range over dumping whole files.",
        ),
        readonly: true,
        exec: |args, ctx| exec(args, ctx).boxed(),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: SmartReadArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("smart_read args: {e}")))?;
    let path = ctx.resolve(&parsed.path)?;
    let content = tokio::fs::read_to_string(&path).await?;
    let hash = format!("{:016x}", xxh3_64(content.as_bytes()));

    let mode = parsed.mode.as_deref().unwrap_or_else(|| {
        if parsed.pattern.is_some() {
            "search"
        } else {
            "range"
        }
    });

    match mode {
        "outline" => Ok(ToolResult::text(outline(&parsed.path, &content, &hash))),
        "search" => {
            let pat = parsed.pattern.as_deref().ok_or_else(|| {
                ToolError::Args("search mode requires 'pattern'".into())
            })?;
            Ok(ToolResult::text(search(&parsed.path, &content, pat, &hash)))
        }
        _ => Ok(ToolResult::text(range(&parsed.path, &content, parsed.start, parsed.end, &hash))),
    }
}

fn header(path: &str, hash: &str) -> String {
    format!("{path}  (content_hash: {hash})\n")
}

/// `range` mode: numbered `NNNN │ code` lines, hard cap + continuation hint.
fn range(path: &str, content: &str, start: Option<usize>, end: Option<usize>, hash: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let s = start.unwrap_or(1).max(1);
    let e = end.unwrap_or(total).min(total).min(s + MAX_RANGE_LINES - 1);
    if s > total {
        return format!(
            "{header}{path}: {total} lines total; requested start {s} is past EOF",
            header = header(path, hash)
        );
    }
    let mut out = header(path, hash);
    for (i, line) in lines[s - 1..e].iter().enumerate() {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("{:>4} │ {}\n", s + i, line),
        );
    }
    if e < total {
        out.push_str(&format!("… [truncated — {total} lines total, continue with start={}] …\n", e + 1));
    }
    out
}

/// `search` mode: matching lines with numbers, capped.
fn search(path: &str, content: &str, pattern: &str, hash: &str) -> String {
    let re = regex_lite(pattern);
    let mut out = header(path, hash);
    let mut hits = 0usize;
    for (i, line) in content.lines().enumerate() {
        let matched = match &re {
            Some(r) => r.is_match(line),
            None => line.contains(pattern),
        };
        if matched {
            hits += 1;
            if hits <= MAX_SEARCH_HITS {
                out.push_str(&format!("{:>4} │ {}\n", i + 1, line));
            }
        }
    }
    if hits == 0 {
        out.push_str("no matches\n");
    } else if hits > MAX_SEARCH_HITS {
        out.push_str(&format!("… [{hits} matches total, first {MAX_SEARCH_HITS} shown] …\n"));
    }
    out
}

/// Cheap regex via `grep-regex` if it compiles, else literal substring.
fn regex_lite(pattern: &str) -> Option<regex::Regex> {
    regex::Regex::new(pattern).ok()
}

/// `outline` mode: signature skeleton. Regex-based Phase-1 extraction —
/// `fn`/`struct`/`enum`/`trait`/`impl`/`pub` lines + common JS/TS/Python
/// signatures; bodies folded. Tree-sitter grammars land in Phase 2.
fn outline(path: &str, content: &str, hash: &str) -> String {
    let mut out = header(path, hash);
    let mut emitted = 0usize;
    let mut prev_blank = false;
    for (i, line) in content.lines().enumerate() {
        let t = line.trim_end();
        let is_sig = looks_like_signature(t);
        if is_sig {
            // Attach a preceding doc comment line if adjacent.
            if prev_blank && i > 0 {
                let prev = content.lines().nth(i - 1).unwrap_or("").trim();
                if prev.starts_with("///") || prev.starts_with('#') || prev.starts_with("*") || prev.starts_with("//") {
                    out.push_str(&format!("{:>4} │ {}\n", i, prev));
                }
            }
            out.push_str(&format!("{:>4} │ {}\n", i + 1, t));
            emitted += 1;
            prev_blank = false;
        } else {
            prev_blank = t.trim().is_empty() || t.trim_start().starts_with("//") || t.trim_start().starts_with('#');
        }
        if emitted >= 400 {
            out.push_str("… [outline truncated] …\n");
            break;
        }
    }
    if emitted == 0 {
        out.push_str("(no recognizable signatures — use mode=range)\n");
    }
    out
}

/// Phase-1 heuristic: does this line open a named item?
fn looks_like_signature(trimmed: &str) -> bool {
    let t = trimmed.trim_start();
    if t.starts_with("//") || t.starts_with('#') && !t.starts_with("#[") {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "pub fn", "pub async fn", "async fn", "fn ", "pub struct", "struct ",
        "pub enum", "enum ", "pub trait", "trait ", "impl ", "pub impl",
        "pub const", "pub static", "pub type", "pub mod", "mod ",
        "def ", "class ", "async def ",
        "function ", "const ", "export ", "interface ", "type ",
        "public ", "private ", "protected ", "static ",
    ];
    PREFIXES.iter().any(|p| t.starts_with(p))
        || t.starts_with("#[")
        // impl blocks without `pub`: `impl Foo {`, `impl Trait for Foo`
        || (t.starts_with("impl") && t.contains('{'))
}

// `regex` isn't a workspace dep — use grep-regex's matcher or plain substring.
// Kept tiny: try `grep_regex` through a thin shim so `search` mode gets real
// regex without pulling another crate into the dep graph.
mod regex {
    /// Minimal wrapper: compile as regex, expose `is_match`.
    pub struct Regex(grep_regex::RegexMatcher);

    impl Regex {
        pub fn new(pat: &str) -> Result<Self, ()> {
            grep_regex::RegexMatcher::new(pat)
                .map(Regex)
                .map_err(|_| ())
        }

        pub fn is_match(&self, line: &str) -> bool {
            use grep_matcher::Matcher;
            self.0.is_match(line.as_bytes()).unwrap_or(false)
        }
    }
}
