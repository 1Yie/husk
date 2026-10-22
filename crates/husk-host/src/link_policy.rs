//! Linked workspaces — the host's allow-list of directories outside the
//! session's own workspace.
//!
//! A project can span repositories (a kernel, its extensions, the app that
//! consumes them). The file tools reach exactly one root by construction, so
//! the host grants more: a link widens *path reachability* and nothing else.
//! Relative paths, `.blueprint/`, the memory partition and the git headline
//! stay the workspace's, because a second meaning for "here" would make every
//! tool answer depend on which root happened to match.
//!
//! Two objects, on purpose:
//! - [`validate`] answers "may this directory be linked at all" — the refusal
//!   list lives here so the settings page, the composer and the IPC layer all
//!   give the same answer;
//! - [`normalize`] turns whatever the store and the session accumulated into
//!   the list the tools use: canonical, deduplicated, outermost-wins.

use std::path::{Path, PathBuf};

use agent_capabilities::{LinkedRoot, RootAccess};

/// Directories that must never be reachable through a link: system trees, and
/// the places credentials live. A link to a *parent* of one of these is the
/// same grant as linking it, so the check is resolved against both sides.
const SYSTEM_DIRS: [&str; 10] = [
    "/etc", "/usr", "/bin", "/sbin", "/boot", "/dev", "/proc", "/sys", "/var", "/lib",
];

/// Why `path` cannot be linked into `workspace_root`.
///
/// `Ok` is the canonical path. Nothing here depends on the session: the same
/// call decides what the settings page may store and what a running session
/// may add.
pub fn validate(
    path: &Path,
    workspace_root: &Path,
    paths: &agent_paths::AppPaths,
) -> Result<PathBuf, String> {
    let Some(canon) = canonical(path) else {
        return Err(format!("{} 不是一个可用的目录", path.display()));
    };
    let ws = canonical(workspace_root).unwrap_or_else(|| workspace_root.to_path_buf());
    if canon == ws || canon.starts_with(&ws) {
        return Err(format!("{} 已经在当前工作区内", canon.display()));
    }
    if ws.starts_with(&canon) {
        // Linking an ancestor hands over the workspace's own siblings too —
        // and makes "relative path" ambiguous for everything under it.
        return Err(format!(
            "{} 包含当前工作区，不能作为关联工作区",
            canon.display()
        ));
    }
    if canon.parent().is_none() {
        return Err("不能关联文件系统根目录".into());
    }
    if let Some(home) = home() {
        if canon == home {
            return Err("不能关联整个主目录".into());
        }
    }
    for dir in SYSTEM_DIRS {
        let dir = PathBuf::from(dir);
        if canon == dir || canon.starts_with(&dir) {
            return Err(format!("{} 是系统目录", canon.display()));
        }
    }
    for secret in secrets(paths) {
        if canon == secret || canon.starts_with(&secret) || secret.starts_with(&canon) {
            return Err(format!(
                "{} 会暴露 {} 里的密钥或应用数据",
                canon.display(),
                secret.display()
            ));
        }
    }
    Ok(canon)
}

