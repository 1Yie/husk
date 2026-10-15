//! Source-level import edges — the half of the graph manifests can't see:
//! two nodes inside ONE package (kernel submodules, frontend features).
//!
//! Line-based, not a parser: it reads the import forms that name a path
//! inside the workspace and ignores everything else.
//!
//! | Language | Form                                   | Resolves against        |
//! |----------|----------------------------------------|-------------------------|
//! | Rust     | `use crate::a::b` (incl. `{…}` groups, multi-line) and inline `crate::a::b` paths | the crate's `src/` |
//! | TS / JS  | `from "./x"`, `import("../x")`, `require("./x")` | the importing file |
//! | Go       | `"<module>/pkg"` inside `import`       | the `go.mod` directory  |
//!
//! Cross-package edges are the manifests' job, except where one package holds
//! several nodes: there a manifest can only credit the node that owns it, so
//! Rust sources also report the crates they name ([`rust_crate_refs`]).
//! Bare JS specifiers and path aliases (`@/…`) are skipped rather than guessed.

/// Files larger than this are skipped — generated code, bundles.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Workspace-relative import targets of one source file, extensionless
/// where the language allows it — callers resolve them to owning nodes.
pub fn targets(rel: &str, text: &str, go_modules: &[(String, String)]) -> Vec<String> {
    let ext = rel.rsplit('.').next().unwrap_or("");
    let mut out = match ext {
        "rs" => rust(rel, text),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => js(rel, text),
        "go" => go(text, go_modules),
        _ => Vec::new(),
    };
    // `use a::{X, Y}` names module `a` twice — keep first occurrences.
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.clone()));
    out
}

/// Crate names a Rust file refers to by path — `agent_sandbox::run` yields
/// `agent_sandbox`. Deliberately loose (`std`, local modules and enum paths
/// come out too): the caller keeps only names some node's package provides.
/// String literals and everything from `#[cfg(test)]` on are skipped — a
/// fixture mentioning `agent_llm_core::x` is not a dependency.
pub fn rust_crate_refs(text: &str) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for raw in text.lines() {
        if raw.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        let line = strip_literals(raw);
        let bytes = line.as_bytes();
        let mut i = 0;
        while let Some(off) = line[i..].find("::") {
            let at = i + off;
            let start = bytes[..at]
                .iter()
                .rposition(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
                .map_or(0, |p| p + 1);
            // Only the head of a path names a crate: skip segments that
            // continue one (`a::b`), or open inside a `use a::{b::c}` group
            // or a generic/call argument list.
            let before = line[..start].trim_end();
            let continues = before.ends_with("::")
                || before.ends_with('{')
                || before.ends_with(',') && line[..start].contains('{');
            if start < at && !continues {
                out.insert(line[start..at].to_string());
            }
            i = at + 2;
        }
    }
    out.into_iter().collect()
}

/// One line with `"…"` contents and a trailing `//` comment blanked out.
fn strip_literals(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut prev = '\0';
    for c in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push('"');
        } else if c == '/' && prev == '/' {
            out.pop();
            break;
        } else {
            out.push(c);
        }
        prev = c;
    }
    out
}

/// Candidate spellings of a target — owner globs may name files with
/// extensions (`src/x.rs`) or directories (`src/x/**`).
pub fn candidates(target: &str) -> Vec<String> {
    let mut out = vec![target.to_string()];
    for suffix in [
        ".rs",
        "/mod.rs",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        "/index.ts",
        "/index.tsx",
        "/index.js",
    ] {
        out.push(format!("{target}{suffix}"));
    }
    out
}

