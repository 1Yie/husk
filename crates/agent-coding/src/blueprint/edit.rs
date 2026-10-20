//! `blueprint_edit` — the only writer of `.blueprint/` node files.
//!
//! An architecture change is a structured edit, not a text patch. The node's
//! normative facts (`owns`, `forbidden_deps`, `invariants`) are arguments the
//! tool validates, and the file it renders is the exact text
//! [`super::parse_node_file`] reads back. The generic patch tools refuse
//! `.blueprint/` paths for the same reason — [`super::guard_patch`] enforces
//! that side.
//!
//! Staged like every other writer: `exec` validates and renders, the engine
//! shows the diff, and the commit lands only after the permission decision.

use std::path::Path;
use std::sync::Arc;

use agent_capabilities::WorkspaceCtx;
use futures::FutureExt;
use serde::Deserialize;

use agent_kernel::tools::registry::{
    schema_for, Args, Review, ToolClass, ToolCtx, ToolError, ToolResult, ToolSpec,
};

use agent_tools_fs::commit::{commit_writes, FileWrite, WriteObserver, WriteOp};

use super::{
    glob, parse_node_file, render_node_file, Blueprint, Frontmatter, NodeParse, BLUEPRINT_DIR,
};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Action {
    /// A node that does not exist yet (`.blueprint/<node>.md`).
    Create,
    /// Change the fields given — omitted fields keep their current value.
    Update,
    /// Remove the node file; the paths it owned must be claimed elsewhere.
    Delete,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EditArgs {
    action: Action,
    /// Node name — also the file stem (`.blueprint/<node>.md`). Letters,
    /// digits, `-` and `_` only.
    node: String,
    /// Workspace-relative globs this module is responsible for. Required on
    /// `create`; replaces the list wholesale on `update`.
    #[serde(default)]
    owns: Option<Vec<String>>,
    /// Package names or node names this module must never depend on.
    #[serde(default)]
    forbidden_deps: Option<Vec<String>>,
    /// Rules the code must always keep.
    #[serde(default)]
    invariants: Option<Vec<String>>,
    /// The prose after the frontmatter — `## 职责` / `## 不做` / `## 对外契约`.
    /// Its first line is the entry the prompt index shows.
    #[serde(default)]
    body: Option<String>,
    /// Why the architecture moved — echoed back so the turn's summary can say
    /// what changed and why.
    #[serde(default)]
    reason: Option<String>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "blueprint_edit",
        schema: schema_for::<EditArgs>(
            "Create, update or delete ONE architecture node in the workspace's `.blueprint/` \
             layer. This is the only sanctioned writer of node files: the patch tools \
             (`apply_patch`, `fuzzy_patch`) refuse `.blueprint/` paths. Use it BEFORE the code \
             change when a task adds a module, moves a responsibility, changes a node's \
             contract, or adds a dependency between nodes. The tool validates the node (globs \
             compile, the file loads), shows the diff for review, and writes only after \
             approval. Fields `owns` / `forbidden_deps` / `invariants` are replaced wholesale \
             on `update`; omit a field to keep it. `body` carries the prose sections. State \
             `reason` — the user is reviewing an architecture decision, not an edit.",
        ),
        readonly: false,
        class: ToolClass::WorkspaceMutation,
        network: false,
        staged: true,
        review: Some(Arc::new(review)),
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

/// The approval card's one line: which node file, which action.
fn review(args: &Args, _ctx: &Arc<ToolCtx>) -> Review {
    let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("?");
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("edit");
    Review {
        summary: Some(format!("`{BLUEPRINT_DIR}/{node}.md` — {action}")),
        ..Review::default()
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: EditArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(format!("blueprint_edit args: {e}")))?;
    let root = ctx.root()?.to_path_buf();
    let observer = ctx
        .ext
        .get::<agent_tools_fs::commit::WriteObserverSlot>()
        .map(|s| s.0.clone());
    // Node files are small and the walk is blocking I/O.
    tokio::task::spawn_blocking(move || run(&root, parsed, observer))
        .await
        .map_err(|e| ToolError::Failed(format!("blueprint_edit task: {e}")))?
}

fn run(
    root: &Path,
    args: EditArgs,
    observer: Option<Arc<dyn WriteObserver>>,
) -> Result<ToolResult, ToolError> {
    let EditArgs {
        action,
        node,
        owns,
        forbidden_deps,
        invariants,
        body,
        reason,
    } = args;
    let name = node.trim().to_string();
    validate_name(&name)?;
    let blueprint = Blueprint::load(root);
    if blueprint.is_none() && !matches!(action, Action::Create) {
        return Err(ToolError::Failed(format!(
            "this workspace has no `{BLUEPRINT_DIR}/` — bootstrap one with `/blueprint init`"
        )));
    }
    let existing = blueprint.as_ref().and_then(|bp| bp.node(&name).cloned());
    let rel = existing
        .as_ref()
        .map_or_else(|| format!("{BLUEPRINT_DIR}/{name}.md"), |n| n.file.clone());
    let path = root.join(&rel);

    let (fields, prose, op) = match action {
        Action::Create => {
            if existing.is_some() {
                return Err(ToolError::Failed(format!(
                    "node `{name}` already exists (`{rel}`) — use `{{\"action\":\"update\"}}` to change it"
                )));
            }
            // The lookup is by node name, the file by stem: a file can sit at
            // `<name>.md` while declaring another name (`node = "…"`), and
            // writing over it would silently delete that node.
            if path.exists() {
                let owner = blueprint
                    .as_ref()
                    .and_then(|bp| bp.nodes.iter().find(|n| n.file == rel))
                    .map(|n| format!(" — it holds node `{}`", n.name))
                    .unwrap_or_default();
                return Err(ToolError::Failed(format!(
                    "`{rel}` already exists{owner}; pick another node name or update that node"
                )));
            }
            let owns = clean(owns.unwrap_or_default());
            if owns.is_empty() {
                return Err(ToolError::Args(format!(
                    "`create` needs `owns` — the workspace paths node `{name}` is responsible \
                     for, e.g. [\"crates/{name}/**\"]"
                )));
            }
            let fm = Frontmatter {
                node: None,
                owns,
                forbidden_deps: clean(forbidden_deps.unwrap_or_default()),
                invariants: clean(invariants.unwrap_or_default()),
            };
            (fm, body.unwrap_or_default(), WriteOp::Write)
        }
        Action::Update => {
            let node = existing
                .as_ref()
                .ok_or_else(|| unknown_node(&name, &blueprint))?;
            let text = std::fs::read_to_string(&path)?;
            let (mut fm, old_body) = match parse_node_file(&path, &text) {
                NodeParse::Node(fm, body) => (fm, body),
                NodeParse::Malformed(msg) => {
                    return Err(ToolError::Failed(format!("{}: {msg}", node.file)))
                }
                NodeParse::Prose => {
                    return Err(ToolError::Failed(format!(
                        "`{}` is not a node file",
                        node.file
                    )))
                }
            };
            if let Some(v) = owns {
                fm.owns = clean(v);
            }
            if let Some(v) = forbidden_deps {
                fm.forbidden_deps = clean(v);
            }
            if let Some(v) = invariants {
                fm.invariants = clean(v);
            }
            (fm, body.unwrap_or(old_body), WriteOp::Write)
        }
        Action::Delete => {
            existing
                .as_ref()
                .ok_or_else(|| unknown_node(&name, &blueprint))?;
            (Frontmatter::default(), String::new(), WriteOp::Delete)
        }
    };

    let mut advisories = Vec::new();
    let content = match op {
        WriteOp::Delete => {
            advisories.push(format!(
                "paths only `{name}` owned now belong to no node — claim them in another node"
            ));
            Vec::new()
        }
        WriteOp::Write => {
            validate_node(&fields, &name)?;
            if fields.owns.is_empty() {
                advisories.push("node owns no paths — `blueprint` `check` will warn".into());
            }
            let text = render_node_file(&fields, &prose);
            // What we are about to write must be what the loader reads back.
            match parse_node_file(&path, &text) {
                NodeParse::Node(..) => {}
                NodeParse::Malformed(msg) => {
                    return Err(ToolError::Failed(format!(
                        "the node this edit produces does not load: {msg}"
                    )))
                }
                NodeParse::Prose => {
                    return Err(ToolError::Failed(
                        "the node this edit produces would not be read as a node".into(),
                    ))
                }
            }
            text.into_bytes()
        }
    };

    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let diff = unified(&old, &String::from_utf8_lossy(&content), &rel);
    let verb = match (op, existing.is_some()) {
        (WriteOp::Delete, _) => "deleted",
        (WriteOp::Write, true) => "updated",
        (WriteOp::Write, false) => "created",
    };
    let mut summary = format!("{verb} `{rel}`");
    if let Some(reason) = reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        summary.push_str(&format!(" — {reason}"));
    }
    summary.push('\n');
    for a in &advisories {
        summary.push_str(&format!("! {a}\n"));
    }
    summary.push('\n');
    summary.push_str(&diff);

    let write = FileWrite {
        path,
        content,
        op,
        // `/blueprint init` creates the first node before the directory exists.
        auto_mkdir: true,
    };
    Ok(ToolResult {
        content: summary,
        ui_type: Some("diff"),
        fuzzy: false,
        commit: commit_writes(vec![write], observer),
        images: Vec::new(),
    })
}

/// A node name becomes a file name, so it is restricted before anything is
/// read or written.
fn validate_name(name: &str) -> Result<(), ToolError> {
    const MAX_NAME: usize = 64;
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !name.eq_ignore_ascii_case("readme");
    if ok {
        Ok(())
    } else {
        Err(ToolError::Args(format!(
            "`{name}` is not a usable node name — letters, digits, `-` and `_` only (the node \
             is the file stem `{BLUEPRINT_DIR}/<node>.md`; `README` is reserved for prose)"
        )))
    }
}

/// Trims each entry and drops the empty ones — a rule list never carries
/// blanks that would silently never match.
fn clean(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Rules that would make the node contradict itself, checked before the write
/// so the mistake costs a rejection instead of a rework round.
fn validate_node(fm: &Frontmatter, name: &str) -> Result<(), ToolError> {
    for pat in &fm.owns {
        glob(pat).map_err(|e| {
            ToolError::Failed(format!("`owns` entry `{pat}` is not a valid glob: {e}"))
        })?;
    }
    let mut banned = globset::GlobSetBuilder::new();
    for pat in &fm.forbidden_deps {
        let g = globset::Glob::new(pat).map_err(|e| {
            ToolError::Failed(format!(
                "`forbidden_deps` entry `{pat}` is not a valid glob: {e}"
            ))
        })?;
        banned.add(g);
    }
    let banned = banned
        .build()
        .map_err(|e| ToolError::Failed(format!("`forbidden_deps`: {e}")))?;
    if banned.is_match(name) {
        return Err(ToolError::Args(format!(
            "a node cannot forbid itself — drop `{name}` from `forbidden_deps`"
        )));
    }
    Ok(())
}

fn unknown_node(name: &str, blueprint: &Option<Blueprint>) -> ToolError {
    let known: Vec<&str> = blueprint
        .as_ref()
        .map(|bp| bp.nodes.iter().map(|n| n.name.as_str()).collect())
        .unwrap_or_default();
    if known.is_empty() {
        ToolError::Failed(format!(
            "no node `{name}` — this blueprint has no nodes yet, use `{{\"action\":\"create\"}}`"
        ))
    } else {
        ToolError::Failed(format!("no node `{name}` — known: {}", known.join(", ")))
    }
}

fn unified(old: &str, new: &str, rel: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{rel}"), &format!("b/{rel}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(root: &Path) -> Arc<ToolCtx> {
        Arc::new(ToolCtx::new().with_workspace(root))
    }

    fn seed(root: &Path) {
        std::fs::create_dir_all(root.join(BLUEPRINT_DIR)).unwrap();
        std::fs::write(
            root.join(".blueprint/core.md"),
            "+++\nowns = [\"src/**\"]\nforbidden_deps = [\"ui\"]\n+++\nCore engine.\n",
        )
        .unwrap();
    }

    async fn edit(root: &Path, args: serde_json::Value) -> Result<ToolResult, ToolError> {
        exec(args, ctx(root)).await
    }

    /// `exec` validates and stages; the engine commits. The file stays
    /// untouched until the commit runs, so the approval card shows a change
    /// that has not happened yet.
    #[tokio::test]
    async fn create_stages_a_render_the_loader_accepts() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        seed(r);
        let out = edit(
            r,
            serde_json::json!({
                "action": "create",
                "node": "web",
                "owns": ["web/**"],
                "forbidden_deps": ["tauri*"],
                "invariants": ["no backend imports"],
                "body": "## 职责\nThe web shell.",
                "reason": "new UI package",
            }),
        )
        .await
        .unwrap();
        assert!(
            out.content.contains("created `.blueprint/web.md`"),
            "{}",
            out.content
        );
        assert!(out.content.contains("new UI package"), "{}", out.content);
        assert!(!r.join(".blueprint/web.md").exists(), "staged, not written");
        assert!(out.commit.is_some());

        // Render the same node through the loader's own parser.
        let text = render_node_file(
            &Frontmatter {
                node: None,
                owns: vec!["web/**".into()],
                forbidden_deps: vec!["tauri*".into()],
                invariants: vec!["no backend imports".into()],
            },
            "## 职责\nThe web shell.",
        );
        let p = r.join(".blueprint/web.md");
        match parse_node_file(&p, &text) {
            NodeParse::Node(fm, body) => {
                assert_eq!(fm.owns, ["web/**"]);
                assert_eq!(body, "## 职责\nThe web shell.");
            }
            _ => panic!("rendered node must load: {text}"),
        }
    }

    #[tokio::test]
    async fn update_keeps_omitted_fields_and_rewrites_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        seed(r);
        let out = edit(
            r,
            serde_json::json!({"action": "update", "node": "core", "owns": ["src/**", "lib/**"]}),
        )
        .await
        .unwrap();
        assert!(
            out.content.contains("updated `.blueprint/core.md`"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("+owns = [\"src/**\", \"lib/**\"]"),
            "{}",
            out.content
        );
        // Untouched fields survive the rewrite: they appear as context, never
        // as a removal.
        assert!(
            out.content.contains("forbidden_deps = [\"ui\"]"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("-forbidden_deps"), "{}", out.content);
    }

    #[tokio::test]
    async fn create_bootstraps_the_directory() {
        let d = tempfile::tempdir().unwrap();
        let out = edit(
            d.path(),
            serde_json::json!({"action": "create", "node": "core", "owns": ["src/**"], "body": "Core."}),
        )
        .await
        .unwrap();
        assert!(out.commit.is_some(), "commit staged");
        assert!(out.content.contains("created"), "{}", out.content);
    }

    #[tokio::test]
    async fn rejections_teach_the_fix() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        seed(r);
        // Duplicate create.
        let err = edit(
            r,
            serde_json::json!({"action": "create", "node": "core", "owns": ["x/**"]}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        // Unknown node on update.
        let err = edit(r, serde_json::json!({"action": "update", "node": "nope"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("known: core"), "{err}");
        // Missing owns on create.
        let err = edit(r, serde_json::json!({"action": "create", "node": "web"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("needs `owns`"), "{err}");
        // A node cannot forbid itself.
        let err = edit(
            r,
            serde_json::json!({"action": "update", "node": "core", "forbidden_deps": ["co*"]}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("forbid itself"), "{err}");
        // A glob that does not compile.
        let err = edit(
            r,
            serde_json::json!({"action": "update", "node": "core", "owns": ["src/[**"]}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not a valid glob"), "{err}");
        // A name that is not a file stem.
        let err = edit(
            r,
            serde_json::json!({"action": "create", "node": "../escape", "owns": ["x/**"]}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not a usable node name"), "{err}");
        // A file at `<name>.md` that declares another node name is not free.
        std::fs::write(
            r.join(".blueprint/web.md"),
            "+++\nnode = \"frontend\"\nowns = [\"web/**\"]\n+++\nUI.\n",
        )
        .unwrap();
        let err = edit(
            r,
            serde_json::json!({"action": "create", "node": "web", "owns": ["x/**"]}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("holds node `frontend`"), "{err}");
        // `update` in a workspace with no `.blueprint/`.
        let empty = tempfile::tempdir().unwrap();
        let err = edit(
            empty.path(),
            serde_json::json!({"action": "update", "node": "core"}),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("/blueprint init"), "{err}");
    }
}
