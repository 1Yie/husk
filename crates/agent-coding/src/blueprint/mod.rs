//! `blueprint` — the architecture layer the agent reads before the code.
//!
//! A workspace opts in by carrying a `.blueprint/` directory of Markdown
//! nodes, each with a TOML frontmatter block (`+++ … +++`):
//!
//! ```text
//! +++
//! node = "kernel"                      # defaults to the file stem
//! owns = ["crates/kernel/**"]          # globs, workspace-relative
//! forbidden_deps = ["iced", "tauri*"]  # package names or other node names
//! invariants = ["tool output is truncated"]
//! +++
//! ## Responsibility
//! ...
//! ```
//!
//! Only *normative* facts live in the files — what a node is for, what it must
//! not do, what it must never depend on. Everything descriptive (the dependency
//! graph) is re-derived on every query — package manifests for cross-package
//! edges, source imports for edges inside one package — so it cannot drift:
//! [`Blueprint::check`] narrows "drift detection" to "rule violations".
//!
//! Node files are written through [`edit`] (`blueprint_edit`), never through
//! the generic patch tools — [`guard`] refuses those paths. An architecture
//! change is a structured edit whose rendered file the loader must accept,
//! not a text diff against a Markdown file.
//!
//! Markdown files without a frontmatter block (a `README.md` in the directory)
//! are prose, not nodes, and are skipped.
//!
//! Precision limits of the derived graph: source imports are read line by line
//! rather than parsed, so `use super::`, `mod` declarations and macro-generated
//! references are invisible; cross-package edges come only from manifests, so a
//! `forbidden_deps` entry naming a node applies to the node's own manifests and
//! imports. A manifest that cannot be read or parsed is reported as a warning,
//! and its dependencies go unchecked until it is fixed.

pub mod edit;
pub mod guard;
pub mod imports;
pub mod manifest;
mod runtime;

pub use guard::guard_patch;
pub use runtime::{changed_since, check_turn, command, nudge, prompt_block, snapshot, Baseline};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use agent_context::WorkspaceScanner;

/// Workspace-relative directory holding the nodes.
pub const BLUEPRINT_DIR: &str = ".blueprint";

/// Rendered index cap — the block rides in every request's system prompt.
pub const INDEX_MAX_CHARS: usize = 6_000;

/// Per-node summary cap inside the index.
const SUMMARY_MAX_CHARS: usize = 120;

/// One architecture node.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    /// Workspace-relative path of the node file (`.blueprint/kernel.md`).
    pub file: String,
    pub owns: Vec<String>,
    pub forbidden_deps: Vec<String>,
    pub invariants: Vec<String>,
    /// First prose line of the body — the index entry.
    pub summary: String,
    /// Markdown body after the frontmatter.
    pub body: String,
}

/// A node file's frontmatter — one type for both directions, so what
/// [`edit`] renders is by construction what the loader reads back.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) owns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) forbidden_deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) invariants: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Issue {
    pub severity: Severity,
    /// The node the issue is about, when there is one.
    pub node: Option<String>,
    pub message: String,
}

/// A check result — errors first, then warnings, each in discovery order.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    fn push(&mut self, severity: Severity, node: Option<&str>, message: String) {
        self.issues.push(Issue {
            severity,
            node: node.map(str::to_string),
            message,
        });
    }

    fn finish(mut self) -> Self {
        self.issues.sort_by_key(|i| i.severity);
        self
    }

    /// Plain-text rendering for tool output and nudges.
    pub fn render(&self) -> String {
        if self.issues.is_empty() {
            return "blueprint check: clean".into();
        }
        let mut out = format!(
            "blueprint check: {} error(s), {} warning(s)\n",
            self.errors(),
            self.issues.len() - self.errors()
        );
        for i in &self.issues {
            let tag = match i.severity {
                Severity::Error => "error",
                Severity::Warning => "warn ",
            };
            match &i.node {
                Some(n) => out.push_str(&format!("- {tag} [{n}] {}\n", i.message)),
                None => out.push_str(&format!("- {tag} {}\n", i.message)),
            }
        }
        out
    }
}

/// The derived dependency graph — recomputed from manifests on demand.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    /// node → package names it provides.
    pub provides: BTreeMap<String, BTreeSet<String>>,
    /// node → runtime dependency names its manifests require.
    pub requires: BTreeMap<String, BTreeSet<String>>,
    /// node → other nodes it depends on (requires ∩ their provides).
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// Owned manifests that could not be read — their deps are unknown.
    pub unreadable: Vec<String>,
    /// Owned manifests that failed to parse — their deps are unknown.
    pub unparsed: Vec<String>,
}

impl Graph {
    /// Nodes one hop away (either direction).
    pub fn neighbors(&self, node: &str) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.edges.get(node).cloned().unwrap_or_default();
        for (from, tos) in &self.edges {
            if tos.contains(node) {
                out.insert(from.clone());
            }
        }
        out.remove(node);
        out
    }

    pub fn mermaid(&self, nodes: &[Node]) -> String {
        let mut out = String::from("graph LR\n");
        for n in nodes {
            out.push_str(&format!("  {}[\"{}\"]\n", mermaid_id(&n.name), n.name));
        }
        for (from, tos) in &self.edges {
            for to in tos {
                out.push_str(&format!("  {} --> {}\n", mermaid_id(from), mermaid_id(to)));
            }
        }
        out
    }
}