fn rust(rel: &str, text: &str) -> Vec<String> {
    // `crates/k/src/tools/x.rs` → crate source root `crates/k/src`.
    let Some(idx) = rel
        .rfind("/src/")
        .map(|i| i + 4)
        .or_else(|| rel.starts_with("src/").then_some(3))
    else {
        return Vec::new();
    };
    let src_root = &rel[..idx];
    let mut out = Vec::new();
    // A `use` statement may span lines — collect its body up to the `;`.
    let mut pending: Option<String> = None;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if let Some(stmt) = pending.as_mut() {
            stmt.push(' ');
            stmt.push_str(line);
            if line.contains(';') {
                let done = pending.take().unwrap_or_default();
                use_targets(src_root, &done, &mut out);
            }
            continue;
        }
        let rest = line
            .strip_prefix("pub use ")
            .or_else(|| line.strip_prefix("use "))
            .or_else(|| {
                line.strip_prefix("pub(crate) use ")
                    .or_else(|| line.strip_prefix("pub(super) use "))
            });
        if let Some(rest) = rest {
            if line.contains(';') {
                use_targets(src_root, rest, &mut out);
            } else {
                pending = Some(rest.to_string());
            }
            continue;
        }
        inline_targets(src_root, line, &mut out);
    }
    out
}

/// Targets of one `use` body (`crate::a::{b, c}` — prefix and `;` included).
fn use_targets(src_root: &str, body: &str, out: &mut Vec<String>) {
    let Some(rest) = body.trim().strip_prefix("crate::") else {
        return;
    };
    let rest = rest.trim_end_matches(';').trim();
    for path in expand_group(rest) {
        out.extend(module_targets(src_root, &path));
    }
}

/// `crate::a::b(..)` written out in code rather than imported.
fn inline_targets(src_root: &str, line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(i) = rest.find("crate::") {
        let before_ok = rest[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = &rest[i + "crate::".len()..];
        let end = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(after.len());
        if before_ok {
            out.extend(module_targets(src_root, &after[..end]));
        }
        rest = after;
    }
}

/// `a::b::Type::new` → [`<src_root>/a/b`, `<src_root>/a`] — the longest
/// lowercase module prefix, then each shorter one: a lowercase tail may be
/// a function (`crate::mode::resolve`), not a module, and only the shorter
/// prefix names the file that owns it. The first type, `*` or `self` ends
/// the path.
fn module_targets(src_root: &str, path: &str) -> Vec<String> {
    let segs: Vec<&str> = path
        .split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "*" && *s != "self")
        .map(|s| s.split(" as ").next().unwrap_or(s).trim())
        .collect();
    let modules: Vec<&str> = segs
        .iter()
        .copied()
        .take_while(|s| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_lowercase() || c == '_')
        })
        .collect();
    (1..=modules.len())
        .rev()
        .map(|n| format!("{src_root}/{}", modules[..n].join("/")))
        .collect()
}

/// `a::{b, c::d}` → [`a::b`, `a::c::d`]; a group-free path passes through.
/// One level of nesting is expanded — deeper groups keep their prefix.
fn expand_group(path: &str) -> Vec<String> {
    let Some(open) = path.find('{') else {
        return vec![path.to_string()];
    };
    let prefix = &path[..open];
    let inner = path[open + 1..].trim_end_matches('}');
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(format!("{prefix}{}", inner[start..i].trim()));
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        out.push(format!("{prefix}{tail}"));
    }
    out.into_iter()
        .map(|p| {
            p.split('{')
                .next()
                .unwrap_or(&p)
                .trim_end_matches("::")
                .to_string()
        })
        .collect()
}

fn js(rel: &str, text: &str) -> Vec<String> {
    let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut out = Vec::new();
    for line in text.lines() {
        for spec in quoted_after(line, &["from ", "import(", "require(", "import "]) {
            if spec.starts_with("./") || spec.starts_with("../") {
                if let Some(p) = join(dir, spec) {
                    out.push(strip_js_ext(&p).to_string());
                }
            }
        }
    }
    out
}

fn go(text: &str, modules: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("import (") {
            in_block = true;
            continue;
        }
        if in_block && t.starts_with(')') {
            in_block = false;
            continue;
        }
        if !(in_block || t.starts_with("import ")) {
            continue;
        }
        let Some(spec) = t.split('"').nth(1) else {
            continue;
        };
        for (module, dir) in modules {
            if let Some(sub) = spec.strip_prefix(module.as_str()) {
                if sub.is_empty() || sub.starts_with('/') {
                    let sub = sub.trim_start_matches('/');
                    out.push(match (dir.is_empty(), sub.is_empty()) {
                        (true, _) => sub.to_string(),
                        (false, true) => dir.clone(),
                        (false, false) => format!("{dir}/{sub}"),
                    });
                }
            }
        }
    }
    out
}

