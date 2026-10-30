use std::path::{Component, Path};

use agent_context::git::{self, FileDiff, GitError};

const PATCH_BYTES: usize = 64 * 1024;
const TOTAL_PATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub struct WorkspaceFile {
    pub change: FileDiff,
    pub preview_error: Option<String>,
}

#[derive(Debug)]
pub enum WorkspaceRead {
    NotRepository,
    Repository {
        branch: Option<String>,
        files: Vec<WorkspaceFile>,
        limited: bool,
    },
}

/// Blocking filesystem work; callers must run this outside the UI executor.
pub fn read_workspace(root: &Path) -> Result<WorkspaceRead, String> {
    if !cfg!(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )) {
        return Err("当前平台尚不支持安全的工作区改动扫描".into());
    }
    // Canonicalization failures are real errors, not a clean/non-repository state.
    let workspace = root.canonicalize().map_err(|error| error.to_string())?;
    let worktree = match git::worktree_root(&workspace) {
        Ok(root) => root.canonicalize().map_err(|error| error.to_string())?,
        Err(GitError::NotARepo(_)) => return Ok(WorkspaceRead::NotRepository),
        Err(error) => return Err(error.to_string()),
    };
    // Do not call git_snapshot/status here: gix can hash worktree contents
    // before a UI-side filter or path check gets a chance to reject them.
    let branch = git::worktree_branch(&workspace).map_err(|error| error.to_string())?;
    let files = git::worktree_changes_scoped(&workspace).map_err(|error| error.to_string())?;
    // The upstream cap now applies inside this workspace, before UI rebasing.
    let mut limited = files.iter().any(|file| file.truncated);
    let mut files = scope_changes(&workspace, &worktree, files)?;
    limited |= limit_patches(&mut files);
    let files = files
        .into_iter()
        .map(|change| workspace_file(&workspace, change))
        .collect();
    Ok(WorkspaceRead::Repository {
        branch,
        files,
        limited,
    })
}

fn workspace_file(workspace: &Path, mut change: FileDiff) -> WorkspaceFile {
    let preview_error = check_preview_path(workspace, Path::new(&change.path)).err();
    if preview_error.is_some() {
        change.diff.clear();
        change.adds = 0;
        change.dels = 0;
    }
    WorkspaceFile {
        change,
        preview_error,
    }
}

fn limit_patches(files: &mut [FileDiff]) -> bool {
    let mut remaining = TOTAL_PATCH_BYTES;
    let mut limited = false;
    for file in files {
        // A capped result can mark a nonempty diff truncated.
        // Never render that patch as if it were complete.
        if file.truncated || file.diff.len() > PATCH_BYTES || file.diff.len() > remaining {
            file.diff.clear();
            file.truncated = true;
            limited = true;
        } else {
            remaining -= file.diff.len();
        }
    }
    limited
}

/// Recheck whether a scoped result is still suitable for display. This is not a
/// safe-open primitive; content must already come from the scoped Git reader.
fn check_preview_path(workspace: &Path, relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("无效的预览路径".into());
    }
    let mut path = workspace.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err("无效的预览路径".into());
        };
        path.push(name);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            // Deleted files (including deleted parent directories) use HEAD.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("无法确认预览路径安全：{error}")),
        };
        if metadata.file_type().is_symlink() {
            return Err("为保护工作区，不预览符号链接或经符号链接访问的文件".into());
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err("预览路径的父级不是目录".into());
            }
        } else if !metadata.is_file() {
            return Err("只支持预览普通文件，目录或特殊文件不支持预览".into());
        }
    }
    Ok(())
}

