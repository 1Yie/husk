use std::path::Path;

const MAX_ENTRIES: usize = 2_000;
const TEXT_LIMIT: u64 = 2 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub modified: u64,
}

#[derive(Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum RasterFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
}

#[derive(Clone, Debug)]
pub enum Preview {
    Text {
        text: String,
        size: u64,
    },
    Image {
        bytes: std::sync::Arc<Vec<u8>>,
        format: RasterFormat,
        size: u64,
    },
    Binary {
        size: u64,
    },
    TooLarge {
        size: u64,
    },
}

/// All entry points are blocking; invoke only on the background executor.
pub fn list_directory(root: &Path, relative: &str) -> Result<Listing, String> {
    safe::list(root, relative).map_err(|e| e.to_string())
}

pub fn preview_file(root: &Path, relative: &str) -> Result<Preview, String> {
    safe::preview(root, relative).map_err(|e| e.to_string())
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod safe {
    use super::*;
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
    use std::{
        fs::File,
        io::{self, Read},
        path::{Component, PathBuf},
    };

    fn invalid() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, "无效的工作区相对路径")
    }

    fn child(parent: &File, name: &Path, directory: bool) -> io::Result<File> {
        if name.components().count() != 1
            || !matches!(name.components().next(), Some(Component::Normal(_)))
        {
            return Err(invalid());
        }
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        if directory {
            flags |= OFlags::DIRECTORY;
        }
        openat(parent, name, flags, Mode::empty())
            .map(File::from)
            .map_err(io::Error::from)
    }

    fn directory(mut parent: File, path: &Path) -> io::Result<File> {
        for part in path.components() {
            let Component::Normal(name) = part else {
                return Err(invalid());
            };
            parent = child(&parent, Path::new(name), true)?;
        }
        Ok(parent)
    }

    fn root(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        // Pin every ancestor, including the workspace root. No canonicalize +
        // unchecked open, no path-based traversal after checking metadata.
        let mut file = File::open("/")?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => file = child(&file, Path::new(name), true)?,
                _ => return Err(invalid()),
            }
        }
        Ok(file)
    }

    pub(super) fn list(root_path: &Path, relative: &str) -> io::Result<Listing> {
        let dir = directory(root(root_path)?, Path::new(relative))?;
        let mut listing = Listing {
            entries: Vec::new(),
            truncated: false,
        };
        let mut scanned = 0;
        for entry in Dir::read_from(&dir).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if scanned == MAX_ENTRIES {
                listing.truncated = true;
                break;
            }
            scanned += 1;
            let Ok(name) = std::str::from_utf8(bytes) else {
                continue;
            };
            // fstatat relative to the pinned fd never follows a link. Unknown
            // d_type filesystems receive the same safe classification.
            let stat = statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
            let kind = match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => EntryKind::Directory,
                FileType::RegularFile => EntryKind::File,
                FileType::Symlink => EntryKind::Symlink,
                _ => continue,
            };
            let path = if relative.is_empty() {
                name.into()
            } else {
                format!("{relative}/{name}")
            };
            listing.entries.push(Entry {
                name: name.into(),
                path,
                kind,
                modified: if kind == EntryKind::Symlink {
                    0
                } else {
                    child(&dir, Path::new(name), kind == EntryKind::Directory)
                        .ok()
                        .and_then(|file| file.metadata().ok())
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |time| time.as_secs())
                },
            });
        }
        listing.entries.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(listing)
    }

    pub(super) fn preview(root_path: &Path, relative: &str) -> io::Result<Preview> {
        let path = PathBuf::from(relative);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(invalid());
        }
        let name = path.file_name().ok_or_else(invalid)?;
        let parent = directory(root(root_path)?, path.parent().unwrap_or(Path::new("")))?;
        let file = child(&parent, Path::new(name), false)?;
        // fstat the actual open descriptor, not the previously listed path.
        // NONBLOCK above means a swapped-in FIFO can never hang the executor.
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other(
                "只支持预览普通文件；不读取链接、目录或特殊文件",
            ));
        }
        let size = metadata.len();
        let mut bytes = Vec::new();
        let mut reader = file.take(IMAGE_LIMIT + 1);
        reader.by_ref().take(8192).read_to_end(&mut bytes)?;
        let format = raster_format(&bytes);
        let limit = if format.is_some() {
            IMAGE_LIMIT
        } else {
            TEXT_LIMIT
        };
        if size > limit {
            return Ok(Preview::TooLarge { size });
        }
        reader
            .by_ref()
            .take(limit + 1 - bytes.len() as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Ok(Preview::TooLarge {
                size: bytes.len() as u64,
            });
        }
        if let Some(format) = format {
            return Ok(Preview::Image {
                bytes: std::sync::Arc::new(bytes),
                format,
                size,
            });
        }
        if bytes
            .iter()
            .any(|b| *b == 0 || (*b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 12)))
        {
            return Ok(Preview::Binary { size });
        }
        match String::from_utf8(bytes) {
            Ok(text) => Ok(Preview::Text { text, size }),
            Err(_) => Ok(Preview::Binary { size }),
        }
    }
}

