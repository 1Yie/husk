//! Package-manifest parsing — the code-derived half of the blueprint graph.
//!
//! A manifest yields the package names it *declares* and the runtime
//! dependency names it *requires*. Dev/test dependencies are deliberately
//! excluded: an architecture rule ("the kernel never links a UI toolkit")
//! is about what ships, not what a test harness pulls in.

use std::path::Path;

/// Manifest file names recognised, in no particular order.
pub const MANIFEST_NAMES: [&str; 4] = ["Cargo.toml", "package.json", "go.mod", "pyproject.toml"];

/// What one manifest declares.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Package names this manifest publishes (`[package] name`, `"name"`, …).
    pub provides: Vec<String>,
    /// Runtime dependency names (renamed Cargo deps contribute both names).
    pub requires: Vec<String>,
}

/// Parse a manifest by its file name — `None` for an unknown name or a file
/// that doesn't parse (a broken manifest is the build's problem to report).
pub fn parse(path: &Path, text: &str) -> Option<Manifest> {
    match path.file_name()?.to_str()? {
        "Cargo.toml" => cargo(text),
        "package.json" => npm(text),
        "go.mod" => Some(gomod(text)),
        "pyproject.toml" => pyproject(text),
        _ => None,
    }
}

fn cargo(text: &str) -> Option<Manifest> {
    let doc: toml::Table = toml::from_str(text).ok()?;
    let mut m = Manifest::default();
    if let Some(name) = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
    {
        m.provides.push(name.to_string());
    }
    let mut tables: Vec<&toml::Table> = Vec::new();
    for key in ["dependencies", "build-dependencies"] {
        if let Some(t) = doc.get(key).and_then(|v| v.as_table()) {
            tables.push(t);
        }
    }
    // `[target.'cfg(..)'.dependencies]` — platform deps still ship.
    if let Some(targets) = doc.get("target").and_then(|v| v.as_table()) {
        for t in targets.values() {
            for key in ["dependencies", "build-dependencies"] {
                if let Some(d) = t.get(key).and_then(|v| v.as_table()) {
                    tables.push(d);
                }
            }
        }
    }
    for t in tables {
        for (key, spec) in t {
            m.requires.push(key.clone());
            // `foo = { package = "bar" }` — the real crate is `bar`.
            if let Some(pkg) = spec.get("package").and_then(|p| p.as_str()) {
                m.requires.push(pkg.to_string());
            }
        }
    }
    Some(m)
}

fn npm(text: &str) -> Option<Manifest> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut m = Manifest::default();
    if let Some(name) = doc.get("name").and_then(|n| n.as_str()) {
        m.provides.push(name.to_string());
    }
    for key in ["dependencies", "peerDependencies", "optionalDependencies"] {
        if let Some(obj) = doc.get(key).and_then(|v| v.as_object()) {
            m.requires.extend(obj.keys().cloned());
        }
    }
    Some(m)
}

fn gomod(text: &str) -> Manifest {
    let mut m = Manifest::default();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("module ") {
            m.provides.push(rest.trim().to_string());
        } else if line == "require (" {
            in_block = true;
        } else if in_block && line == ")" {
            in_block = false;
        } else if let Some(rest) = line.strip_prefix("require ") {
            if let Some(dep) = rest.split_whitespace().next() {
                m.requires.push(dep.to_string());
            }
        } else if in_block {
            if let Some(dep) = line.split_whitespace().next() {
                m.requires.push(dep.to_string());
            }
        }
    }
    m
}

fn pyproject(text: &str) -> Option<Manifest> {
    let doc: toml::Table = toml::from_str(text).ok()?;
    let mut m = Manifest::default();
    let project = doc.get("project");
    if let Some(name) = project.and_then(|p| p.get("name")).and_then(|n| n.as_str()) {
        m.provides.push(name.to_string());
    }
    if let Some(deps) = project
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for d in deps.iter().filter_map(|d| d.as_str()) {
            // PEP 508: the name ends at the first version/extra/marker char.
            let name: String = d
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            if !name.is_empty() {
                m.requires.push(name);
            }
        }
    }
    let poetry = doc.get("tool").and_then(|t| t.get("poetry"));
    if let Some(name) = poetry.and_then(|p| p.get("name")).and_then(|n| n.as_str()) {
        m.provides.push(name.to_string());
    }
    if let Some(deps) = poetry
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        m.requires
            .extend(deps.keys().filter(|k| k.as_str() != "python").cloned());
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_keeps_runtime_deps_and_renames() {
        let m = parse(
            Path::new("Cargo.toml"),
            r#"
[package]
name = "kernel"
[dependencies]
reqwest.workspace = true
web = { package = "actix-web", version = "4" }
[dev-dependencies]
tempfile = "3"
[target.'cfg(unix)'.dependencies]
nix = "0.29"
"#,
        )
        .unwrap();
        assert_eq!(m.provides, ["kernel"]);
        assert!(m.requires.contains(&"reqwest".to_string()));
        assert!(m.requires.contains(&"actix-web".to_string()));
        assert!(m.requires.contains(&"nix".to_string()));
        assert!(!m.requires.contains(&"tempfile".to_string()));
    }

    #[test]
    fn npm_skips_dev_dependencies() {
        let m = parse(
            Path::new("package.json"),
            r#"{"name":"web","dependencies":{"react":"18"},"devDependencies":{"vite":"5"}}"#,
        )
        .unwrap();
        assert_eq!(m.provides, ["web"]);
        assert_eq!(m.requires, ["react"]);
    }

    #[test]
    fn gomod_reads_single_and_block_requires() {
        let m = parse(
            Path::new("go.mod"),
            "module example.com/app\n\nrequire github.com/a/b v1.0.0\nrequire (\n\tgithub.com/c/d v2 // indirect\n)\n",
        )
        .unwrap();
        assert_eq!(m.provides, ["example.com/app"]);
        assert_eq!(m.requires, ["github.com/a/b", "github.com/c/d"]);
    }

    #[test]
    fn pyproject_strips_version_specifiers() {
        let m = parse(
            Path::new("pyproject.toml"),
            "[project]\nname = \"svc\"\ndependencies = [\"requests>=2\", \"pydantic[email]\"]\n",
        )
        .unwrap();
        assert_eq!(m.requires, ["requests", "pydantic"]);
    }
}