fn mermaid_id(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A loaded `.blueprint/` directory.
#[derive(Debug, Clone)]
pub struct Blueprint {
    root: PathBuf,
    pub nodes: Vec<Node>,
    /// One matcher per node, same order as `nodes`.
    owners: Vec<GlobSet>,
    /// Node files that failed to parse — surfaced by [`Blueprint::check`].
    load_errors: Vec<(String, String)>,
}

impl Blueprint {
    /// Load `<root>/.blueprint/`. `None` when the directory doesn't exist —
    /// the workspace hasn't opted in and the feature costs nothing.
    pub fn load(root: &Path) -> Option<Self> {
        let dir = root.join(BLUEPRINT_DIR);
        if !dir.is_dir() {
            return None;
        }
        let mut files = Vec::new();
        collect_md(&dir, &mut files);
        files.sort();

        let mut nodes: Vec<Node> = Vec::new();
        let mut owners = Vec::new();
        let mut load_errors = Vec::new();
        for path in files {
            let rel = rel_str(root, &path);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => {
                    load_errors.push((rel, format!("cannot read node file: {e}")));
                    continue;
                }
            };
            let (fm, body) = match parse_node_file(&path, &text) {
                NodeParse::Node(fm, body) => (fm, body),
                NodeParse::Prose => continue,
                NodeParse::Malformed(msg) => {
                    load_errors.push((rel, msg));
                    continue;
                }
            };
            let name = fm.node.clone().unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            if nodes.iter().any(|n| n.name == name) {
                load_errors.push((rel, format!("duplicate node name `{name}`")));
                continue;
            }
            let mut set = GlobSetBuilder::new();
            for pat in &fm.owns {
                match glob(pat) {
                    Ok(g) => {
                        set.add(g);
                    }
                    Err(e) => {
                        load_errors.push((rel.clone(), format!("bad owns glob `{pat}`: {e}")))
                    }
                }
            }
            owners.push(set.build().unwrap_or_else(|_| GlobSet::empty()));
            nodes.push(Node {
                name,
                file: rel,
                owns: fm.owns,
                forbidden_deps: fm.forbidden_deps,
                invariants: fm.invariants,
                summary: summary_of(&body),
                body,
            });
        }
        Some(Self {
            root: root.to_path_buf(),
            nodes,
            owners,
            load_errors,
        })
    }

    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }

    /// Nodes owning `rel` (a `/`-separated workspace-relative path).
    pub fn owners_of(&self, rel: &str) -> Vec<&Node> {
        self.owners
            .iter()
            .zip(&self.nodes)
            .filter(|(set, _)| set.is_match(rel))
            .map(|(_, n)| n)
            .collect()
    }

    /// The system-prompt index: one line per node, capped at
    /// [`INDEX_MAX_CHARS`]. Edges are left out on purpose — they're live
    /// (`locate`/`graph`), the index only changes when `.blueprint/` does.
    pub fn index_block(&self) -> String {
        let mut out = String::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let line = format!("- {} — {} [{}]\n", n.name, n.summary, n.owns.join(", "));
            if out.len() + line.len() > INDEX_MAX_CHARS {
                out.push_str(&format!(
                    "- … {} more node(s) — `blueprint {{\"action\":\"locate\"}}` lists them\n",
                    self.nodes.len() - i
                ));
                break;
            }
            out.push_str(&line);
        }
        if self.nodes.is_empty() {
            out.push_str(
                "(no nodes yet — `.blueprint/` exists but carries no `+++` frontmatter files)\n",
            );
        }
        out
    }

    /// Workspace file list — gitignore-respected, hidden dirs skipped.
    fn files(&self) -> Vec<String> {
        WorkspaceScanner::build_file_index(&self.root, 0).unwrap_or_default()
    }

    /// Whether the file index hit its cap — then every file-based check is
    /// partial, and the report says so.
    fn index_truncated(files: &[String]) -> bool {
        files.len() >= agent_context::workspace::MAX_INDEX_ENTRIES
    }

    /// Derive the dependency graph: package manifests (attributed to the
    /// node(s) owning them) plus source-level imports between nodes.
    pub fn graph(&self) -> Graph {
        self.graph_from(&self.files())
    }

    fn graph_from(&self, files: &[String]) -> Graph {
        let mut g = Graph::default();
        let mut go_modules: Vec<(String, String)> = Vec::new();
        for rel in files {
            let fname = rel.rsplit('/').next().unwrap_or(rel);
            if !manifest::MANIFEST_NAMES.contains(&fname) {
                continue;
            }
            let owners = self.owners_of(rel);
            if owners.is_empty() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(self.root.join(rel)) else {
                g.unreadable.push(rel.clone());
                continue;
            };
            let Some(m) = manifest::parse(Path::new(fname), &text) else {
                g.unparsed.push(rel.clone());
                continue;
            };
            if fname == "go.mod" {
                let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                for module in &m.provides {
                    go_modules.push((module.clone(), dir.to_string()));
                }
            }
            for n in owners {
                g.provides
                    .entry(n.name.clone())
                    .or_default()
                    .extend(m.provides.iter().cloned());
                g.requires
                    .entry(n.name.clone())
                    .or_default()
                    .extend(m.requires.iter().cloned());
            }
        }
        for (from, reqs) in &g.requires {
            for (to, provs) in &g.provides {
                if from != to && reqs.iter().any(|r| provs.contains(r)) {
                    g.edges.entry(from.clone()).or_default().insert(to.clone());
                }
            }
        }
        self.add_import_edges(files, &go_modules, &mut g);
        g
    }

    fn add_import_edges(&self, files: &[String], go_modules: &[(String, String)], g: &mut Graph) {
        // Package name (`-` folded to `_`, as in a Rust path) → nodes whose
        // manifests provide it, for crate references inside one package.
        let mut by_package: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (node, pkgs) in &g.provides {
            for p in pkgs {
                by_package
                    .entry(p.replace('-', "_"))
                    .or_default()
                    .insert(node.clone());
            }
        }
        for rel in files {
            let from = self.owners_of(rel);
            if from.is_empty() {
                continue;
            }
            let path = self.root.join(rel);
            if std::fs::metadata(&path).map_or(true, |m| m.len() > imports::MAX_FILE_BYTES) {
                continue;
            }
            let targets = match rel.rsplit('.').next() {
                Some("rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" | "go") => {
                    match std::fs::read_to_string(&path) {
                        Ok(text) => {
                            if rel.ends_with(".rs") {
                                for name in imports::rust_crate_refs(&text) {
                                    for to in by_package.get(&name).into_iter().flatten() {
                                        for f in &from {
                                            if *to != f.name {
                                                g.edges
                                                    .entry(f.name.clone())
                                                    .or_default()
                                                    .insert(to.clone());
                                            }
                                        }
                                    }
                                }
                            }
                            imports::targets(rel, &text, go_modules)
                        }
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };
            for t in targets {
                let to: BTreeSet<&str> = imports::candidates(&t)
                    .iter()
                    .flat_map(|c| self.owners_of(c))
                    .map(|n| n.name.as_str())
                    .collect();
                for f in &from {
                    for t in &to {
                        if *t != f.name {
                            g.edges
                                .entry(f.name.clone())
                                .or_default()
                                .insert(t.to_string());
                        }
                    }
                }
            }
        }
    }

    /// Nodes matching a free-text query and/or owning any of `paths`, plus
    /// one-hop graph neighbours of the direct hits. Direct hits come first,
    /// best-scored first; at most `limit` direct hits.
    pub fn locate(&self, query: &str, paths: &[String], limit: usize) -> Located<'_> {
        let mut scored: Vec<(usize, &Node)> = Vec::new();
        let terms: Vec<String> = query
            .split(|c: char| c.is_whitespace() || c == ',' || c == '，')
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        for n in &self.nodes {
            let mut score = 0;
            let name = n.name.to_lowercase();
            let body = n.body.to_lowercase();
            for t in &terms {
                if name == *t {
                    score += 10;
                } else if name.contains(t.as_str()) {
                    score += 5;
                }
                score += body.matches(t.as_str()).count().min(5);
            }
            for p in paths {
                if self
                    .owners_of(&normalize(p))
                    .iter()
                    .any(|o| o.name == n.name)
                {
                    score += 20;
                }
            }
            if score > 0 {
                scored.push((score, n));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.truncate(limit.max(1));
        let hits: Vec<&Node> = scored.into_iter().map(|(_, n)| n).collect();

        let graph = self.graph();
        let mut neighbors = BTreeSet::new();
        for h in &hits {
            neighbors.extend(graph.neighbors(&h.name));
        }
        for h in &hits {
            neighbors.remove(&h.name);
        }
        let neighbors = neighbors.iter().filter_map(|n| self.node(n)).collect();
        Located {
            hits,
            neighbors,
            graph,
        }
    }

    /// Full check over the whole workspace.
    pub fn check(&self) -> Report {
        let files = self.files();
        let graph = self.graph_from(&files);
        let mut r = self.check_rules(&graph);
        if Self::index_truncated(&files) {
            r.push(
                Severity::Warning,
                None,
                format!(
                    "workspace index hit its {} entry cap — files past it are not checked",
                    agent_context::workspace::MAX_INDEX_ENTRIES
                ),
            );
        }
        let unowned: Vec<&String> = files
            .iter()
            .filter(|f| !exempt(f) && self.owners_of(f).is_empty())
            .collect();
        if !unowned.is_empty() {
            let sample: Vec<&str> = unowned.iter().take(8).map(|s| s.as_str()).collect();
            r.push(
                Severity::Warning,
                None,
                format!(
                    "{} file(s) belong to no node: {}{}",
                    unowned.len(),
                    sample.join(", "),
                    if unowned.len() > sample.len() {
                        ", …"
                    } else {
                        ""
                    }
                ),
            );
        }
        self.check_ownership(&files, &mut r);
        r.finish()
    }

    /// `owns` globs that match nothing (a renamed or deleted directory left
    /// the node pointing at air) and files claimed by more than one node.
    /// Hidden-directory patterns are skipped: the file index never lists
    /// them, so "matches nothing" would be a false alarm.
    fn check_ownership(&self, files: &[String], r: &mut Report) {
        for n in &self.nodes {
            for pat in &n.owns {
                if normalize(pat).starts_with('.') {
                    continue;
                }
                let Ok(g) = glob(pat) else { continue };
                let m = g.compile_matcher();
                if !files.iter().any(|f| m.is_match(f)) {
                    r.push(
                        Severity::Warning,
                        Some(&n.name),
                        format!("`owns` pattern `{pat}` matches no files"),
                    );
                }
            }
        }
        let mut shared: BTreeMap<(String, String), (usize, &str)> = BTreeMap::new();
        for f in files {
            let owners = self.owners_of(f);
            for (i, a) in owners.iter().enumerate() {
                for b in &owners[i + 1..] {
                    let e = shared
                        .entry((a.name.clone(), b.name.clone()))
                        .or_insert((0, f.as_str()));
                    e.0 += 1;
                }
            }
        }
        for ((a, b), (count, sample)) in shared {
            r.push(
                Severity::Warning,
                Some(&a),
                format!("{count} file(s) also owned by `{b}`, e.g. {sample} — each file should have one owner"),
            );
        }
    }

    /// Turn-scoped check over the files the agent wrote this turn: rule
    /// violations of the touched nodes, new files outside every node
    /// (error), and nodes whose package manifest changed while their node
    /// file did not (warning — dependencies moved without a record). Plain
    /// code edits inside a node are the normal case and stay silent.
    pub fn check_turn(&self, touched: &[PathBuf]) -> Report {
        let rels: Vec<String> = touched
            .iter()
            .filter_map(|p| p.strip_prefix(&self.root).ok().map(rel_path))
            .collect();
        if rels.is_empty() {
            return Report::default();
        }
        let blueprint_touched: BTreeSet<&str> = rels
            .iter()
            .filter(|r| r.starts_with(&format!("{BLUEPRINT_DIR}/")))
            .map(|s| s.as_str())
            .collect();
        let mut r = Report::default();
        // `.blueprint/` edits that broke a node file — a turn that only
        // touched blueprint files still has to leave them loadable.
        for (file, err) in &self.load_errors {
            if blueprint_touched.contains(file.as_str()) {
                r.push(Severity::Error, None, format!("{file}: {err}"));
            }
        }
        let mut changed_nodes: BTreeSet<&str> = BTreeSet::new();
        let mut manifest_nodes: BTreeSet<&str> = BTreeSet::new();
        for rel in rels
            .iter()
            .filter(|r| !blueprint_touched.contains(r.as_str()))
        {
            let owners = self.owners_of(rel);
            if owners.is_empty() {
                if !exempt(rel) && self.root.join(rel).exists() {
                    r.push(
                        Severity::Error,
                        None,
                        format!(
                            "`{rel}` belongs to no node — add it to a node's `owns` or move it under one"
                        ),
                    );
                }
                continue;
            }
            if is_test_path(rel) {
                continue;
            }
            let is_manifest =
                manifest::MANIFEST_NAMES.contains(&rel.rsplit('/').next().unwrap_or(rel));
            for o in owners {
                changed_nodes.insert(o.name.as_str());
                if is_manifest {
                    manifest_nodes.insert(o.name.as_str());
                }
            }
        }
        if !changed_nodes.is_empty() {
            let graph = self.graph();
            let rules = self.check_rules(&graph);
            r.issues.extend(
                rules
                    .issues
                    .into_iter()
                    .filter(|i| i.node.as_deref().is_some_and(|n| changed_nodes.contains(n))),
            );
            for n in &manifest_nodes {
                let Some(node) = self.node(n) else { continue };
                if !blueprint_touched.contains(node.file.as_str()) {
                    r.push(
                        Severity::Warning,
                        Some(n),
                        format!(
                            "package manifest changed but `{}` did not — record it there if the module's dependencies or role moved",
                            node.file
                        ),
                    );
                }
            }
        }
        r.finish()
    }

    /// Node-level rules against the derived graph: load errors, empty
    /// `owns`, and `forbidden_deps` hits (package names and node edges).
    fn check_rules(&self, graph: &Graph) -> Report {
        let mut r = Report::default();
        for (file, err) in &self.load_errors {
            r.push(Severity::Error, None, format!("{file}: {err}"));
        }
        for file in &graph.unreadable {
            r.push(
                Severity::Warning,
                None,
                format!("`{file}` could not be read — its dependencies are not checked"),
            );
        }
        for file in &graph.unparsed {
            r.push(
                Severity::Warning,
                None,
                format!("`{file}` does not parse — its dependencies are not checked"),
            );
        }
        for n in &self.nodes {
            if n.owns.is_empty() {
                r.push(Severity::Warning, Some(&n.name), "owns no paths".into());
            }
            if n.forbidden_deps.is_empty() {
                continue;
            }
            let mut set = GlobSetBuilder::new();
            for pat in &n.forbidden_deps {
                if let Ok(g) = Glob::new(pat) {
                    set.add(g);
                }
            }
            let Ok(forbidden) = set.build() else { continue };
            if forbidden.is_match(&n.name) {
                r.push(
                    Severity::Error,
                    Some(&n.name),
                    "its own `forbidden_deps` matches the node itself".into(),
                );
            }
            let reqs = graph.requires.get(&n.name);
            for dep in reqs.into_iter().flatten() {
                if forbidden.is_match(dep) {
                    r.push(
                        Severity::Error,
                        Some(&n.name),
                        format!("depends on `{dep}`, which its `forbidden_deps` rules out"),
                    );
                }
            }
            for to in graph.edges.get(&n.name).into_iter().flatten() {
                if forbidden.is_match(to) {
                    r.push(
                        Severity::Error,
                        Some(&n.name),
                        format!("depends on node `{to}`, which its `forbidden_deps` rules out"),
                    );
                }
            }
        }
        r
    }
}

/// [`Blueprint::locate`] result.
pub struct Located<'a> {
    pub hits: Vec<&'a Node>,
    pub neighbors: Vec<&'a Node>,
    pub graph: Graph,
}

impl Located<'_> {
    pub fn render(&self) -> String {
        if self.hits.is_empty() {
            return "no blueprint node matched — try other terms or `paths`".into();
        }
        let mut out = String::new();
        for n in &self.hits {
            out.push_str(&render_node(n, &self.graph));
            out.push('\n');
        }
        if !self.neighbors.is_empty() {
            out.push_str("── one-hop neighbours ──\n");
            for n in &self.neighbors {
                out.push_str(&format!(
                    "- {} — {} [{}]\n",
                    n.name,
                    n.summary,
                    n.owns.join(", ")
                ));
            }
        }
        out
    }
}