fn raster_format(bytes: &[u8]) -> Option<RasterFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(RasterFormat::Png)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(RasterFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(RasterFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(RasterFormat::Webp)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
mod safe {
    use super::*;
    use std::io;
    pub(super) fn list(_: &Path, _: &str) -> io::Result<Listing> {
        Err(io::Error::other("当前平台尚不支持安全的工作区文件浏览"))
    }
    pub(super) fn preview(_: &Path, _: &str) -> io::Result<Preview> {
        Err(io::Error::other("当前平台尚不支持安全的工作区文件预览"))
    }
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "android", target_os = "macos")
))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn directory_sorting_and_literal_text_are_read_only_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("z-dir")).unwrap();
        fs::write(root.path().join("a.html"), "<script>not executed</script>").unwrap();
        fs::write(root.path().join("binary"), [0, 1, 2]).unwrap();
        let listing = list_directory(root.path(), "").unwrap();
        assert_eq!(listing.entries[0].kind, EntryKind::Directory);
        assert!(
            matches!(preview_file(root.path(), "a.html").unwrap(), Preview::Text {text, ..} if text.contains("<script>"))
        );
        assert!(matches!(
            preview_file(root.path(), "binary").unwrap(),
            Preview::Binary { .. }
        ));
        let file = fs::File::create(root.path().join("large")).unwrap();
        file.set_len(TEXT_LIMIT + 1).unwrap();
        assert!(matches!(
            preview_file(root.path(), "large").unwrap(),
            Preview::TooLarge { .. }
        ));
    }

    #[test]
    fn images_are_recognized_by_signature_not_filename_and_have_a_separate_budget() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("not-really.png"), "plain text").unwrap();
        assert!(matches!(
            preview_file(root.path(), "not-really.png").unwrap(),
            Preview::Text { .. }
        ));
        fs::write(root.path().join("picture.bin"), b"\x89PNG\r\n\x1a\n").unwrap();
        assert!(matches!(
            preview_file(root.path(), "picture.bin").unwrap(),
            Preview::Image {
                format: RasterFormat::Png,
                ..
            }
        ));
        let mut file = fs::File::create(root.path().join("huge")).unwrap();
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        file.set_len(IMAGE_LIMIT + 1).unwrap();
        assert!(matches!(
            preview_file(root.path(), "huge").unwrap(),
            Preview::TooLarge { .. }
        ));
    }

    #[test]
    fn links_traversal_and_replaced_directories_never_escape_scope() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(list_directory(root.path(), "link").is_err());
        assert!(preview_file(root.path(), "link/secret").is_err());
        assert!(preview_file(root.path(), "../secret").is_err());
        assert!(preview_file(root.path(), "/etc/passwd").is_err());
        assert!(list_directory(root.path(), "../").is_err());
        symlink(outside.path().join("secret"), root.path().join("file")).unwrap();
        assert!(preview_file(root.path(), "file").is_err());
        let listing = list_directory(root.path(), "").unwrap();
        assert!(listing.entries.iter().all(|e| e.kind == EntryKind::Symlink));
    }

    #[test]
    fn fifo_is_not_enumerated_or_read_and_root_links_are_rejected() {
        use rustix::fs::{CWD, FileType, Mode, mknodat};
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        mknodat(CWD, &fifo, FileType::Fifo, Mode::RUSR | Mode::WUSR, 0).unwrap();
        assert!(list_directory(root.path(), "").unwrap().entries.is_empty());
        assert!(preview_file(root.path(), "fifo").is_err());
        let link = root.path().join("root-link");
        symlink(root.path(), &link).unwrap();
        assert!(list_directory(&link, "").is_err());
    }
}
