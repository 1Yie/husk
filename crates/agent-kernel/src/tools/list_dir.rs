//! `list_dir` — bounded directory listing with an entry budget.
//!
//! Truncation table: "entry budget — 'too large to list fully' marker".

use std::sync::Arc;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Max entries returned before the fold marker.
const MAX_ENTRIES: usize = 500;

#[derive(Debug, serde::Serialize, Deserialize, schemars::JsonSchema)]
struct ListDirArgs {
    /// Workspace-relative directory path (default ".").
    path: Option<String>,
    /// Recurse depth (default 1 = direct children only).
    depth: Option<usize>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "list_dir",
        schema: schema_for::<ListDirArgs>(
            "List directory entries (dirs carry a trailing `/`). Respects \
             .gitignore. Bounded output — use smart_grep/smart_read for \
             targeted lookups instead of deep listings.",
        ),
        readonly: true,
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: ListDirArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("list_dir args: {e}")))?;
    let rel = parsed.path.as_deref().unwrap_or(".");
    let root = ctx.resolve(rel)?;
    if !root.is_dir() {
        return Err(ToolError::Failed(format!("not a directory: {rel}")));
    }
    let depth = parsed.depth.unwrap_or(1).min(4); // deep listings waste tokens

    let mut out = format!("{rel}/\n");
    let mut count = 0usize;
    let mut truncated = false;

    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .max_depth(Some(depth))
        .sort_by_file_name(|a, b| a.cmp(b));

    for result in builder.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.depth() == 0 {
            continue;
        }
        if count >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let rel_path = match entry.path().strip_prefix(&root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let indent = "  ".repeat(entry.depth().saturating_sub(1));
        let mut name = rel_path.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_some_and(|ft| ft.is_dir()) && !name.ends_with('/') {
            name.push('/');
        }
        out.push_str(&format!("{indent}{name}\n"));
        count += 1;
    }

    if truncated {
        out.push_str("… [too large to list fully — narrow the path or use smart_grep] …\n");
    }
    Ok(ToolResult::text(out))
}
