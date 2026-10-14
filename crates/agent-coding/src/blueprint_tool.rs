//! `blueprint` — read access to the workspace's `.blueprint/` architecture
//! layer (see `agent_context::blueprint`).
//!
//! Readonly in every mode, plan and subagents included: locating the
//! architecture before reading code is exactly what a planner needs. Writes
//! to node files go through `fuzzy_patch`/`apply_patch` on purpose — an
//! architecture change then rides the same diff review as a code change.

use std::sync::Arc;

use agent_context::blueprint::{render_node, Blueprint, BLUEPRINT_DIR};
use futures::FutureExt;
use serde::Deserialize;

use agent_kernel::tools::registry::{schema_for, Args, ToolClass, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Direct hits `locate` returns at most.
const LOCATE_LIMIT: usize = 5;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Action {
    /// Nodes matching `query` and/or owning `paths`, plus their one-hop
    /// neighbours in the derived dependency graph.
    Locate,
    /// One node in full (`node` required).
    Read,
    /// Rule check over the whole workspace — forbidden dependencies,
    /// files no node owns, unloadable node files.
    Check,
    /// The derived dependency graph as Mermaid.
    Graph,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BlueprintArgs {
    action: Action,
    /// `locate`: free-text terms matched against node names and bodies.
    #[serde(default)]
    query: Option<String>,
    /// `locate`: workspace-relative paths — the nodes owning them rank first.
    #[serde(default)]
    paths: Vec<String>,
    /// `read`: node name.
    #[serde(default)]
    node: Option<String>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "blueprint",
        schema: schema_for::<BlueprintArgs>(
            "Query the workspace's architecture blueprint (`.blueprint/`): which node owns a \
             change, what it is responsible for, what it must never depend on, and the \
             dependency graph derived live from package manifests. Call `locate` BEFORE \
             reading code for any task that adds behaviour or crosses modules, then read \
             only the paths the matched nodes own. Edit node files with the normal patch \
             tools.",
        ),
        readonly: true,
        class: ToolClass::Observation,
        network: false,
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: BlueprintArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("blueprint args: {e}")))?;
    let root = ctx.workspace_root.clone();
    // Manifest + file-index walks are blocking I/O.
    tokio::task::spawn_blocking(move || run(&root, parsed))
        .await
        .map_err(|e| ToolError::Failed(format!("blueprint task: {e}")))?
}

fn run(root: &std::path::Path, args: BlueprintArgs) -> Result<ToolResult, ToolError> {
    let Some(bp) = Blueprint::load(root) else {
        return Err(ToolError::Failed(format!(
            "this workspace has no `{BLUEPRINT_DIR}/` directory — the blueprint layer is off. \
             The user can bootstrap one with `/blueprint init`."
        )));
    };
    let text = match args.action {
        Action::Locate => {
            let query = args.query.unwrap_or_default();
            if query.trim().is_empty() && args.paths.is_empty() {
                return Err(ToolError::Args("`locate` needs `query` or `paths`".into()));
            }
            let located = bp.locate(&query, &args.paths, LOCATE_LIMIT);
            if located.hits.is_empty() {
                // A miss is usually vocabulary (the nodes are written in one
                // language, the query in another) — show every node instead
                // of leaving the model to conclude none is relevant.
                format!("{}\nall nodes:\n{}", located.render(), bp.index_block())
            } else {
                located.render()
            }
        }
        Action::Read => {
            let name = args
                .node
                .ok_or_else(|| ToolError::Args("`read` needs `node`".into()))?;
            let node = bp.node(&name).ok_or_else(|| {
                let known: Vec<&str> = bp.nodes.iter().map(|n| n.name.as_str()).collect();
                ToolError::Failed(format!("no node `{name}` — known: {}", known.join(", ")))
            })?;
            render_node(node, &bp.graph())
        }
        Action::Check => bp.check().render(),
        Action::Graph => bp.graph().mermaid(&bp.nodes),
    };
    Ok(ToolResult::text(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(root: &std::path::Path) -> Arc<ToolCtx> {
        Arc::new(ToolCtx::new(root))
    }

    #[tokio::test]
    async fn absent_blueprint_is_a_clear_failure() {
        let d = tempfile::tempdir().unwrap();
        let err = exec(serde_json::json!({"action": "check"}), ctx(d.path()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("/blueprint init"), "{err}");
    }

    #[tokio::test]
    async fn locate_and_read_render_nodes() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join(".blueprint")).unwrap();
        std::fs::write(
            r.join(".blueprint/auth.md"),
            "+++\nowns = [\"src/auth/**\"]\nforbidden_deps = [\"db\"]\n+++\nIssues and checks sessions.\n",
        )
        .unwrap();
        let out = exec(
            serde_json::json!({"action": "locate", "paths": ["src/auth/login.rs"]}),
            ctx(r),
        )
        .await
        .unwrap();
        assert!(out.content.contains("## node `auth`"), "{}", out.content);
        let out = exec(
            serde_json::json!({"action": "read", "node": "auth"}),
            ctx(r),
        )
        .await
        .unwrap();
        assert!(
            out.content.contains("forbidden deps: db"),
            "{}",
            out.content
        );
        let out = exec(
            serde_json::json!({"action": "locate", "query": "zzz-no-such-term"}),
            ctx(r),
        )
        .await
        .unwrap();
        assert!(out.content.contains("all nodes:"), "{}", out.content);
        assert!(out.content.contains("- auth —"), "{}", out.content);
        let err = exec(
            serde_json::json!({"action": "read", "node": "nope"}),
            ctx(r),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("known: auth"), "{err}");
    }
}
