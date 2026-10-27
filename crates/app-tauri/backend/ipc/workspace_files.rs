//! Read-only, bounded workspace browsing. No recursive scans or document execution.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};
use tauri::{Manager, WebviewWindow};

const MAX_ENTRIES: usize = 2_000;
const TEXT_LIMIT: u64 = 2 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct WorkspaceEntry {
    name: String,
    path: String,
    kind: &'static str,
    size: u64,
}

#[derive(Debug, Serialize)]
pub struct DirectoryListing {
    path: String,
    entries: Vec<WorkspaceEntry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct FilePreview {
    path: String,
    name: String,
    size: u64,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
}

fn local_application(window: &WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|e| e.to_string())?;
    let bundled = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"));
    let mut local =
        bundled && url.port().is_none() && url.username().is_empty() && url.password().is_none();
    #[cfg(debug_assertions)]
    if let Some(dev_url) = &window.app_handle().config().build.dev_url {
        // Unlike a configurable remote dev server, only loopback development is local.
        let loopback = matches!(
            dev_url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        local |= loopback
            && matches!(dev_url.scheme(), "http" | "https")
            && url.origin() == dev_url.origin()
            && url.username().is_empty()
            && url.password().is_none();
    }
    if !local {
        return Err("workspace files IPC requires a local application webview".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn workspace_list_files(
    window: WebviewWindow,
    root: String,
    path: String,
) -> Result<DirectoryListing, String> {
    local_application(&window)?;
    tauri::async_runtime::spawn_blocking(move || list_files(Path::new(&root), &path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn workspace_preview_file(
    window: WebviewWindow,
    root: String,
    path: String,
) -> Result<FilePreview, String> {
    local_application(&window)?;
    tauri::async_runtime::spawn_blocking(move || preview_file(Path::new(&root), &path))
        .await
        .map_err(|e| e.to_string())?
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

/// Return both the canonical target and a normalized, root-relative display path.
fn resolve(root: &Path, relative: &str) -> Result<(PathBuf, String), String> {
    if !root.is_absolute() {
        return Err("workspace root must be absolute".into());
    }
    let root = root.canonicalize().map_err(io_error)?;
    if !root.is_dir() {
        return Err("workspace root must be a directory".into());
    }
    let mut clean = PathBuf::new();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(name) => clean.push(name),
            Component::CurDir => {}
            _ => return Err("file path must be relative and cannot contain '..'".into()),
        }
    }
    let target = root.join(&clean).canonicalize().map_err(io_error)?;
    if !target.starts_with(&root) {
        return Err("file path escapes the workspace".into());
    }
    let path = clean
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok((target, path))
}

fn list_files(root: &Path, relative: &str) -> Result<DirectoryListing, String> {
    let (target, path) = resolve(root, relative)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    // Bound enumeration as well as the payload; a large node_modules stays responsive.
    for (scanned, item) in fs::read_dir(target).map_err(io_error)?.enumerate() {
        if scanned == MAX_ENTRIES {
            truncated = true;
            break;
        }
        let item = item.map_err(io_error)?;
        // Never follow a link just to classify or size it.
        let metadata = fs::symlink_metadata(item.path()).map_err(io_error)?;
        let kind = if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            continue; // Sockets, FIFOs and devices cannot be previewed.
        };
        // Non-UTF8 names cannot be addressed losslessly over this string IPC.
        let Some(name) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let child = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        entries.push(WorkspaceEntry {
            name,
            path: child,
            kind,
            size: metadata.len(),
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != "directory")
            .cmp(&(b.kind != "directory"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(DirectoryListing {
        path,
        entries,
        truncated,
    })
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn binary(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .any(|b| *b == 0 || (*b < 0x20 && !matches!(*b, b'\n' | b'\r' | b'\t' | 0x0c)))
}

fn preview_file(root: &Path, relative: &str) -> Result<FilePreview, String> {
    let (target, path) = resolve(root, relative)?;
    if !fs::metadata(&target).map_err(io_error)?.is_file() {
        return Err("only regular files can be previewed".into());
    }
    let file = File::open(&target).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err("only regular files can be previewed".into());
    }
    let mut result = FilePreview {
        name: Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path,
        size: metadata.len(),
        kind: "binary",
        text: None,
        mime: None,
        data: None,
    };
    let mut reader = file.take(IMAGE_LIMIT + 1);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(8192)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if let Some(mime) = image_mime(&bytes) {
        result.mime = Some(mime);
        if result.size > IMAGE_LIMIT {
            result.kind = "too_large";
            return Ok(result);
        }
        reader.read_to_end(&mut bytes).map_err(io_error)?;
        if bytes.len() as u64 > IMAGE_LIMIT {
            result.kind = "too_large";
        } else {
            result.kind = "image";
            result.data = Some(STANDARD.encode(bytes));
        }
        return Ok(result);
    }
    if binary(&bytes) {
        return Ok(result);
    }
    if result.size > TEXT_LIMIT {
        result.kind = "too_large";
        return Ok(result);
    }
    // A file growing during the read cannot bypass the text size bound.
    reader
        .take(TEXT_LIMIT + 1 - bytes.len() as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > TEXT_LIMIT {
        result.kind = "too_large";
    } else if !binary(&bytes) {
        if let Ok(text) = String::from_utf8(bytes) {
            result.kind = "text";
            result.text = Some(text);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_shallow_listing_and_lazy_subdirectory() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("z.txt"), "z").unwrap();
        fs::write(temp.path().join("A.txt"), "a").unwrap();
        fs::write(temp.path().join("src/lib.rs"), "fn main() {}").unwrap();
        let listing = list_files(temp.path(), "").unwrap();
        assert_eq!(
            listing
                .entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["src", "A.txt", "z.txt"]
        );
        assert!(!listing.truncated);
        let nested = list_files(temp.path(), "src").unwrap();
        assert_eq!(nested.entries[0].path, "src/lib.rs");
        assert_eq!(
            preview_file(temp.path(), "src/lib.rs")
                .unwrap()
                .text
                .as_deref(),
            Some("fn main() {}")
        );
    }

    #[test]
    fn utf8_and_inert_documents() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["hello.txt", "page.html", "icon.svg", "readme.md"] {
            fs::write(temp.path().join(name), "<script>你好 👋</script>").unwrap();
            let file = preview_file(temp.path(), name).unwrap();
            assert_eq!(file.kind, "text");
            assert_eq!(file.text.as_deref(), Some("<script>你好 👋</script>"));
            assert!(file.data.is_none());
        }
        fs::write(temp.path().join("empty"), "").unwrap();
        assert_eq!(
            preview_file(temp.path(), "empty").unwrap().text.as_deref(),
            Some("")
        );
    }

    #[test]
    fn image_signatures_binary_and_size_limits() {
        let temp = tempfile::tempdir().unwrap();
        for (index, (signature, mime)) in [
            (&b"\x89PNG\r\n\x1a\n"[..], "image/png"),
            (&b"\xff\xd8\xff"[..], "image/jpeg"),
            (&b"GIF89a"[..], "image/gif"),
            (&b"RIFF1234WEBP"[..], "image/webp"),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("{index}.dat");
            fs::write(temp.path().join(&name), signature).unwrap();
            let file = preview_file(temp.path(), &name).unwrap();
            assert_eq!(file.kind, "image");
            assert_eq!(file.mime, Some(mime));
            assert_eq!(STANDARD.decode(file.data.unwrap()).unwrap(), signature);
        }
        for (name, content) in [("binary", &b"a\0b"[..]), ("invalid", &b"\xff\xfe"[..])] {
            fs::write(temp.path().join(name), content).unwrap();
            assert_eq!(preview_file(temp.path(), name).unwrap().kind, "binary");
        }
        let text = File::create(temp.path().join("large.txt")).unwrap();
        text.set_len(TEXT_LIMIT + 1).unwrap();
        // Sparse NUL-filled data is binary, even when large.
        assert_eq!(
            preview_file(temp.path(), "large.txt").unwrap().kind,
            "binary"
        );
        fs::write(
            temp.path().join("large.txt"),
            vec![b'a'; TEXT_LIMIT as usize + 1],
        )
        .unwrap();
        assert_eq!(
            preview_file(temp.path(), "large.txt").unwrap().kind,
            "too_large"
        );
        fs::write(temp.path().join("large.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        File::options()
            .write(true)
            .open(temp.path().join("large.png"))
            .unwrap()
            .set_len(IMAGE_LIMIT + 1)
            .unwrap();
        let image = preview_file(temp.path(), "large.png").unwrap();
        assert_eq!(image.kind, "too_large");
        assert!(image.data.is_none());
    }

    #[test]
    fn rejects_traversal_absolute_and_invalid_roots() {
        let temp = tempfile::tempdir().unwrap();
        assert!(list_files(temp.path(), "../").is_err());
        assert!(list_files(temp.path(), "src/../").is_err());
        assert!(list_files(temp.path(), temp.path().to_str().unwrap()).is_err());
        assert!(list_files(Path::new("relative-root"), "").is_err());
        fs::write(temp.path().join("file"), "").unwrap();
        assert!(list_files(&temp.path().join("file"), "").is_err());
        assert!(preview_file(temp.path(), "").is_err());
    }

    #[test]
    fn truncated_is_explicit() {
        let temp = tempfile::tempdir().unwrap();
        for n in 0..=MAX_ENTRIES {
            fs::write(temp.path().join(n.to_string()), "").unwrap();
        }
        let listing = list_files(temp.path(), "").unwrap();
        assert_eq!(listing.entries.len(), MAX_ENTRIES);
        assert!(listing.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_escape_root() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private"), "secret").unwrap();
        symlink(outside.path(), temp.path().join("outside")).unwrap();
        symlink(
            outside.path().join("private"),
            temp.path().join("file-link"),
        )
        .unwrap();
        assert!(list_files(temp.path(), "outside").is_err());
        assert!(preview_file(temp.path(), "file-link").is_err());
        assert!(list_files(temp.path(), "")
            .unwrap()
            .entries
            .iter()
            .all(|e| e.kind == "symlink"));
        fs::write(temp.path().join("local"), "safe").unwrap();
        symlink(temp.path().join("local"), temp.path().join("inside")).unwrap();
        assert_eq!(
            preview_file(temp.path(), "inside").unwrap().text.as_deref(),
            Some("safe")
        );
    }
}
