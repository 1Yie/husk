//! Standing constraints — fine-grained `*.md` rule files.
//!
//! A rule is not a skill: a skill is a procedure the model loads on demand,
//! a rule is a constraint that holds every turn. Rules live in two scopes —
//! the host's config dir (`<config>/rules/`) and the workspace's own state dir
//! (`<workspace_dir>/rules/`) — and a workspace file shadows the global one of
//! the same file stem.
//!
//! ```text
//! ---
//! description: one line
//! globs: src/**/*.rs, *.toml     # optional; comma list or [a, b]
//! always: true                   # default: true without globs, false with them
//! ---
//! body
//! ```

use std::path::{Path, PathBuf};

use agent_paths::AppPaths;

/// One rule file, parsed. `name` is the file stem — what the model and the
/// settings editor call it, and what decides shadowing between the scopes.
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub description: String,
    /// Patterns the rule applies to. Empty = no pattern scope of its own.
    pub globs: Vec<String>,
    /// Inlined into every prompt. `false` moves the rule to the index, where
    /// the model reads it only when it touches a matching file.
    pub always: bool,
    pub body: String,
    pub path: String,
    pub global: bool,
}

/// Section marker of the rules block. The resume path truncates the persisted
/// prompt here to re-render the rules from disk, so the literal has to stay put.
pub const PROMPT_MARKER: &str = "\n\n## Rules\n";

/// How much inlined rule text one prompt may carry. Bodies past the budget move
/// to the index — a rule file is documentation, so the model can still read it.
pub const INLINE_BUDGET: usize = 16 * 1024;

/// Where workspace-scoped rules live — the host's own workspace state dir, next
/// to its `agents/` and `attachments/`.
pub fn workspace_rules_dir(root: &Path, paths: &AppPaths) -> PathBuf {
    paths.workspace_dir(root).join("rules")
}

/// Where user-scoped rules live — the config dir `AGENTS.md` also sits in.
pub fn global_rules_dir(paths: &AppPaths) -> PathBuf {
    paths.config_file("rules")
}

/// `<name>.md` in one scope. The file may not exist; this is the path the
/// settings editor writes and deletes.
pub fn rule_file(root: &Path, paths: &AppPaths, name: &str, global: bool) -> PathBuf {
    let dir = if global {
        global_rules_dir(paths)
    } else {
        workspace_rules_dir(root, paths)
    };
    dir.join(format!("{name}.md"))
}

/// The file a name currently resolves to — workspace first, which is the same
/// order [`discover_rules`] lets that copy shadow the global one.
pub fn find_rule_file(root: &Path, paths: &AppPaths, name: &str) -> Option<PathBuf> {
    [false, true]
        .into_iter()
        .map(|global| rule_file(root, paths, name, global))
        .find(|p| p.is_file())
}