/// Scope worktree-relative changes to a workspace, rewriting displayed paths.
/// Roots must already be absolute and canonical; this function performs no I/O.
pub fn scope_changes(
    workspace: &Path,
    worktree: &Path,
    files: Vec<FileDiff>,
) -> Result<Vec<FileDiff>, String> {
    if !workspace.is_absolute() || !worktree.is_absolute() {
        return Err("工作区与仓库路径必须为绝对路径".into());
    }
    if workspace
        .components()
        .chain(worktree.components())
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("工作区与仓库路径必须已规范化".into());
    }
    let prefix = workspace
        .strip_prefix(worktree)
        .map_err(|_| "工作区不在仓库目录内".to_owned())?;
    Ok(files
        .into_iter()
        .filter_map(|mut file| {
            let path = Path::new(&file.path);
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return None;
            }
            let relative = path.strip_prefix(prefix).ok()?;
            if relative.as_os_str().is_empty() {
                return None;
            }
            file.path = relative.to_string_lossy().into_owned();
            Some(file)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_context::git::ChangeKind;

    fn changed(path: &str) -> FileDiff {
        FileDiff {
            path: path.into(),
            kind: ChangeKind::Update,
            adds: 3,
            dels: 1,
            diff: "@@ -1 +1 @@\n-old\n+new\n".into(),
            binary: false,
            truncated: false,
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    #[test]
    fn unsupported_platform_is_rejected_before_reading_the_path() {
        let error = read_workspace(Path::new("Z:\\missing\\workspace")).unwrap_err();
        assert!(error.contains("尚不支持"));
    }

    #[test]
    fn subdirectory_workspace_excludes_siblings_and_keeps_diff_semantics() {
        let files = scope_changes(
            Path::new("/repo/crates/app"),
            Path::new("/repo"),
            vec![
                changed("crates/app/src/main.rs"),
                changed("crates/app-other/src/main.rs"),
                changed("Cargo.toml"),
            ],
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!((files[0].adds, files[0].dels), (3, 1));
        assert_eq!(files[0].kind, ChangeKind::Update);
        assert_eq!(files[0].diff, changed("").diff);
    }

    #[test]
    fn repository_root_keeps_paths_but_rejects_outside_entries() {
        let mut binary = changed("image.png");
        binary.binary = true;
        binary.truncated = true;
        binary.diff.clear();
        let files = scope_changes(
            Path::new("/repo"),
            Path::new("/repo"),
            vec![binary, changed("../outside"), changed("/outside")],
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "image.png");
        assert!(files[0].binary);
        assert!(files[0].truncated);
    }

    #[test]
    fn invalid_scope_is_an_error_not_a_clean_workspace() {
        assert!(scope_changes(Path::new("/outside"), Path::new("/repo"), vec![]).is_err());
        assert!(scope_changes(Path::new("repo"), Path::new("/repo"), vec![]).is_err());
        assert!(scope_changes(Path::new("/repo/../outside"), Path::new("/repo"), vec![]).is_err());
        assert!(
            scope_changes(Path::new("/repo"), Path::new("/repo"), vec![])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn truncated_marker_drops_even_a_nonempty_upstream_patch() {
        let mut file = changed("last.rs");
        file.truncated = true;
        let mut files = vec![file];
        assert!(limit_patches(&mut files));
        assert!(files[0].diff.is_empty());
        assert!(files[0].truncated);
    }

    #[test]
    fn patch_limits_accept_exact_budgets_and_drop_overflow_as_a_whole() {
        let mut exact = changed("exact.rs");
        exact.diff = "x".repeat(PATCH_BYTES);
        let mut oversized = changed("large.rs");
        oversized.diff = "x".repeat(PATCH_BYTES + 1);
        let mut files = vec![oversized, exact];
        assert!(limit_patches(&mut files));
        assert!(files[0].diff.is_empty());
        assert!(files[0].truncated);
        assert_eq!(files[1].diff.len(), PATCH_BYTES);
        assert!(!files[1].truncated);

        let mut files: Vec<_> = (0..TOTAL_PATCH_BYTES / PATCH_BYTES)
            .map(|index| {
                let mut file = changed(&format!("{index}.rs"));
                file.diff = "x".repeat(PATCH_BYTES);
                file
            })
            .collect();
        assert!(!limit_patches(&mut files));
        files.push(changed("overflow.rs"));
        assert!(limit_patches(&mut files));
        let last = files.last().unwrap();
        assert!(last.diff.is_empty());
        assert!(last.truncated);
    }

    #[test]
    fn preview_paths_allow_regular_and_deleted_files_but_not_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/file.rs"), "text").unwrap();
        assert!(check_preview_path(dir.path(), Path::new("src/file.rs")).is_ok());
        assert!(check_preview_path(dir.path(), Path::new("src/deleted.rs")).is_ok());
        assert!(check_preview_path(dir.path(), Path::new("deleted/file.rs")).is_ok());
        assert!(check_preview_path(dir.path(), Path::new("src")).is_err());
        assert!(check_preview_path(dir.path(), Path::new("../outside")).is_err());
        assert!(check_preview_path(dir.path(), Path::new("missing/../outside")).is_err());
        assert!(check_preview_path(dir.path(), Path::new("/outside")).is_err());
        assert!(check_preview_path(dir.path(), Path::new("")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_and_special_files_have_no_preview_or_derived_line_counts() {
        use std::os::unix::{fs::symlink, net::UnixListener};

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "private").unwrap();
        symlink(
            outside.path().join("secret"),
            workspace.path().join("linked"),
        )
        .unwrap();
        symlink(outside.path(), workspace.path().join("linked-dir")).unwrap();
        symlink(
            outside.path().join("missing"),
            workspace.path().join("dangling"),
        )
        .unwrap();
        let _socket = UnixListener::bind(workspace.path().join("socket")).unwrap();

        for path in ["linked", "linked-dir/secret", "dangling", "socket"] {
            let entry = workspace_file(workspace.path(), changed(path));
            assert!(entry.preview_error.is_some(), "{path}");
            assert!(entry.change.diff.is_empty(), "{path}");
            assert_eq!((entry.change.adds, entry.change.dels), (0, 0), "{path}");
        }
    }

    #[test]
    fn missing_root_is_an_error_and_non_repository_is_distinct() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_workspace(&dir.path().join("missing")).is_err());
        assert!(matches!(
            read_workspace(dir.path()).unwrap(),
            WorkspaceRead::NotRepository
        ));
    }
}
