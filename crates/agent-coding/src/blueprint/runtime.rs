//! Session integration for the `.blueprint/` architecture layer — the prompt
//! block, the turn-end check, and the `/blueprint` command. The data model
//! and rules live in the parent module; this module decides when the coding
//! policy consults them.
//!
//! Off by construction for workspaces without `.blueprint/`: every entry
//! point loads first and returns the empty/neutral value on `None`, so the
//! feature costs one `is_dir` per turn there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{Blueprint, Report, BLUEPRINT_DIR};

/// Load the blueprint for a session. `enabled` is that session's switch
/// (settings 偏好, copied into its config) — it lives on the session, not in
/// process-global state, so one session's toggle never reaches another.
fn load(root: &Path, enabled: bool) -> Option<Blueprint> {
    if !enabled {
        return None;
    }
    Blueprint::load(root)
}

/// The `{{BLUEPRINT_BLOCK}}` region — the contract plus the node index, or
/// a one-line "off" marker. Rebuilt every turn by the session but swapped
/// into the system prompt only when the text changed, so the cached prompt
/// prefix survives every turn that didn't edit `.blueprint/`.
pub fn prompt_block(root: &Path, enabled: bool) -> String {
    let Some(bp) = load(root, enabled) else {
        return "(no `.blueprint/` in this workspace — architecture layer off)".into();
    };
    format!(
        "This workspace keeps its architecture in `{BLUEPRINT_DIR}/` — one node per \
         module: what it owns, what it is for, what it must never depend on. The \
         dependency graph is derived live from package manifests, never written down.\n\
         \n\
         - **Locate first**: for any task that adds behaviour, crosses modules or moves \
           a responsibility, call `blueprint` (`locate`) before reading code, then read \
           only the paths the matched nodes own. A one-line fix in a known file needs \
           no lookup.\n\
         - **Architecture before code**: when a change adds a module, moves a \
           responsibility, changes a node's contract or adds a dependency between \
           nodes, edit the node file first (`fuzzy_patch`), then the code. A new file \
           must fall under some node's `owns`.\n\
         - **Rules are hard**: never add a dependency a node's `forbidden_deps` rules \
           out or break one of its `invariants`; if the task genuinely needs it, say \
           so and change the node file deliberately — the user reviews that diff.\n\
         - Node files hold intent, not history: no changelog lines, no \"fixed X\" \
           notes. The same goes for code comments.\n\
         \n\
         Nodes:\n{}",
        bp.index_block()
    )
}

/// The workspace as the turn found it, taken before the turn's first write:
/// each file's mtime and size, plus the full check's findings. The turn end
/// diffs against it, so it reports only what this turn introduced.
#[derive(Debug, Clone)]
pub struct Baseline {
    files: BTreeMap<String, (Option<SystemTime>, u64)>,
    report: Report,
}

/// Baseline for the turn about to write. Taken lazily by the engine right
/// before the first tool that can change the workspace. `None` when the
/// layer is off, so workspaces without a `.blueprint/` pay one `is_dir`.
pub fn snapshot(root: &Path, enabled: bool) -> Option<Baseline> {
    if !enabled || !root.join(BLUEPRINT_DIR).is_dir() {
        return None;
    }
    let files = agent_context::WorkspaceScanner::build_file_index(root, 0).ok()?;
    Some(Baseline {
        files: files
            .into_iter()
            .map(|f| {
                let stamp = stamp(&root.join(&f));
                (f, stamp)
            })
            .collect(),
        report: load(root, enabled)?.check(),
    })
}

fn stamp(path: &Path) -> (Option<SystemTime>, u64) {
    std::fs::metadata(path).map_or((None, 0), |m| (m.modified().ok(), m.len()))
}

/// Files written since the baseline — created, or rewritten in place (a
/// `bash` `cargo add` edits an existing manifest). What `bash` and other
/// tools that bypass the hunk tracker changed. Deletions are not reported.
pub fn changed_since(root: &Path, before: &Baseline) -> Vec<PathBuf> {
    let Ok(now) = agent_context::WorkspaceScanner::build_file_index(root, 0) else {
        return Vec::new();
    };
    now.into_iter()
        .filter(|f| {
            before
                .files
                .get(f)
                .map_or(true, |old| *old != stamp(&root.join(f)))
        })
        .map(|f| root.join(f))
        .collect()
}

/// Turn-end check over the files the agent wrote this turn. Findings the
/// baseline already had are dropped: a rule broken before the turn began is
/// not this turn's to fix. `None` when the layer is off or nothing is new.
pub fn check_turn(
    root: &Path,
    enabled: bool,
    touched: &[PathBuf],
    before: Option<&Baseline>,
) -> Option<Report> {
    if touched.is_empty() {
        return None;
    }
    let mut report = load(root, enabled)?.check_turn(touched);
    if let Some(before) = before {
        report.issues.retain(|i| !before.report.issues.contains(i));
    }
    (!report.is_clean()).then_some(report)
}

/// The hidden follow-up the engine pushes when a turn ends with blueprint
/// errors — one shot, the model either fixes or explains.
pub fn nudge(report: &Report) -> String {
    format!(
        "架构蓝图检查未通过 — 修正后再结束本轮：\n{}\n\
         违反 `forbidden_deps`/`invariants` 时改代码；新文件需归入某个节点的 `owns`；\
         确属有意的架构变化，则修改对应的 `{BLUEPRINT_DIR}/` 节点文件并说明理由。",
        report.render()
    )
}