/// Scan both rule dirs. Workspace first so its copy shadows a global one, then
/// sorted by name so the prompt and the settings list agree.
pub fn discover_rules(root: &Path, paths: &AppPaths) -> Vec<Rule> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (dir, global) in [
        (workspace_rules_dir(root, paths), false),
        (global_rules_dir(paths), true),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() || p.extension().is_none_or(|x| x != "md") {
                continue;
            }
            let name = p
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }
            let content = std::fs::read_to_string(&p).unwrap_or_default();
            let (fm, body) = split_rule(&content);
            out.push(Rule {
                name,
                description: fm.description,
                always: fm.always.unwrap_or(fm.globs.is_empty()),
                globs: fm.globs,
                body,
                path: p.to_string_lossy().into_owned(),
                global,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The prompt's `## Rules` section. Always-on rules are inlined verbatim —
/// they are constraints, and a constraint the model has to fetch is one it
/// will forget. Pattern-scoped ones stay in an index with their own file path,
/// so a rule about `*.rs` costs nothing in an unrelated conversation. Empty
/// when nothing is installed.
pub fn prompt_block(rules: &[Rule]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let mut inline = String::new();
    let mut index: Vec<String> = Vec::new();
    let mut used = 0usize;
    for rule in rules {
        let body = rule.body.trim();
        if rule.always && !body.is_empty() && used + body.len() <= INLINE_BUDGET {
            used += body.len();
            inline.push_str(&format!("### {}\n{body}\n\n", rule.name));
            continue;
        }
        // Over the budget, or an `always` rule with no body to inline: the
        // index still names it, so the rule is not silently lost.
        index.push(index_line(rule, rule.always && !body.is_empty()));
    }
    let mut out = String::from(PROMPT_MARKER);
    out.push_str(
        "\nStanding constraints from this project's rule files. They hold for every turn — \
         unlike skills, which are procedures loaded on demand.\n",
    );
    if !inline.is_empty() {
        out.push('\n');
        out.push_str(inline.trim_end());
        out.push('\n');
    }
    if !index.is_empty() {
        out.push_str(
            "\nPattern-scoped rules — before editing a file that matches one, read the rule file \
             with `smart_read`:\n",
        );
        out.push_str(&index.join("\n"));
        out.push('\n');
    }
    out
}

/// One index row: name, description, the patterns it scopes to, the file to
/// read. `overflow` marks an always-on rule that did not fit the inline budget.
fn index_line(rule: &Rule, overflow: bool) -> String {
    let description = match rule.description.trim() {
        "" => "(no description)",
        d => d,
    };
    let globs = if rule.globs.is_empty() {
        "no globs".to_string()
    } else {
        rule.globs.join(", ")
    };
    let note = if overflow {
        " — too large to inline, read it with `smart_read`"
    } else {
        ""
    };
    format!(
        "- `{}` — {description} (applies to: {globs}) — {}{note}",
        rule.name, rule.path
    )
}

/// Render a rule file for the settings editor. `always` is written as given:
/// the editor's choice is what [`discover_rules`] then reads back.
pub fn render_rule(description: &str, globs: &[String], always: bool, body: &str) -> String {
    let mut out = String::from("---\n");
    let description = description.trim();
    if !description.is_empty() {
        out.push_str(&format!("description: {description}\n"));
    }
    let globs: Vec<&str> = globs
        .iter()
        .map(|g| g.trim())
        .filter(|g| !g.is_empty())
        .collect();
    if !globs.is_empty() {
        out.push_str(&format!("globs: {}\n", globs.join(", ")));
    }
    out.push_str(&format!("always: {always}\n---\n\n{}\n", body.trim()));
    out
}

/// Write one rule, removing the same-named file in the other scope.
///
/// A scope switch that left both copies behind would never take effect: the
/// workspace file keeps shadowing the global one, so the edit looks saved and
/// changes nothing.
pub fn save_rule(
    root: &Path,
    paths: &AppPaths,
    name: &str,
    global: bool,
    description: &str,
    globs: &[String],
    always: bool,
    body: &str,
) -> std::io::Result<PathBuf> {
    // The name becomes a file stem; a separator would escape the rule dir.
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid rule name",
        ));
    }
    let target = rule_file(root, paths, name, global);
    let other = rule_file(root, paths, name, !global);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&target, render_rule(description, globs, always, body))?;
    if other.is_file() {
        std::fs::remove_file(&other)?;
    }
    Ok(target)
}

/// A rule file's frontmatter. `always` stays optional: an absent line means
/// "default", which depends on whether the rule is pattern-scoped.
#[derive(Debug, Default, PartialEq)]
pub struct RuleFrontmatter {
    pub description: String,
    pub globs: Vec<String>,
    pub always: Option<bool>,
}

