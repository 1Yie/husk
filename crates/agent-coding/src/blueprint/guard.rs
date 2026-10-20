//! The `.blueprint/` write guard.
//!
//! Node files are the architecture's normative record, so an edit to one is a
//! decision with structure — which module, which paths, which rules. The
//! generic patch tools express the same change as free text against a Markdown
//! file, which can leave a hand-edited frontmatter block the loader refuses.
//! [`super::edit`] is the sanctioned writer; this module makes that the only
//! writer by refusing patch calls that target a node file and naming the tool
//! to use instead.
//!
//! The wrapper is host-side on purpose: the file tools come from another
//! crate and know nothing about `.blueprint/`, and the kernel must not learn a
//! product concept. A host that assembles the blueprint layer wraps its patch
//! tools; a host that doesn't gets an unguarded registry.

use std::sync::Arc;

use futures::FutureExt;

use agent_kernel::tools::registry::{Args, ExecFn, ToolCtx, ToolError, ToolSpec};

use super::BLUEPRINT_DIR;

/// Wrap a patch tool so a call that targets `.blueprint/` is refused. Every
/// other call is handed to the wrapped tool untouched.
pub fn guard_patch(inner: ToolSpec) -> ToolSpec {
    let pass_through = inner.exec.clone();
    let exec: ExecFn = Arc::new(move |args: Args, ctx: Arc<ToolCtx>| {
        match targets(&args).into_iter().find(|p| is_blueprint_path(p)) {
            Some(path) => async move { Err(ToolError::Failed(refusal(&path))) }.boxed(),
            None => pass_through(args, ctx),
        }
    });
    ToolSpec { exec, ..inner }
}

/// The paths a patch call would write, read off the arguments the two patch
/// shapes use: `path` for a search/replace block, the `*** … File:` headers
/// for a multi-file patch. A meta-tool (`serena`) names its target one level
/// down, inside `arguments` — the same node file must be refused there too, or
/// the layer has a second writer.
fn targets(args: &Args) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
        out.push(path.to_string());
    }
    if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
        out.extend(patch_targets(patch));
    }
    if let Some(inner) = args.get("arguments").and_then(|v| v.as_object()) {
        // Every spelling Serena's own suite uses for a file it reads or writes.
        for key in ["path", "file", "file_path", "relative_path", "target_file"] {
            if let Some(p) = inner.get(key).and_then(|v| v.as_str()) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// The file-op headers after the `*** ` marker. Matched the way `apply_patch`
/// parses them — the colon ends the keyword and the path is trimmed — so a
/// header the tool accepts (`*** Add File:.blueprint/x.md`) is one the guard
/// sees too.
const HEADERS: [&str; 4] = ["Add File:", "Update File:", "Delete File:", "Move to:"];

fn patch_targets(patch: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.trim().strip_prefix("*** ") else {
            continue;
        };
        for header in HEADERS {
            if let Some(path) = rest.strip_prefix(header) {
                out.push(path.trim().to_string());
            }
        }
    }
    out
}

/// Whether a model-supplied path resolves into the blueprint directory —
/// relative, `./`-prefixed, or absolute inside the workspace.
fn is_blueprint_path(path: &str) -> bool {
    let norm = path.trim().replace('\\', "/");
    let norm = norm.trim_start_matches("./");
    norm == BLUEPRINT_DIR
        || norm.starts_with(&format!("{BLUEPRINT_DIR}/"))
        || norm.contains(&format!("/{BLUEPRINT_DIR}/"))
}

fn refusal(path: &str) -> String {
    format!(
        "`{path}` is inside `{BLUEPRINT_DIR}/` — architecture changes are not text patches. Use \
         `blueprint_edit` (`{{\"action\":\"create|update|delete\",\"node\":\"…\", \
         \"reason\":\"…\"}}`): it takes the node's fields as arguments, validates them, and \
         renders the file the loader expects. Read the node first with `blueprint` \
         `{{\"action\":\"read\",\"node\":\"…\"}}`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_replace_path_is_a_target() {
        let args = serde_json::json!({"path": ".blueprint/core.md", "search": "a", "replace": "b"});
        assert_eq!(targets(&args), [".blueprint/core.md"]);
    }

    #[test]
    fn every_patch_header_is_a_target() {
        let patch = "*** Begin Patch\n\
                     *** Add File: .blueprint/new.md\n+x\n\
                     *** Update File: src/lib.rs\n@@\n-a\n+b\n\
                     *** Delete File: .blueprint/old.md\n\
                     *** Move to: .blueprint/moved.md\n\
                     *** End Patch\n";
        let args = serde_json::json!({ "patch": patch });
        assert_eq!(
            targets(&args),
            [
                ".blueprint/new.md",
                "src/lib.rs",
                ".blueprint/old.md",
                ".blueprint/moved.md"
            ]
        );
    }

    /// `apply_patch` accepts a header with no space after the colon; the guard
    /// must read the same path or the header is a way around it. An indented
    /// header is read too — refusing more than the tool parses is safe.
    #[test]
    fn a_header_spelled_tightly_is_still_a_target() {
        let patch = "*** Begin Patch\n\
                     *** Add File:.blueprint/a.md\n+x\n\
                     \x20 *** Update File:\t.blueprint/b.md\n@@\n-a\n+b\n\
                     *** End Patch\n";
        let args = serde_json::json!({ "patch": patch });
        assert_eq!(targets(&args), [".blueprint/a.md", ".blueprint/b.md"]);
    }

    #[test]
    fn a_meta_tool_names_its_target_inside_arguments() {
        // `serena` is one dispatcher: the file lives in `arguments`, so a guard
        // that only reads the top level would leave a second writer in place.
        let args = serde_json::json!({
            "tool": "replace_symbol_body",
            "arguments": { "relative_path": ".blueprint/core.md", "name_path": "x" }
        });
        assert_eq!(targets(&args), [".blueprint/core.md"]);
    }

    #[test]
    fn blueprint_paths_are_recognised_in_every_spelling() {
        for p in [
            ".blueprint/core.md",
            "./.blueprint/core.md",
            "/home/me/proj/.blueprint/core.md",
            r".blueprint\core.md",
        ] {
            assert!(is_blueprint_path(p), "{p}");
        }
        for p in [
            "src/.blueprintx/a.rs",
            "crates/.blueprints/x.md",
            "README.md",
        ] {
            assert!(!is_blueprint_path(p), "{p}");
        }
    }

    /// The wrapper keeps the tool's metadata; only a blueprint target diverts.
    #[tokio::test]
    async fn a_blueprint_target_is_refused_before_the_tool_runs() {
        let inner = agent_tools_fs::fs_patch::spec();
        let guarded = guard_patch(inner);
        assert_eq!(guarded.name, "fuzzy_patch");
        assert!(guarded.staged);
        let ctx = Arc::new(ToolCtx::new());
        let err = (guarded.exec)(
            serde_json::json!({"path": ".blueprint/core.md", "search": "a", "replace": "b"}),
            ctx,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("blueprint_edit"), "{err}");
    }
}