/// `/blueprint [check|graph|init]` — `Ok` is a local reply, `Err` a prompt
/// to feed the agent (`init` needs the model to draft nodes).
pub fn command(root: &Path, enabled: bool, args: &str) -> Result<String, String> {
    let sub = args.split_whitespace().next().unwrap_or("");
    if sub == "init" {
        return Err(init_prompt(root));
    }
    if !enabled {
        return Ok("架构蓝图已在设置中关闭".into());
    }
    let Some(bp) = Blueprint::load(root) else {
        return Ok(format!(
            "此工作区没有 `{BLUEPRINT_DIR}/` — 用 `/blueprint init` 让 agent 起草一份"
        ));
    };
    Ok(match sub {
        "graph" => format!("```mermaid\n{}```", bp.graph().mermaid(&bp.nodes)),
        "check" => bp.check().render(),
        "" => format!(
            "{} 个节点（`/blueprint check` 检查规则，`/blueprint graph` 查看依赖图）\n{}",
            bp.nodes.len(),
            bp.index_block()
        ),
        other => format!("未知子命令 `{other}` — 可用：check / graph / init"),
    })
}

fn init_prompt(root: &Path) -> String {
    let existing = if root.join(BLUEPRINT_DIR).is_dir() {
        format!(
            "`{BLUEPRINT_DIR}/` already exists — extend it: keep existing nodes, add nodes \
             for code nothing owns yet (`blueprint` `check` lists those files)."
        )
    } else {
        format!("Create `{BLUEPRINT_DIR}/` from scratch.")
    };
    format!(
        "Bootstrap this workspace's architecture blueprint. {existing}\n\
         \n\
         1. Survey the structure: package manifests (Cargo.toml / package.json / go.mod / \
            pyproject.toml), the top-level layout, and the entry points. Read code only \
            where a module's purpose isn't clear from its manifest and layout.\n\
         2. Choose nodes along ARCHITECTURE boundaries — packages, services, layers, \
            domains — not one per file. Aim for 5–20 nodes; every non-root source file \
            should fall under exactly one node's `owns`.\n\
         3. Write one file per node, `{BLUEPRINT_DIR}/<node>.md`, with `apply_patch`:\n\
         \n\
         ```\n\
         +++\n\
         owns = [\"path/to/module/**\"]\n\
         forbidden_deps = []   # package or node names this module must never depend on\n\
         invariants = []       # rules the code must always keep\n\
         +++\n\
         <one-line responsibility — becomes the index entry>\n\
         \n\
         ## 职责\n\
         ## 不做\n\
         ## 对外契约\n\
         ```\n\
         \n\
         Keep each node under ~300 tokens. Do NOT write dependency lists — the graph is \
         derived from manifests. Only fill `forbidden_deps`/`invariants` with rules the \
         code already respects or the user stated; list doubtful ones in your summary \
         instead of inventing them.\n\
         4. Run `blueprint` `check` and fix what it reports, then summarize the nodes and \
            the rules you'd suggest adding."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_dir_renders_off_marker_and_no_report() {
        let d = tempfile::tempdir().unwrap();
        assert!(prompt_block(d.path(), true).contains("architecture layer off"));
        assert!(check_turn(d.path(), true, &[d.path().join("x.rs")], None).is_none());
        assert!(command(d.path(), true, "")
            .unwrap()
            .contains("/blueprint init"));
        assert!(command(d.path(), true, "init")
            .unwrap_err()
            .contains("from scratch"));
    }

    #[test]
    fn present_dir_renders_index() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".blueprint")).unwrap();
        std::fs::write(
            d.path().join(".blueprint/core.md"),
            "+++\nowns = [\"src/**\"]\n+++\nCore engine.\n",
        )
        .unwrap();
        let block = prompt_block(d.path(), true);
        assert!(block.contains("- core — Core engine. [src/**]"), "{block}");
        assert!(command(d.path(), true, "graph")
            .unwrap()
            .contains("graph LR"));
        assert!(command(d.path(), true, "init")
            .unwrap_err()
            .contains("extend it"));
    }

    /// A session whose switch is off sees the layer as absent, even though the
    /// directory exists — and another session in the same process is unaffected.
    #[test]
    fn switch_is_per_session_not_process_wide() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".blueprint")).unwrap();
        std::fs::write(
            d.path().join(".blueprint/core.md"),
            "+++\nowns = [\"src/**\"]\n+++\nCore.\n",
        )
        .unwrap();
        assert!(prompt_block(d.path(), false).contains("architecture layer off"));
        assert!(prompt_block(d.path(), true).contains("- core"));
        assert!(snapshot(d.path(), false).is_none());
        assert!(command(d.path(), false, "check")
            .unwrap()
            .contains("架构蓝图已在设置中关闭"));
    }

    #[test]
    fn changed_since_sees_new_and_rewritten_files_outside_the_hunk_tracker() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".blueprint")).unwrap();
        std::fs::write(d.path().join("old.rs"), "").unwrap();
        std::fs::write(d.path().join("manifest.toml"), "a").unwrap();
        let before = snapshot(d.path(), true).expect("blueprint present");
        std::fs::write(d.path().join("loose.rs"), "").unwrap();
        std::fs::write(d.path().join("manifest.toml"), "a longer body").unwrap();
        let made = changed_since(d.path(), &before);
        assert_eq!(
            made,
            [d.path().join("loose.rs"), d.path().join("manifest.toml")]
        );
        let empty = tempfile::tempdir().unwrap();
        assert!(snapshot(empty.path(), true).is_none());
    }
}