/// Frontmatter + body. Same `---` convention as subagent manifests; a file with
/// no frontmatter is all body.
pub fn split_rule(content: &str) -> (RuleFrontmatter, String) {
    let trimmed = content.trim_start();
    let Some(fm) = trimmed.strip_prefix("---") else {
        return (RuleFrontmatter::default(), content.trim().to_string());
    };
    let Some(end) = fm.find("\n---") else {
        return (RuleFrontmatter::default(), content.trim().to_string());
    };
    let mut out = RuleFrontmatter::default();
    for line in fm[..end].lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("globs:") {
            let v = v.trim().trim_start_matches('[').trim_end_matches(']');
            out.globs = v
                .split(',')
                .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|t| !t.is_empty())
                .collect();
        } else if let Some(v) = line.strip_prefix("always:") {
            out.always = Some(v.trim().eq_ignore_ascii_case("true"));
        } else if let Some(v) = line.strip_prefix("description:") {
            out.description = v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    (out, fm[end + 4..].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_paths(dir: &Path) -> AppPaths {
        AppPaths::new(dir.join("husk"), dir, ".husk")
    }

    fn write_rule(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{name}.md")), text).unwrap();
    }

    /// `always` follows the globs unless the file says otherwise — that default
    /// is what keeps a bare `*.md` rule a standing constraint.
    #[test]
    fn frontmatter_defaults_follow_the_globs() {
        let (bare, body) = split_rule("---\ndescription: d\n---\n\nBODY\n");
        assert_eq!(bare.description, "d");
        assert!(bare.globs.is_empty());
        assert_eq!(bare.always, None);
        assert_eq!(body, "BODY");

        let (flow, _) = split_rule("---\nglobs: [src/**/*.rs, *.toml]\nalways: true\n---\nB");
        assert_eq!(
            flow.globs,
            vec!["src/**/*.rs".to_string(), "*.toml".to_string()]
        );
        assert_eq!(flow.always, Some(true));

        let (comma, _) = split_rule("---\nglobs: src/**/*.rs, *.toml\n---\nB");
        assert_eq!(
            comma.globs,
            vec!["src/**/*.rs".to_string(), "*.toml".to_string()]
        );
        assert_eq!(comma.always, None);

        // No frontmatter at all is still a rule — all body.
        let (none, body) = split_rule("JUST BODY");
        assert_eq!(none, RuleFrontmatter::default());
        assert_eq!(body, "JUST BODY");
    }

    /// A workspace rule shadows the global one of the same stem, and both the
    /// shadowing copy and a lone global rule are discovered once.
    #[test]
    fn workspace_rule_shadows_the_global_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        write_rule(
            &global_rules_dir(&paths),
            "style",
            "---\ndescription: global\n---\nGLOBAL",
        );
        write_rule(
            &workspace_rules_dir(dir.path(), &paths),
            "style",
            "---\ndescription: project\n---\nPROJECT",
        );
        write_rule(&global_rules_dir(&paths), "shared", "SHARED");

        let found = discover_rules(dir.path(), &paths);
        let hits: Vec<&Rule> = found.iter().filter(|r| r.name == "style").collect();
        assert_eq!(hits.len(), 1, "{found:?}");
        assert!(!hits[0].global);
        assert_eq!(hits[0].body, "PROJECT");
        assert_eq!(hits[0].description, "project");
        // Sorted by name, and a global-only rule still shows up.
        let names: Vec<&str> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["shared", "style"], "{found:?}");
        assert!(!found[1].global);
        assert!(found[0].global);
    }

    /// Always-on rules are inlined; pattern-scoped ones stay in an index that
    /// carries the file to read.
    #[test]
    fn prompt_block_inlines_always_and_indexes_globs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        write_rule(
            &workspace_rules_dir(dir.path(), &paths),
            "commit",
            "---\ndescription: no commits\n---\nNEVER COMMIT",
        );
        write_rule(
            &workspace_rules_dir(dir.path(), &paths),
            "rust",
            "---\ndescription: rust style\nglobs: src/**/*.rs\n---\nUSE EDITION 2021",
        );

        let block = prompt_block(&discover_rules(dir.path(), &paths));
        assert!(block.starts_with(PROMPT_MARKER), "{block}");
        // The always-on body is in the prompt, with its name as the heading.
        assert!(block.contains("### commit\nNEVER COMMIT"), "{block}");
        assert!(!block.contains("USE EDITION 2021"), "not inlined: {block}");
        // The scoped rule is an index row naming its file.
        assert!(
            block.contains("`rust` — rust style (applies to: src/**/*.rs) — "),
            "{block}"
        );
        assert!(block.contains("smart_read"), "{block}");
        let rust_file = workspace_rules_dir(dir.path(), &paths)
            .join("rust.md")
            .to_string_lossy()
            .into_owned();
        assert!(block.contains(&rust_file), "{block}");

        assert!(prompt_block(&[]).is_empty(), "no rules, no block");
    }

    /// The inline budget is a hard ceiling: a body past it moves to the index
    /// with a note instead of eating the prompt.
    #[test]
    fn prompt_block_caps_the_inlined_bodies() {
        let body = "x".repeat(INLINE_BUDGET + 1);
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        write_rule(
            &workspace_rules_dir(dir.path(), &paths),
            "huge",
            &format!("---\ndescription: huge\n---\n{body}"),
        );
        write_rule(
            &workspace_rules_dir(dir.path(), &paths),
            "small",
            "---\ndescription: small\n---\nSMALL",
        );

        let block = prompt_block(&discover_rules(dir.path(), &paths));
        assert!(!block.contains(&body), "the body must not be inlined");
        assert!(block.contains("### small\nSMALL"), "{block}");
        assert!(block.contains("`huge` — huge"), "{block}");
        assert!(block.contains("too large to inline"), "{block}");
        assert!(
            block.len() < INLINE_BUDGET,
            "prompt block grew to {} bytes",
            block.len()
        );
    }

    /// Saving into the other scope moves the rule: one file, one row, and the
    /// discovered copy is the one that was just written.
    #[test]
    fn save_rule_moves_scope_and_drops_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        let globs = vec!["src/**/*.rs".to_string()];

        let ws = save_rule(
            dir.path(),
            &paths,
            "style",
            false,
            "project style",
            &globs,
            false,
            "PROJECT",
        )
        .unwrap();
        assert_eq!(ws, rule_file(dir.path(), &paths, "style", false));
        assert_eq!(
            find_rule_file(dir.path(), &paths, "style"),
            Some(ws.clone())
        );
        let rule = discover_rules(dir.path(), &paths).remove(0);
        assert!(!rule.global);
        assert_eq!(rule.body, "PROJECT");
        assert_eq!(rule.globs, globs);
        assert!(!rule.always, "globs given, no `always:` line");

        let gl = save_rule(
            dir.path(),
            &paths,
            "style",
            true,
            "user style",
            &[],
            true,
            "GLOBAL",
        )
        .unwrap();
        assert_eq!(gl, rule_file(dir.path(), &paths, "style", true));
        assert!(!ws.exists(), "the workspace file must be gone");
        let rule = discover_rules(dir.path(), &paths).remove(0);
        assert!(rule.global);
        assert_eq!(rule.description, "user style");
        assert!(rule.always && rule.globs.is_empty());

        let found = discover_rules(dir.path(), &paths);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            find_rule_file(dir.path(), &paths, "style"),
            Some(gl),
            "the global copy is the one left"
        );
        assert_eq!(find_rule_file(dir.path(), &paths, "missing"), None);
    }

    /// A name is a file stem: anything that could leave the rule dir is refused
    /// before a write.
    #[test]
    fn save_rule_refuses_a_name_that_is_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let paths = app_paths(dir.path());
        assert!(save_rule(dir.path(), &paths, "../escape", false, "", &[], true, "B").is_err());
        assert!(save_rule(dir.path(), &paths, "", false, "", &[], true, "B").is_err());
    }
}