/// Canonical and deduplicated. A duplicate path keeps the wider access; a
/// path nested inside another is KEPT, because the two may disagree about
/// access and the narrower grant is the one the user meant: `~/.config`
/// read-only plus `~/.config/nvim` read-write must not turn into "all of
/// `~/.config` is writable". Path scope resolves to the most specific root
/// (`WorkspaceCtx::scope_of`), so keeping both entries is what makes that
/// rule reachable — dropping the inner one would silently widen it.
pub fn normalize(roots: impl IntoIterator<Item = LinkedRoot>) -> Vec<LinkedRoot> {
    let mut out: Vec<LinkedRoot> = Vec::new();
    for r in roots {
        let Some(canon) = canonical(&r.path) else {
            continue;
        };
        match out.iter_mut().find(|k| k.path == canon) {
            // Same directory granted twice: the wider access wins, so the
            // order the two layers were read in cannot decide permissions.
            Some(existing) => {
                if r.access == RootAccess::ReadWrite {
                    existing.access = RootAccess::ReadWrite;
                }
            }
            None => out.push(LinkedRoot {
                path: canon,
                access: r.access,
            }),
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Do two stored paths name the same directory? Canonical when both exist
/// (so `/a/../b` and `/b` match), lexical when one is gone — unlink and
/// access changes must still work for a directory that was moved away or
/// unmounted, otherwise the grant outlives the user's decision.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => lexical(a) == lexical(b),
    }
}

/// `path` with `.` and `..` folded textually, for the missing-directory case.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}



/// The nearest directory above `path` that looks like a project root — what a
/// grant should name. The scan lives in `agent-capabilities`, beside the
/// refusal that suggests it.
pub fn suggest_root(path: &Path) -> Option<PathBuf> {
    agent_capabilities::suggest_root(path)
}

/// What a grant should point at for `path`: the project root above it when one
/// is detectable, otherwise the directory itself. A card names the *file* the
/// tool tried; a grant is always a directory.
pub fn grant_target(path: &Path) -> PathBuf {
    if let Some(root) = agent_capabilities::suggest_root(path) {
        return root;
    }
    if path.is_dir() {
        return path.to_path_buf();
    }
    path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
}
/// Access label for the wire (`"read"` / `"readwrite"`).
pub fn access_label(access: RootAccess) -> &'static str {
    access.as_str()
}

/// One grant as the frontends read it: absolute path, access label, and which
/// layer granted it (`workspace` survives the session, `session` does not).
pub fn root_json(root: &LinkedRoot, scope: &str) -> serde_json::Value {
    serde_json::json!({
        "path": root.path.to_string_lossy(),
        "access": access_label(root.access),
        "scope": scope,
    })
}

/// The access a stored label names. Anything unknown is read-only: a value
/// written by a future version must not silently grant writes.
pub fn access_from_label(label: &str) -> RootAccess {
    RootAccess::from_str(label)
}

fn canonical(path: &Path) -> Option<PathBuf> {
    let canon = path.canonicalize().ok()?;
    canon.is_dir().then_some(canon)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|h| h.canonicalize().ok())
}