/// Full node rendering — frontmatter facts, live edges, body.
pub fn render_node(n: &Node, graph: &Graph) -> String {
    let mut out = format!("## node `{}` ({})\n", n.name, n.file);
    out.push_str(&format!("owns: {}\n", n.owns.join(", ")));
    if let Some(deps) = graph.edges.get(&n.name).filter(|d| !d.is_empty()) {
        out.push_str(&format!(
            "depends on (derived): {}\n",
            deps.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let used_by: Vec<&String> = graph
        .edges
        .iter()
        .filter(|(_, tos)| tos.contains(&n.name))
        .map(|(from, _)| from)
        .collect();
    if !used_by.is_empty() {
        out.push_str(&format!(
            "used by (derived): {}\n",
            used_by
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !n.forbidden_deps.is_empty() {
        out.push_str(&format!(
            "forbidden deps: {}\n",
            n.forbidden_deps.join(", ")
        ));
    }
    for inv in &n.invariants {
        out.push_str(&format!("invariant: {inv}\n"));
    }
    if !n.body.is_empty() {
        out.push('\n');
        out.push_str(&n.body);
        out.push('\n');
    }
    out
}

pub(crate) fn glob(pat: &str) -> Result<Glob, globset::Error> {
    // `*` stays within one path segment; `**` crosses them.
    GlobBuilder::new(pat.trim_start_matches("./"))
        .literal_separator(true)
        .build()
}

/// `---`-free TOML frontmatter: the file opens with a `+++` line and the
/// block ends at the next `+++` line.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("+++\n")
        .or_else(|| text.strip_prefix("+++\r\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "+++" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn summary_of(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let line = line.trim_start_matches(['-', '*', ' ']);
    if line.chars().count() > SUMMARY_MAX_CHARS {
        let cut: String = line.chars().take(SUMMARY_MAX_CHARS).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

/// What reading one `.blueprint/*.md` file yielded.
pub(crate) enum NodeParse {
    /// A node: its frontmatter and the trimmed body after it.
    Node(Frontmatter, String),
    /// Markdown that never claimed to be a node — a `README.md`, or a file
    /// with no `+++` fence at all.
    Prose,
    /// It claimed to be a node and failed; the message says why.
    Malformed(String),
}

/// Read one node file's text. The loader and [`edit`] share this, so an edit
/// cannot produce something the loader would then refuse.
pub(crate) fn parse_node_file(path: &Path, text: &str) -> NodeParse {
    let Some((front, body)) = split_frontmatter(text) else {
        return if looks_like_node(path, text) {
            NodeParse::Malformed(
                "frontmatter fence malformed — a node file opens with `+++` on its own line and closes with another `+++`".into(),
            )
        } else {
            NodeParse::Prose
        };
    };
    match toml::from_str::<Frontmatter>(front) {
        Ok(fm) => NodeParse::Node(fm, body.trim().to_string()),
        Err(e) => NodeParse::Malformed(e.message().to_string()),
    }
}

/// The file text for a node: the `+++` frontmatter block, a blank line, then
/// the prose. Empty fields are omitted so a node file states only what it
/// rules.
pub(crate) fn render_node_file(fm: &Frontmatter, body: &str) -> String {
    let front = toml::to_string(fm).unwrap_or_default();
    let body = body.trim();
    let mut out = String::with_capacity(front.len() + body.len() + 16);
    out.push_str("+++\n");
    out.push_str(&front);
    out.push_str("+++\n");
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    out
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_md(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// A markdown file that tried to be a node — it opens with a `+++`/`---`
/// fence the splitter rejected. A bare `README.md` is prose by design.
fn looks_like_node(path: &Path, text: &str) -> bool {
    let is_readme = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("README.md"));
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    !is_readme && (text.starts_with("++") || text.starts_with("---"))
}

/// Files no node is expected to own: root-level files (manifests, README,
/// lockfiles), the blueprint itself, and anything under a hidden directory
/// — the same set the file index never lists, so `check` and `check_turn`
/// agree.
fn exempt(rel: &str) -> bool {
    !rel.contains('/')
        || rel.starts_with(&format!("{BLUEPRINT_DIR}/"))
        || rel.split('/').any(|seg| seg.starts_with('.'))
}

/// Test code may change without the architecture changing.
fn is_test_path(rel: &str) -> bool {
    rel.split('/').any(|seg| {
        matches!(
            seg,
            "tests" | "test" | "__tests__" | "spec" | "testdata" | "fixtures"
        )
    }) || {
        let file = rel.rsplit('/').next().unwrap_or(rel);
        file.contains(".test.")
            || file.contains(".spec.")
            || file.ends_with("_test.go")
            || file.starts_with("test_")
            || file.ends_with("_test.py")
    }
}

fn normalize(p: &str) -> String {
    p.trim_start_matches("./").replace('\\', "/")
}

pub(crate) fn rel_path(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn rel_str(root: &Path, p: &Path) -> String {
    rel_path(p.strip_prefix(root).unwrap_or(p))
}

/// Every workspace-relative path the architecture layer watches: the ones the
/// workspace walk indexes, plus the node files it cannot see. `.blueprint/` is
/// hidden, so `build_file_index` skips it — a node rewritten by `bash` would be
/// invisible to the turn-end diff without this union.
///
/// `None` when the walk fails, which is also when a baseline is impossible:
/// reporting every file as changed would be worse than no baseline at all.
pub(crate) fn watched_files(root: &Path) -> Option<Vec<String>> {
    let mut files = agent_context::WorkspaceScanner::build_file_index(root, 0).ok()?;
    let dir = root.join(BLUEPRINT_DIR);
    if dir.is_dir() {
        let mut nodes = Vec::new();
        collect_md(&dir, &mut nodes);
        files.extend(nodes.iter().map(|p| rel_str(root, p)));
    }
    files.sort();
    files.dedup();
    Some(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// Two-crate Cargo workspace: `kernel` depends on `ui`, which its
    /// blueprint forbids.
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        write(
            r,
            "crates/kernel/Cargo.toml",
            "[package]\nname = \"kernel\"\n[dependencies]\nui = { path = \"../ui\" }\nreqwest = \"0.12\"\n",
        );
        write(r, "crates/kernel/src/lib.rs", "");
        write(r, "crates/ui/Cargo.toml", "[package]\nname = \"ui\"\n");
        write(r, "crates/ui/src/lib.rs", "");
        write(
            r,
            ".blueprint/kernel.md",
            "+++\nowns = [\"crates/kernel/**\"]\nforbidden_deps = [\"ui\", \"req*\"]\ninvariants = [\"no UI\"]\n+++\n## 职责\nReAct loop and tool registry.\n",
        );
        write(
            r,
            ".blueprint/ui.md",
            "+++\nnode = \"ui\"\nowns = [\"crates/ui/**\"]\n+++\nRenders the stream.\n",
        );
        write(r, ".blueprint/README.md", "# prose, not a node\n");
        d
    }

    #[test]
    fn absent_dir_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert!(Blueprint::load(d.path()).is_none());
    }

    #[test]
    fn loads_nodes_and_skips_prose() {
        let d = fixture();
        let bp = Blueprint::load(d.path()).unwrap();
        let names: Vec<&str> = bp.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["kernel", "ui"]);
        assert_eq!(
            bp.node("kernel").unwrap().summary,
            "ReAct loop and tool registry."
        );
        assert_eq!(bp.owners_of("crates/kernel/src/lib.rs")[0].name, "kernel");
        assert!(bp.owners_of("crates/other/x.rs").is_empty());
        let idx = bp.index_block();
        assert!(idx.contains("- kernel — ReAct loop"), "{idx}");
    }

    #[test]
    fn graph_derives_edges_from_manifests() {
        let d = fixture();
        let g = Blueprint::load(d.path()).unwrap().graph();
        assert!(g.edges["kernel"].contains("ui"));
        assert_eq!(
            g.neighbors("ui").into_iter().collect::<Vec<_>>(),
            ["kernel"]
        );
        assert!(g.mermaid(&[]).contains("kernel --> ui"));
    }

    #[test]
    fn check_flags_forbidden_deps_and_unowned() {
        let d = fixture();
        write(d.path(), "scripts/run.sh", "");
        let r = Blueprint::load(d.path()).unwrap().check();
        let text = r.render();
        assert!(text.contains("depends on `reqwest`"), "{text}");
        assert!(text.contains("depends on node `ui`"), "{text}");
        assert!(text.contains("scripts/run.sh"), "{text}");
        assert!(
            !text.contains("Cargo.toml belong"),
            "root files are exempt: {text}"
        );
        assert!(r.errors() >= 2);
        assert_eq!(r.issues[0].severity, Severity::Error);
    }

    #[test]
    fn unparsed_manifest_is_a_warning_not_a_silent_skip() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(
            r,
            ".blueprint/ui.md",
            "+++\nowns = [\"ui/**\"]\nforbidden_deps = [\"reqwest\"]\n+++\nUi.\n",
        );
        write(r, "ui/Cargo.toml", "[package\nname = \"ui\"\n");
        let text = Blueprint::load(r).unwrap().check().render();
        assert!(text.contains("`ui/Cargo.toml` does not parse"), "{text}");
    }

    #[test]
    fn unreadable_node_file_is_a_load_error() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(
            r,
            ".blueprint/ok.md",
            "+++\nowns = [\"src/**\"]\n+++\nOk.\n",
        );
        // Invalid UTF-8 makes `read_to_string` fail; it used to vanish silently.
        std::fs::write(r.join(".blueprint/bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        let text = Blueprint::load(r).unwrap().check().render();
        assert!(text.contains(".blueprint/bin.md"), "{text}");
        assert!(text.contains("cannot read node file"), "{text}");
    }

    #[test]
    fn import_edges_inside_one_package_hit_forbidden_deps() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "Cargo.toml", "[package]\nname = \"app\"\n");
        write(r, "src/tools/bash.rs", "use crate::session::Session;\n");
        write(r, "src/session.rs", "use crate::tools::bash;\n");
        write(
            r,
            ".blueprint/tools.md",
            "+++\nowns = [\"src/tools/**\"]\nforbidden_deps = [\"session\"]\n+++\nTools.\n",
        );
        write(
            r,
            ".blueprint/session.md",
            "+++\nowns = [\"src/session.rs\"]\n+++\nSession.\n",
        );
        let bp = Blueprint::load(r).unwrap();
        let g = bp.graph();
        assert!(g.edges["tools"].contains("session"));
        assert!(g.edges["session"].contains("tools"));
        let text = bp.check().render();
        assert!(text.contains("[tools] depends on node `session`"), "{text}");
    }

    #[test]
    fn crate_references_credit_the_node_that_names_them() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(
            r,
            "crates/dep/Cargo.toml",
            "[package]\nname = \"dep-lib\"\n",
        );
        write(r, "crates/dep/src/lib.rs", "");
        write(
            r,
            "crates/app/Cargo.toml",
            "[package]\nname = \"app\"\n[dependencies]\ndep-lib = { path = \"../dep\" }\n",
        );
        write(r, "crates/app/src/lib.rs", "");
        write(r, "crates/app/src/tools/t.rs", "use dep_lib::go;\n");
        write(r, "crates/app/src/skills/s.rs", "fn f() {}\n");
        write(
            r,
            ".blueprint/dep.md",
            "+++\nowns = [\"crates/dep/**\"]\n+++\nDep.\n",
        );
        write(
            r,
            ".blueprint/core.md",
            "+++\nowns = [\"crates/app/Cargo.toml\", \"crates/app/src/*.rs\"]\n+++\nCore.\n",
        );
        write(
            r,
            ".blueprint/tools.md",
            "+++\nowns = [\"crates/app/src/tools/**\"]\nforbidden_deps = [\"dep\"]\n+++\nTools.\n",
        );
        write(
            r,
            ".blueprint/skills.md",
            "+++\nowns = [\"crates/app/src/skills/**\"]\n+++\nSkills.\n",
        );
        let bp = Blueprint::load(r).unwrap();
        let g = bp.graph();
        assert!(g.edges["tools"].contains("dep"), "{:?}", g.edges);
        assert!(g.edges["core"].contains("dep"), "manifest edge stays");
        assert!(
            !g.edges.get("skills").is_some_and(|e| e.contains("dep")),
            "a node that never names the crate gets no edge: {:?}",
            g.edges
        );
        let text = bp.check().render();
        assert!(text.contains("[tools] depends on node `dep`"), "{text}");
    }

    #[test]
    fn check_turn_scopes_to_touched_nodes() {
        let d = fixture();
        let root = d.path();
        let bp = Blueprint::load(root).unwrap();

        // A plain code edit in `ui`: kernel's forbidden deps stay out of
        // the report, and an ordinary edit is not an architecture event.
        assert!(bp
            .check_turn(&[root.join("crates/ui/src/lib.rs")])
            .is_clean());

        // A manifest edit without the node file → one warning…
        let r = bp.check_turn(&[root.join("crates/ui/Cargo.toml")]);
        assert_eq!(r.errors(), 0, "{}", r.render());
        assert_eq!(r.issues.len(), 1);
        assert_eq!(r.issues[0].node.as_deref(), Some("ui"));

        // …which updating the node file alongside silences.
        let r = bp.check_turn(&[
            root.join("crates/ui/Cargo.toml"),
            root.join(".blueprint/ui.md"),
        ]);
        assert!(r.is_clean(), "{}", r.render());

        // Tests don't count as an architecture change.
        write(root, "crates/ui/tests/t.rs", "");
        assert!(bp
            .check_turn(&[root.join("crates/ui/tests/t.rs")])
            .is_clean());

        // A new file outside every node is an error.
        write(root, "loose/thing.rs", "");
        let r = bp.check_turn(&[root.join("loose/thing.rs")]);
        assert_eq!(r.errors(), 1, "{}", r.render());

        // Touching kernel surfaces its forbidden-dep errors.
        let r = bp.check_turn(&[root.join("crates/kernel/src/lib.rs")]);
        assert!(r.errors() >= 2, "{}", r.render());
    }

    #[test]
    fn locate_ranks_paths_and_terms_with_neighbors() {
        let d = fixture();
        let bp = Blueprint::load(d.path()).unwrap();
        let l = bp.locate("", &["crates/ui/src/lib.rs".into()], 5);
        assert_eq!(l.hits[0].name, "ui");
        assert_eq!(l.neighbors[0].name, "kernel");
        let l = bp.locate("react tool", &[], 5);
        assert_eq!(l.hits[0].name, "kernel");
        assert!(l.render().contains("forbidden deps: ui, req*"));
    }

    #[test]
    fn bad_frontmatter_and_duplicates_are_errors() {
        let d = fixture();
        write(d.path(), ".blueprint/bad.md", "+++\nowns = 3\n+++\n");
        write(d.path(), ".blueprint/z/dup.md", "+++\nnode = \"ui\"\n+++\n");
        write(d.path(), ".blueprint/typo.md", "+++\nown = [\"x\"]\n+++\n");
        let bp = Blueprint::load(d.path()).unwrap();
        let text = bp.check().render();
        assert!(text.contains(".blueprint/bad.md"), "{text}");
        assert!(text.contains("duplicate node name `ui`"), "{text}");
        assert!(
            text.contains(".blueprint/typo.md"),
            "unknown keys rejected: {text}"
        );
    }
    #[test]
    fn hidden_dirs_are_exempt_in_check_turn_like_in_check() {
        let d = fixture();
        let root = d.path();
        write(root, ".github/workflows/ci.yml", "");
        let bp = Blueprint::load(root).unwrap();
        assert!(bp
            .check_turn(&[root.join(".github/workflows/ci.yml")])
            .is_clean());
        assert!(!bp.check().render().contains(".github"));
    }

    #[test]
    fn malformed_fence_is_an_error_but_readme_is_prose() {
        let d = fixture();
        write(
            d.path(),
            ".blueprint/broken.md",
            "++\nowns = [\"x\"]\n+++\n",
        );
        write(
            d.path(),
            ".blueprint/unclosed.md",
            "+++\nowns = [\"x\"]\nbody\n",
        );
        write(d.path(), ".blueprint/notes.md", "# plain notes\n");
        let bp = Blueprint::load(d.path()).unwrap();
        let text = bp.check().render();
        assert!(text.contains(".blueprint/broken.md"), "{text}");
        assert!(text.contains(".blueprint/unclosed.md"), "{text}");
        assert!(!text.contains("notes.md"), "{text}");
        assert!(!text.contains("README.md"), "{text}");
    }

    #[test]
    fn dead_and_overlapping_owns_warn() {
        let d = fixture();
        write(
            d.path(),
            ".blueprint/stale.md",
            "+++\nowns = [\"old_dir/**\", \".hidden/**\"]\n+++\nStale.\n",
        );
        write(
            d.path(),
            ".blueprint/dup.md",
            "+++\nowns = [\"crates/ui/**\"]\n+++\nOverlaps ui.\n",
        );
        let r = Blueprint::load(d.path()).unwrap().check();
        let text = r.render();
        assert!(
            text.contains("[stale] `owns` pattern `old_dir/**` matches no files"),
            "{text}"
        );
        assert!(
            !text.contains(".hidden"),
            "hidden globs are skipped: {text}"
        );
        assert!(text.contains("also owned by"), "{text}");
        assert!(
            text.contains("[dup] 2 file(s) also owned by `ui`"),
            "{text}"
        );
        assert!(
            r.issues
                .iter()
                .filter(|i| i.message.contains("also owned by") || i.message.contains("matches no"))
                .all(|i| i.severity == Severity::Warning),
            "ownership hygiene never blocks a turn: {text}"
        );
    }
}
