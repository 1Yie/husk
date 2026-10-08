//! Kernel side of the `.blueprint/` architecture layer — the prompt block,
//! the turn-end check, and the `/blueprint` command. The data model and
//! every rule live in `agent_context::blueprint`; this module only decides
//! *when* the kernel consults it.
//!
//! Off by construction for workspaces without `.blueprint/`: every entry
//! point loads first and returns the empty/neutral value on `None`, so the
//! feature costs one `is_dir` per turn there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use agent_context::blueprint::{Blueprint, Report, BLUEPRINT_DIR};

/// Process-wide master switch (settings 偏好) — seeded from persisted prefs
/// at boot, flipped live by the settings pane. On by default: the layer is
/// already opt-in per workspace through the directory itself.
static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
}

fn load(root: &Path) -> Option<Blueprint> {
    if !enabled() {
        return None;
    }
    Blueprint::load(root)
}

/// The `{{BLUEPRINT_BLOCK}}` region — the contract plus the node index, or
/// a one-line "off" marker. Rebuilt every turn by the session but swapped
/// into the system prompt only when the text changed, so the cached prompt
/// prefix survives every turn that didn't edit `.blueprint/`.
pub fn prompt_block(root: &Path) -> String {
    let Some(bp) = load(root) else {
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

/// Workspace file list — the baseline [`created_since`] diffs against.
/// Taken lazily by the engine before the first process/orchestration tool.
/// `None` when the layer is off, so workspaces without a `.blueprint/` pay
/// one `is_dir`.
pub fn snapshot(root: &Path) -> Option<BTreeSet<String>> {
    if !enabled() || !root.join(BLUEPRINT_DIR).is_dir() {
        return None;
    }
    agent_context::WorkspaceScanner::build_file_index(root, 0)
        .ok()
        .map(|files| files.into_iter().collect())
}

/// Files that appeared since `before` — what `bash` and other tools that
/// bypass the hunk tracker created. Creations only: they are what the
/// "every file belongs to a node" rule needs, and a set diff stays cheap.
pub fn created_since(root: &Path, before: &BTreeSet<String>) -> Vec<PathBuf> {
    let Ok(now) = agent_context::WorkspaceScanner::build_file_index(root, 0) else {
        return Vec::new();
    };
    now.into_iter()
        .filter(|f| !before.contains(f))
        .map(|f| root.join(f))
        .collect()
}

/// Turn-end check over the files the agent wrote this turn. `None` when the
/// layer is off or there is nothing to report.
pub fn check_turn(root: &Path, touched: &[PathBuf]) -> Option<Report> {
    if touched.is_empty() {
        return None;
    }
    let report = load(root)?.check_turn(touched);
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
pub fn command(root: &Path, args: &str) -> Result<String, String> {
    let sub = args.split_whitespace().next().unwrap_or("");
    if sub == "init" {
        return Err(init_prompt(root));
    }
    if !enabled() {
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
        assert!(prompt_block(d.path()).contains("architecture layer off"));
        assert!(check_turn(d.path(), &[d.path().join("x.rs")]).is_none());
        assert!(command(d.path(), "").unwrap().contains("/blueprint init"));
        assert!(command(d.path(), "init")
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
        let block = prompt_block(d.path());
        assert!(block.contains("- core — Core engine. [src/**]"), "{block}");
        assert!(command(d.path(), "graph").unwrap().contains("graph LR"));
        assert!(command(d.path(), "init").unwrap_err().contains("extend it"));
    }
    #[test]
    fn created_since_sees_files_outside_the_hunk_tracker() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".blueprint")).unwrap();
        std::fs::write(d.path().join("old.rs"), "").unwrap();
        let before = snapshot(d.path()).expect("blueprint present");
        std::fs::write(d.path().join("loose.rs"), "").unwrap();
        let made = created_since(d.path(), &before);
        assert_eq!(made, [d.path().join("loose.rs")]);
        let empty = tempfile::tempdir().unwrap();
        assert!(snapshot(empty.path()).is_none());
    }
}
