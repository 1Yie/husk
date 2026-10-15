//! Office-mode workspace paths and the legacy session namespace.

use std::path::{Path, PathBuf};

/// Initialize the session repository with Husk's stable office namespace.
/// Ordinary workspace roots keep the repository's canonical-path hash.
pub fn init_store(data_dir: PathBuf) {
    agent_store::init_data_dir_with_namespaces(data_dir, &[("office", "office")]);
}

/// The office-mode pseudo-workspace root: `<data dir>/office`.
/// Every office session shares this directory as its workspace root, so a
/// document one session produced is readable/writable by all the others
/// (the `resolve` sandbox only confines paths to the workspace root).
/// Created lazily — `SessionManager::switch_workspace` makes it on the
/// first switch.
pub fn office_root() -> Option<PathBuf> {
    agent_store::app_data_dir().map(|d| d.join("office"))
}

/// Whether `root` is the office pseudo-workspace. Compared both raw and
/// canonicalized — the dir may not exist yet on a first switch, which
/// makes `canonicalize` fail on one or both sides.
pub fn is_office_root(root: &Path) -> bool {
    let Some(office) = office_root() else {
        return false;
    };
    if root == office {
        return true;
    }
    let r = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let o = office.canonicalize().unwrap_or(office);
    r == o
}