/// String literals that follow any of `keys` on the line.
fn quoted_after<'a>(line: &'a str, keys: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    for key in keys {
        let mut rest = line;
        while let Some(i) = rest.find(key) {
            let after = rest[i + key.len()..].trim_start();
            if let Some(q) = after
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\'' | '`'))
            {
                if let Some(end) = after[1..].find(q) {
                    out.push(&after[1..1 + end]);
                }
            }
            rest = &rest[i + key.len()..];
        }
    }
    out
}

/// `dir` + relative `spec`, normalized — `None` when it climbs above root.
fn join(dir: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn strip_js_ext(p: &str) -> &str {
    for ext in [".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs"] {
        if let Some(s) = p.strip_suffix(ext) {
            return s;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_crate_paths_and_groups() {
        let t = targets(
            "crates/k/src/engine.rs",
            "use crate::tools::registry::{ToolCtx, ToolRegistry};\n\
             use crate::{mode::AgentMode, hooks};\n\
             pub(crate) use crate::session_store::files;\n\
             use agent_llm_core::types::ChatMessage;\n\
             use super::x;\n",
            &[],
        );
        assert_eq!(
            t,
            [
                "crates/k/src/tools/registry",
                "crates/k/src/tools",
                "crates/k/src/mode",
                "crates/k/src/hooks",
                "crates/k/src/session_store/files",
                "crates/k/src/session_store",
            ]
        );
    }

    #[test]
    fn rust_multiline_use_and_inline_paths() {
        let t = targets(
            "crates/k/src/engine.rs",
            "use crate::{\n    mode::AgentMode, // the mode\n    hooks,\n};\n\
             // use crate::commented::out;\n\
             fn f() { let s = crate::skills::SkillManager::new(); crate::permissions::check(); }\n\
             pub(crate) fn g() {}\n",
            &[],
        );
        assert_eq!(
            t,
            [
                "crates/k/src/mode",
                "crates/k/src/hooks",
                "crates/k/src/skills",
                "crates/k/src/permissions/check",
                "crates/k/src/permissions",
            ]
        );
    }

    #[test]
    fn crate_refs_take_the_first_path_segment_only() {
        let r = rust_crate_refs(
            "use agent_sandbox::{Backend, plan::Plan};\n\
             let x = other_crate::run(std::fs::read(p)?); // ignored::comment\n\
             pub use crate::a::b;\n\
             let s = \"use agent_llm_core::x;\";\n",
        );
        assert_eq!(r, ["agent_sandbox", "crate", "other_crate", "std"]);
        let r = rust_crate_refs("use a::b;\n#[cfg(test)]\nmod t { use c::d; }\n");
        assert_eq!(r, ["a"]);
    }

    #[test]
    fn js_relative_specifiers_only() {
        let t = targets(
            "web/src/features/chat/view.tsx",
            "import { a } from \"../shared/a\";\n\
             import b from '@/lib/b';\n\
             const c = await import('./c.js');\n\
             import \"./side-effect.css\";\n\
             import React from \"react\";\n",
            &[],
        );
        assert_eq!(
            t,
            [
                "web/src/features/shared/a",
                "web/src/features/chat/c",
                "web/src/features/chat/side-effect.css",
            ]
        );
    }

    #[test]
    fn go_module_imports_resolve_to_dirs() {
        let mods = vec![("example.com/app".to_string(), "svc".to_string())];
        let t = targets(
            "svc/cmd/main.go",
            "package main\n\nimport (\n\t\"fmt\"\n\tdb \"example.com/app/internal/db\"\n)\nimport \"example.com/application/x\"\n",
            &mods,
        );
        assert_eq!(t, ["svc/internal/db"]);
    }
}