/// Every path a link must not expose: the app's own data, and the credential
/// directories the sandbox masks anyway (a link would re-expose them to the
/// file tools, which see the real fs, not the sandbox).
fn secrets(paths: &agent_paths::AppPaths) -> Vec<PathBuf> {
    let mut out = crate::sandbox_prefs::protected_paths(paths);
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for name in [".ssh", ".gnupg", ".aws", ".kube"] {
            out.push(home.join(name));
        }
    }
    out.into_iter()
        .filter_map(|p| p.canonicalize().ok().or(Some(p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: tempfile::TempDir,
        ws: PathBuf,
        paths: agent_paths::AppPaths,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let base = root.path().canonicalize().unwrap();
            let ws = base.join("app");
            std::fs::create_dir_all(&ws).unwrap();
            let paths = agent_paths::AppPaths::new(
                base.join("config"),
                base.join("data"),
                ".host",
            );
            std::fs::create_dir_all(&paths.config_dir).unwrap();
            Self { root, ws, paths }
        }

        fn dir(&self, name: &str) -> PathBuf {
            let p = self.root.path().canonicalize().unwrap().join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }

    #[test]
    fn a_sibling_project_links() {
        let f = Fixture::new();
        let kernel = f.dir("kernel");
        assert_eq!(validate(&kernel, &f.ws, &f.paths).unwrap(), kernel);
    }

    #[test]
    fn the_workspace_and_its_children_are_not_links() {
        let f = Fixture::new();
        let sub = f.ws.join("crates");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(validate(&f.ws, &f.ws, &f.paths).is_err());
        assert!(validate(&sub, &f.ws, &f.paths).is_err());
    }

    #[test]
    fn an_ancestor_of_the_workspace_is_refused() {
        let f = Fixture::new();
        let base = f.root.path().canonicalize().unwrap();
        assert!(validate(&base, &f.ws, &f.paths).is_err());
    }

    #[test]
    fn system_and_root_paths_are_refused() {
        let f = Fixture::new();
        for p in ["/", "/etc", "/usr/lib", "/proc"] {
            assert!(validate(Path::new(p), &f.ws, &f.paths).is_err(), "{p}");
        }
    }

    #[test]
    fn the_home_directory_and_its_secret_children_are_refused() {
        let Ok(home) = std::env::var("HOME").map(PathBuf::from) else {
            return;
        };
        let paths = agent_paths::AppPaths::new(
            home.join(".config/probe"),
            home.join(".local/share/probe"),
            ".probe",
        );
        let ws = home.join("probe-ws");
        assert!(validate(&home, &ws, &paths).is_err(), "home itself");
        if home.join(".ssh").exists() {
            assert!(validate(&home.join(".ssh"), &ws, &paths).is_err());
            // A parent of a secret dir would expose it just the same.
            assert!(validate(&home, &ws, &paths).is_err());
        }
    }

    #[test]
    fn a_missing_directory_is_refused() {
        let f = Fixture::new();
        let gone = f.root.path().join("nope");
        assert!(validate(&gone, &f.ws, &f.paths).is_err());
    }

    #[test]
    fn normalize_keeps_a_nested_root_and_widens_only_exact_duplicates() {
        let f = Fixture::new();
        let outer = f.dir("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let other = f.dir("other");
        let roots = normalize(vec![
            LinkedRoot {
                path: inner.clone(),
                access: RootAccess::ReadWrite,
            },
            LinkedRoot {
                path: outer.clone(),
                access: RootAccess::Read,
            },
            LinkedRoot {
                path: other.clone(),
                access: RootAccess::ReadWrite,
            },
            LinkedRoot {
                path: other.clone(),
                access: RootAccess::Read,
            },
        ]);
        // Both survive, each with the access it was granted: the outer stays
        // read-only, so a read-write child cannot widen its parent.
        assert_eq!(
            roots,
            [
                LinkedRoot {
                    path: other,
                    access: RootAccess::ReadWrite
                },
                LinkedRoot {
                    path: outer,
                    access: RootAccess::Read
                },
                LinkedRoot {
                    path: inner,
                    access: RootAccess::ReadWrite
                },
            ]
        );
    }

    #[test]
    fn same_path_matches_canonical_names_and_deleted_directories() {
        let f = Fixture::new();
        let dir = f.dir("gone-soon");
        assert!(same_path(&dir, &dir.join(".")));
        let tricky = f.root.path().canonicalize().unwrap().join("gone-soon");
        assert!(same_path(&dir, &tricky.join("..").join("gone-soon")));
        std::fs::remove_dir_all(&dir).unwrap();
        // The directory is gone — the lexical comparison still recognises the
        // stored grant, so nothing keeps reaching it.
        assert!(same_path(&dir, &dir));
        assert!(!same_path(&dir, &f.dir("elsewhere")));
    }

    #[test]
    fn normalize_skips_directories_that_vanished() {
        let f = Fixture::new();
        let keep = f.dir("keep");
        let roots = normalize(vec![
            LinkedRoot {
                path: f.root.path().join("gone"),
                access: RootAccess::Read,
            },
            LinkedRoot {
                path: keep.clone(),
                access: RootAccess::Read,
            },
        ]);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, keep);
    }

    #[test]
    fn suggest_root_finds_the_nearest_project_marker() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let proj = base.join("kernel");
        let deep = proj.join("crates/core/src");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(proj.join("Cargo.toml"), "").unwrap();
        assert_eq!(suggest_root(&deep.join("lib.rs")).unwrap(), proj);
        assert!(suggest_root(Path::new("/")).is_none());
    }
}
