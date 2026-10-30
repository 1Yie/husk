//! Composer attachments and the existing queued-payload string protocol.
//!
//! No UI/backend dependencies. External files are streamed into the workspace;
//! only a bounded prefix of a non-image is decoded. Nothing writes to a picked
//! file. Binary detection covers that prefix, not the unbounded remainder.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(any(
    test,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
use std::fs::OpenOptions;

const MAX_TEXT_BYTES: usize = 32 * 1024;
const BINARY_PREFIX: &str = "[attached binary: ";
const BINARY_SUFFIX: &str = " — binary file, not inlined]";
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Attachment {
    /// Original selection: use this, not `path`, to deduplicate pending chips.
    pub(super) source_path: PathBuf,
    /// Original workspace path, or the durable workspace-local staged copy.
    pub(super) path: PathBuf,
    pub(super) name: String,
    pub(super) is_image: bool,
    pub(super) truncated: bool,
    // None means an image or a binary reference; never inline image bytes.
    content: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AttachmentDisplay {
    pub(super) path: PathBuf,
    pub(super) name: String,
    pub(super) is_image: bool,
    pub(super) is_binary: bool,
    pub(super) truncated: bool,
}

/// Editing prompt text must not re-read attachments or reconstruct their
/// markers from display labels (which would discard inline/truncated content).
pub(super) struct EditablePayload {
    pub(super) text: String,
    pub(super) attachments: Vec<AttachmentDisplay>,
    markers: Vec<String>,
    original: String,
}

impl EditablePayload {
    pub(super) fn parse(payload: &str) -> Self {
        let mut markers = Vec::new();
        let mut attachments = Vec::new();
        let text = split_payload(payload, |marker, attachment| {
            markers.push(marker.to_owned());
            attachments.push(attachment);
        });
        Self {
            text,
            attachments,
            markers,
            original: payload.to_owned(),
        }
    }

    pub(super) fn with_text(&self, text: &str) -> String {
        if text == self.text {
            return self.original.clone();
        }
        let mut payload = text.to_owned();
        for marker in &self.markers {
            payload.push_str("\n\n");
            payload.push_str(marker);
        }
        payload
    }
}

impl Attachment {
    /// Load a picker-selected regular file. No canonicalization is applied to
    /// `source_path`; canonical paths are used only for containment checks.
    ///
    /// Paths the unescaped legacy protocol cannot represent are rejected.
    /// Invalid UTF-8/NUL in the inspected prefix produces a binary reference.
    /// A valid UTF-8 character crossing the 32 KiB cap is omitted, not replaced.
    pub(super) fn load(path: PathBuf, workspace_root: &Path) -> Result<Self, String> {
        protocol_path(&path)?;
        protocol_path(workspace_root)?;
        let workspace = fs::canonicalize(workspace_root)
            .map_err(|e| format!("resolve workspace {}: {e}", workspace_root.display()))?;
        protocol_path(&workspace)?;
        if !workspace.is_dir() {
            return Err("attachment workspace is not a directory".into());
        }
        let resolved = fs::canonicalize(&path)
            .map_err(|e| format!("resolve attachment {}: {e}", path.display()))?;
        protocol_path(&resolved)?;
        let mut file = open_selected_file(&resolved)
            .map_err(|e| format!("open attachment {}: {e}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("attachment needs a UTF-8 file name")?
            .to_owned();
        let is_image = is_image_path(&path);
        let (content, truncated) = if is_image {
            (None, false)
        } else {
            read_prefix(&mut file)
                .map_err(|e| format!("read attachment {}: {e}", path.display()))?
        };
        // Checking the canonical target alone is insufficient: an outside
        // symlink pointing inwards would still send an unreachable outside path.
        let in_workspace = reachable_inside(&path, workspace_root, &workspace, &resolved)?;
        let send_path = if in_workspace {
            // A picker normally returns absolute paths, but relative paths
            // would otherwise be resolved against the agent's workspace a
            // second time when the prompt is consumed.
            if path.is_absolute() && path.starts_with(&workspace) {
                path.clone()
            } else {
                resolved.clone()
            }
        } else {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(0))
                .map_err(|e| format!("rewind attachment {}: {e}", path.display()))?;
            stage_file(&mut file, &name, &workspace)?
        };
        Ok(Self {
            source_path: path,
            path: send_path,
            name,
            is_image,
            truncated,
            content,
        })
    }
}

/// Exactly the current queued-payload.ts layout: paths are raw (not XML
/// entities), image names lose double quotes, binaries use the bracket marker.
pub(super) fn build_payload(text: &str, attachments: &[Attachment]) -> String {
    let mut output = text.trim_matches(js_whitespace).to_owned();
    for attachment in attachments {
        // `load` establishes UTF-8/protocol validity; don't silently use lossy
        // path conversion. Public fields must retain that invariant if edited.
        let path = attachment.path.to_str().expect("validated attachment path");
        if attachment.is_image {
            output.push_str("\n\n<attached-image path=\"");
            output.push_str(path);
            output.push_str("\" name=\"");
            output.push_str(&attachment.name.replace('"', ""));
            output.push_str("\"/>");
        } else if let Some(content) = &attachment.content {
            output.push_str("\n\n<attached-file path=\"");
            output.push_str(path);
            output.push_str("\">\n```\n");
            output.push_str(content);
            if attachment.truncated {
                output.push_str("\n… (truncated)");
            }
            output.push_str("\n```");
        } else {
            output.push_str("\n\n");
            output.push_str(BINARY_PREFIX);
            output.push_str(path);
            output.push_str(BINARY_SUFFIX);
        }
    }
    output
}

/// Strip recognizable attachment blocks, retaining prose both before and after
/// them. This never reads paths found in a message. Attributes remain raw:
/// decoding `&quot;` would corrupt legitimate filenames in the current protocol.
/// An opened, fenced text attachment with a missing closing fence is hidden to
/// EOF, rather than exposing its potentially large inline contents.
pub(super) fn display_payload(text: &str) -> (String, Vec<AttachmentDisplay>) {
    let mut attachments = Vec::new();
    let body = split_payload(text, |_, attachment| attachments.push(attachment));
    (body, attachments)
}

fn split_payload(text: &str, mut found: impl FnMut(&str, AttachmentDisplay)) -> String {
    let mut body = String::new();
    let mut copied = 0;
    let mut cursor = 0;
    while cursor < text.len() {
        let (separator, start) = if cursor == 0 && marker_start(text) {
            (0, 0)
        } else if let Some(offset) = text[cursor..].find("\n\n") {
            (cursor + offset, cursor + offset + 2)
        } else {
            break;
        };
        if let Some((consumed, attachment)) = parse_marker(&text[start..]) {
            body.push_str(&text[copied..separator]);
            found(&text[start..start + consumed], attachment);
            copied = start + consumed;
            cursor = copied;
        } else {
            cursor = start.max(cursor + 1);
        }
    }
    body.push_str(&text[copied..]);
    body
}

fn protocol_path(path: &Path) -> Result<&str, String> {
    let value = path
        .to_str()
        .ok_or("attachment paths must be lossless UTF-8")?;
    if value.is_empty()
        || value.chars().any(|c| c == '"' || c.is_control())
        || value.contains(BINARY_SUFFIX)
    {
        return Err(format!(
            "attachment path cannot be represented by the existing marker protocol: {path:?}"
        ));
    }
    Ok(value)
}

fn is_image_path(path: &Path) -> bool {
    agent_llm::types::ImageRef::for_path(path.to_path_buf()).is_some()
}

fn read_prefix(file: &mut File) -> io::Result<(Option<String>, bool)> {
    let mut bytes = Vec::with_capacity(MAX_TEXT_BYTES + 1);
    file.take((MAX_TEXT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    bytes.truncate(MAX_TEXT_BYTES);
    if bytes.contains(&0) {
        return Ok((None, false));
    }
    let content = match std::str::from_utf8(&bytes) {
        Ok(text) => text.to_owned(),
        Err(error) if truncated && error.error_len().is_none() => {
            // Only a partial codepoint at the cap: all preceding bytes have
            // already been validated. Never use from_utf8_lossy on that edge.
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .expect("validated UTF-8 prefix")
                .to_owned()
        }
        Err(_) => return Ok((None, false)),
    };
    Ok((Some(content), truncated))
}

fn reachable_inside(
    source: &Path,
    workspace_root: &Path,
    workspace: &Path,
    resolved: &Path,
) -> Result<bool, String> {
    if !resolved.starts_with(workspace) {
        return Ok(false);
    }
    let cwd = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
    let absolute = if source.is_absolute() {
        source.to_path_buf()
    } else {
        cwd.join(source)
    };
    let root = if workspace_root.is_absolute() {
        workspace_root.to_path_buf()
    } else {
        cwd.join(workspace_root)
    };
    let base = if absolute.starts_with(&root) {
        &root
    } else {
        workspace
    };
    let Ok(relative) = absolute.strip_prefix(base) else {
        return Ok(false);
    };
    // Do not let `..` or an intermediate symlink leave the mounted workspace
    // and then re-enter it through another symlink.
    let mut current = base.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Ok(false);
        }
        current.push(component);
        let target = fs::canonicalize(&current)
            .map_err(|e| format!("resolve attachment ancestor {}: {e}", current.display()))?;
        if !target.starts_with(workspace) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn ensure_directory(path: &Path, workspace: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!(
                "create attachment directory {}: {e}",
                path.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "attachment directory must not be a symlink or file: {}",
            path.display()
        ));
    }
    let resolved = fs::canonicalize(path).map_err(|e| e.to_string())?;
    protocol_path(&resolved)?;
    if !resolved.starts_with(workspace) {
        return Err("attachment staging directory escapes workspace".into());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn stage_file(source: &mut File, name: &str, workspace: &Path) -> Result<PathBuf, String> {
    let husk = workspace.join(".husk");
    ensure_directory(&husk, workspace)?;
    let directory = husk.join("attachments");
    ensure_directory(&directory, workspace)?;
    // create_new never truncates an existing ignore file, including a symlink.
    let ignore_path = directory.join(".gitignore");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ignore_path)
    {
        Ok(mut ignore) => {
            if let Err(error) = ignore.write_all(b"*\n") {
                let _ = fs::remove_file(&ignore_path);
                return Err(format!("write attachment gitignore: {error}"));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("create attachment gitignore: {error}")),
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..64 {
        let id = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let unique = directory.join(format!("picked-{}-{stamp:x}-{id:x}", std::process::id()));
        match fs::create_dir(&unique) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create attachment staging slot: {error}")),
            Ok(()) => {}
        }
        let staged = unique.join(name);
        let result = (|| {
            ensure_directory(&unique, workspace)?;
            protocol_path(&staged)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|e| format!("create staged attachment {}: {e}", staged.display()))?;
            io::copy(source, &mut output)
                .map_err(|e| format!("copy staged attachment {}: {e}", staged.display()))?;
            output
                .sync_all()
                .map_err(|e| format!("flush staged attachment: {e}"))?;
            let resolved =
                fs::canonicalize(&staged).map_err(|e| format!("resolve staged attachment: {e}"))?;
            protocol_path(&resolved)?;
            if !resolved.starts_with(workspace) {
                return Err("staged attachment escapes workspace".into());
            }
            Ok(staged.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staged);
            let _ = fs::remove_dir(&unique);
        }
        return result;
    }
    Err("could not allocate a unique attachment staging slot".into())
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn open_selected_file(path: &Path) -> io::Result<File> {
    scoped_files::open_regular(path)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn open_selected_file(path: &Path) -> io::Result<File> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn stage_file(source: &mut File, name: &str, workspace: &Path) -> Result<PathBuf, String> {
    scoped_files::stage(source, name, workspace).map_err(|error| error.to_string())
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod scoped_files {
    use super::*;
    use rustix::fs::{AtFlags, Mode, OFlags, mkdirat, openat, unlinkat};
    use std::path::Component;

    fn single_component(path: &Path) -> io::Result<()> {
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid attachment path component",
            ));
        }
        Ok(())
    }

    fn open_child(parent: &File, name: &Path, directory: bool) -> io::Result<File> {
        single_component(name)?;
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW;
        if directory {
            flags |= OFlags::DIRECTORY;
        }
        openat(parent, name, flags, Mode::empty())
            .map(File::from)
            .map_err(io::Error::from)
    }

    fn open_directory(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "attachment directory must be absolute",
            ));
        }
        let mut directory = File::open("/")?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = open_child(&directory, Path::new(name), true)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid attachment directory",
                    ));
                }
            }
        }
        Ok(directory)
    }

    fn ensure_child(parent: &File, name: &str) -> io::Result<File> {
        match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        open_child(parent, Path::new(name), true)
    }

    fn create_file(parent: &File, name: &Path) -> io::Result<File> {
        single_component(name)?;
        openat(
            parent,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map(File::from)
        .map_err(io::Error::from)
    }

    pub(super) fn open_regular(path: &Path) -> io::Result<File> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "attachment has no parent")
        })?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "attachment has no name"))?;
        let directory = open_directory(parent)?;
        // NONBLOCK prevents a FIFO swapped in after the picker from hanging
        // the loader; inspect the opened object, not earlier path metadata.
        let file = open_child(&directory, Path::new(name), false)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "attachment is not a regular file",
            ));
        }
        Ok(file)
    }

    pub(super) fn stage(source: &mut File, name: &str, workspace: &Path) -> io::Result<PathBuf> {
        single_component(Path::new(name))?;
        let root = open_directory(workspace)?;
        let husk = ensure_child(&root, ".husk")?;
        let attachments = ensure_child(&husk, "attachments")?;
        match create_file(&attachments, Path::new(".gitignore")) {
            Ok(mut ignore) => ignore.write_all(b"*\n")?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..64 {
            let id = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let slot = format!("picked-{}-{stamp:x}-{id:x}", std::process::id());
            match mkdirat(&attachments, slot.as_str(), Mode::from_raw_mode(0o700)) {
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
                Ok(()) => {}
            }
            let dir = match open_child(&attachments, Path::new(&slot), true) {
                Ok(dir) => dir,
                Err(error) => {
                    let _ = unlinkat(&attachments, slot.as_str(), AtFlags::REMOVEDIR);
                    return Err(error);
                }
            };
            let staged = workspace.join(".husk/attachments").join(&slot).join(name);
            let result = (|| {
                protocol_path(&staged).map_err(io::Error::other)?;
                let mut output = create_file(&dir, Path::new(name))?;
                io::copy(source, &mut output)?;
                output.sync_all()?;
                let resolved = fs::canonicalize(&staged)?;
                protocol_path(&resolved).map_err(io::Error::other)?;
                if !resolved.starts_with(workspace) {
                    return Err(io::Error::other("staged attachment escapes workspace"));
                }
                Ok(staged)
            })();
            if result.is_err() {
                let _ = unlinkat(&dir, name, AtFlags::empty());
                let _ = unlinkat(&attachments, slot.as_str(), AtFlags::REMOVEDIR);
            }
            return result;
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique attachment staging slot",
        ))
    }
}

fn js_whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn marker_start(text: &str) -> bool {
    text.starts_with("<attached-file ")
        || text.starts_with("<attached-image ")
        || text.starts_with(BINARY_PREFIX)
}

fn display_ref(path: &str, name: Option<&str>, image: bool, binary: bool) -> AttachmentDisplay {
    AttachmentDisplay {
        path: PathBuf::from(path),
        name: name.map(str::to_owned).unwrap_or_else(|| {
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
                .to_owned()
        }),
        is_image: image,
        is_binary: binary,
        truncated: false,
    }
}

fn parse_marker(text: &str) -> Option<(usize, AttachmentDisplay)> {
    if let Some(rest) = text.strip_prefix(BINARY_PREFIX) {
        let end = rest.find(BINARY_SUFFIX)?;
        let path = &rest[..end];
        if path.is_empty() || path.contains(['\n', '\r']) {
            return None;
        }
        return Some((
            BINARY_PREFIX.len() + end + BINARY_SUFFIX.len(),
            display_ref(path, None, false, true),
        ));
    }
    let (tag, image) = if text.starts_with("<attached-image ") {
        ("<attached-image ", true)
    } else if text.starts_with("<attached-file ") {
        ("<attached-file ", false)
    } else {
        return None;
    };
    // Parse quoted attributes, not `find('>')`: `>` is a valid path character.
    let mut offset = tag.len();
    let mut path = None;
    let mut name = None;
    let self_closing;
    loop {
        let rest = &text[offset..];
        let whitespace = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        offset += whitespace;
        let rest = &text[offset..];
        if rest.starts_with("/>") {
            offset += 2;
            self_closing = true;
            break;
        }
        if rest.starts_with('>') {
            offset += 1;
            self_closing = false;
            break;
        }
        let (key, value_start) = if rest.starts_with("path=\"") {
            (false, 6)
        } else if rest.starts_with("name=\"") {
            (true, 6)
        } else {
            return None;
        };
        let end = rest[value_start..].find('"')? + value_start;
        let value = &rest[value_start..end];
        if value.contains(['\n', '\r', '\0']) {
            return None;
        }
        if key {
            if !image || name.replace(value).is_some() {
                return None;
            }
        } else if path.replace(value).is_some() {
            return None;
        }
        offset += end + 1;
        // Attributes must be separated; the closing delimiter need not be.
        if !text[offset..].starts_with([' ', '\t', '/', '>']) {
            return None;
        }
    }
    let path = path.filter(|path| !path.is_empty())?;
    if image {
        return self_closing.then(|| (offset, display_ref(path, name, true, false)));
    }
    let mut attachment = display_ref(path, None, false, self_closing);
    if self_closing {
        return Some((offset, attachment));
    }
    let rest = text[offset..].strip_prefix("\n```\n")?;
    offset += "\n```\n".len();
    let mut search = 0;
    let mut last_fence = None;
    while let Some(found) = rest[search..].find("\n```") {
        let end = search + found;
        let after = end + "\n```".len();
        if rest[after..].is_empty() || rest[after..].starts_with("\n\n") {
            last_fence = Some((end, after));
            // The composer appends attachment blocks to the message. A code
            // fence inside a Markdown attachment can also be followed by a
            // blank line, so only stop early at the next attachment marker.
            let next = rest[after..].strip_prefix("\n\n");
            if rest[after..].is_empty() || next.is_some_and(marker_start) {
                break;
            }
        }
        search = after;
    }
    if let Some((end, after)) = last_fence {
        attachment.truncated = rest[..end].ends_with("\n… (truncated)");
        return Some((offset + after, attachment));
    }
    Some((text.len(), attachment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_edits_preserve_raw_attachment_blocks_without_reading_files() {
        let markers = concat!(
            "<attached-file path=\"/missing/file &quot;.txt\">\n```\n",
            "原附件内容\n… (truncated)\n```\n\n",
            "<attached-image path=\"/missing/image.png\" name=\"原图\"/>\n\n",
            "[attached binary: /missing/raw.bin — binary file, not inlined]"
        );
        let original = format!(" original \n\n{markers}");
        let edit = EditablePayload::parse(&original);
        assert_eq!(edit.text, " original ");
        assert_eq!(edit.attachments.len(), 3);
        assert_eq!(edit.with_text(&edit.text), original);
        assert_eq!(
            edit.with_text("新正文\n第二行"),
            format!("新正文\n第二行\n\n{markers}")
        );
        assert_eq!(edit.with_text(""), format!("\n\n{markers}"));
    }

    #[test]
    fn queued_edits_do_not_discard_unrecognized_or_unmodified_text() {
        for payload in [
            " leading\ntrailing ",
            "body\n\n<attached-image path=\"broken",
            "before\n\n<attached-file wrong=\"/file\">not an attachment",
        ] {
            let edit = EditablePayload::parse(payload);
            assert!(edit.attachments.is_empty());
            assert_eq!(edit.text, payload);
            assert_eq!(edit.with_text(&edit.text), payload);
            assert_eq!(edit.with_text("changed"), "changed");
        }
    }

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "gpui-attachments-{}-{stamp}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
        fn load(&self, name: &str, bytes: impl AsRef<[u8]>) -> Attachment {
            Attachment::load(self.write(name, bytes), &self.0).unwrap()
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exact_current_protocol_and_display_round_trip() {
        let dir = TestDir::new();
        let text = dir.load("note.md", "正文\n<&> \" ' `");
        let image = dir.load("photo.PNG", b"\0not decoded");
        let binary = dir.load("raw.bin", b"a\0b");
        let payload = build_payload(
            " \u{feff}hello\n ",
            &[text.clone(), image.clone(), binary.clone()],
        );
        assert_eq!(
            payload,
            format!(
                "hello\n\n<attached-file path=\"{}\">\n```\n正文\n<&> \" ' `\n```\n\n<attached-image path=\"{}\" name=\"photo.PNG\"/>\n\n[attached binary: {} — binary file, not inlined]",
                text.path.display(),
                image.path.display(),
                binary.path.display()
            )
        );
        let (body, chips) = display_payload(&payload);
        assert_eq!(body, "hello");
        assert_eq!(chips.len(), 3);
        assert_eq!(chips[0].name, "note.md");
        assert!(chips[1].is_image);
        assert!(chips[2].is_binary);
        assert_eq!(chips[2].path, binary.path);
        assert_eq!(text, text.clone());
    }

    #[test]
    fn svg_is_inlined_as_text_not_a_vision_image() {
        let dir = TestDir::new();
        let svg = dir.load("drawing.svg", "<svg></svg>");
        assert!(!svg.is_image);
        let payload = build_payload("", &[svg]);
        assert!(payload.contains("<attached-file "));
        assert!(payload.contains("<svg></svg>"));
    }

    #[test]
    fn unicode_cap_is_byte_based_and_never_splits_a_character() {
        let dir = TestDir::new();
        for character in ["é", "中", "😀"] {
            for overhang in 1..character.len() {
                let prefix = "a".repeat(MAX_TEXT_BYTES - overhang);
                let attachment = dir.load("unicode.txt", format!("{prefix}{character}tail"));
                assert_eq!(attachment.content.as_deref(), Some(prefix.as_str()));
                assert!(attachment.truncated);
                let (body, chips) = display_payload(&build_payload("", &[attachment]));
                assert_eq!(body, "");
                assert!(chips[0].truncated);
            }
        }
        let exact = dir.load("exact.txt", "中".repeat(MAX_TEXT_BYTES / 3) + "ab");
        assert!(!exact.truncated);
        assert_eq!(exact.content.unwrap().len(), MAX_TEXT_BYTES);
        let empty = dir.load("empty.txt", "");
        assert_eq!(empty.content.as_deref(), Some(""));
    }

    #[test]
    fn malformed_utf8_and_nul_are_binary_not_lossy_text() {
        let dir = TestDir::new();
        for bytes in [&b"\xff\xfe"[..], &b"a\0b"[..], &b"\xe4\xb8"[..]] {
            let attachment = dir.load("binary.dat", bytes);
            assert!(!attachment.is_image);
            assert!(attachment.content.is_none());
            assert!(!attachment.truncated);
            let (body, chips) = display_payload(&build_payload("", &[attachment]));
            assert!(body.is_empty());
            assert!(chips[0].is_binary);
        }
    }

    #[test]
    fn prefix_read_is_bounded_even_for_sparse_large_files() {
        let dir = TestDir::new();
        let path = dir.write("large.log", "a".repeat(MAX_TEXT_BYTES + 1));
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(1 << 34)
            .unwrap();
        let attachment = Attachment::load(path.clone(), &dir.0).unwrap();
        assert_eq!(attachment.content.as_ref().unwrap().len(), MAX_TEXT_BYTES);
        assert!(attachment.truncated);
        assert_eq!(fs::metadata(path).unwrap().len(), 1 << 34);
    }

    #[test]
    fn images_are_extension_only_and_do_not_inline_bytes() {
        let dir = TestDir::new();
        for extension in ["png", "JpG", "jpeg", "gif", "webp", "bmp"] {
            let attachment = dir.load(&format!("test.{extension}"), b"\xff\0");
            assert!(attachment.is_image);
            assert!(attachment.content.is_none());
            assert!(!attachment.truncated);
        }
        assert!(!dir.load("not-image.txt", "png").is_image);
    }

    #[test]
    fn special_characters_are_raw_and_unsafe_paths_are_rejected() {
        let dir = TestDir::new();
        let name = "文 &quot; < > ' & [x].txt";
        let attachment = dir.load(name, "hello");
        let (body, chips) = display_payload(&build_payload("before", &[attachment]));
        assert_eq!(body, "before");
        assert_eq!(chips[0].name, name);
        for name in [
            "quote\".txt",
            "line\nbreak.txt",
            "carriage\r.txt",
            "bad — binary file, not inlined].txt",
        ] {
            let path = dir.write(name, "unchanged");
            assert!(Attachment::load(path.clone(), &dir.0).is_err());
            assert_eq!(fs::read_to_string(path).unwrap(), "unchanged");
        }
    }

    #[test]
    fn external_copies_keep_basename_and_never_overwrite() {
        let workspace = TestDir::new();
        let first = TestDir::new();
        let second = TestDir::new();
        let a_path = first.write("same.txt", "first");
        let b_path = second.write("same.txt", "second");
        let a = Attachment::load(a_path.clone(), &workspace.0).unwrap();
        let b = Attachment::load(b_path.clone(), &workspace.0).unwrap();
        let again = Attachment::load(a_path.clone(), &workspace.0).unwrap();
        assert_ne!(a.path, b.path);
        assert_ne!(a.path, again.path);
        assert_eq!(a.source_path, again.source_path);
        for attachment in [&a, &b, &again] {
            assert!(
                attachment
                    .path
                    .starts_with(workspace.0.join(".husk/attachments"))
            );
            assert_eq!(attachment.path.file_name().unwrap(), "same.txt");
            assert_eq!(
                fs::read(&attachment.source_path).unwrap(),
                fs::read(&attachment.path).unwrap()
            );
            let (_, chips) = display_payload(&build_payload("", &[attachment.clone()]));
            assert_eq!(chips[0].name, attachment.name);
        }
        assert_eq!(fs::read_to_string(a_path).unwrap(), "first");
        assert_eq!(fs::read_to_string(b_path).unwrap(), "second");
        assert_eq!(
            fs::read_to_string(workspace.0.join(".husk/attachments/.gitignore")).unwrap(),
            "*\n"
        );
        assert!(!workspace.0.join(".husk/.gitignore").exists());
    }

    #[test]
    fn workspace_paths_stay_exact_and_existing_gitignore_is_preserved() {
        let workspace = TestDir::new();
        let source = workspace.write("local.txt", "local");
        let attachment = Attachment::load(source.clone(), &workspace.0).unwrap();
        assert_eq!(attachment.path, source);
        assert_eq!(attachment.source_path, source);
        assert!(!workspace.0.join(".husk").exists());
        fs::create_dir(workspace.0.join(".husk")).unwrap();
        let ignore = workspace.write(".husk/.gitignore", "keep me\n");
        let external = TestDir::new();
        Attachment::load(external.write("external.txt", "content"), &workspace.0).unwrap();
        assert_eq!(fs::read_to_string(ignore).unwrap(), "keep me\n");
    }

    #[test]
    fn staging_ignores_attachments_without_hiding_other_husk_files() {
        let workspace = TestDir::new();
        let external = TestDir::new();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&workspace.0)
                .status()
                .unwrap()
                .success()
        );
        let attachment =
            Attachment::load(external.write("image.png", b"image"), &workspace.0).unwrap();
        let config = workspace.write(".husk/config.json", "{}");
        let ignored = |path: &Path| {
            std::process::Command::new("git")
                .current_dir(&workspace.0)
                .args(["check-ignore", "--no-index", "-q", "--"])
                .arg(path.strip_prefix(&workspace.0).unwrap())
                .status()
                .unwrap()
                .success()
        };
        assert!(ignored(&attachment.path));
        assert!(!ignored(&config));
    }

    #[test]
    fn relative_workspace_selection_produces_an_absolute_prompt_path() {
        let workspace = tempfile::tempdir_in(".").unwrap();
        let source = workspace.path().join("relative.txt");
        fs::write(&source, "content").unwrap();
        let attachment = Attachment::load(source.clone(), workspace.path()).unwrap();
        assert_eq!(attachment.source_path, source);
        assert_eq!(attachment.path, fs::canonicalize(source).unwrap());
        assert!(attachment.path.is_absolute());
    }

    #[test]
    fn failures_are_explicit_and_leave_original_untouched() {
        let workspace = TestDir::new();
        let external = TestDir::new();
        let source = external.write("external.txt", "unchanged");
        workspace.write(".husk", "not a directory");
        assert!(Attachment::load(source.clone(), &workspace.0).is_err());
        assert_eq!(fs::read_to_string(source).unwrap(), "unchanged");
        assert!(Attachment::load(external.0.clone(), &workspace.0).is_err());
        assert!(Attachment::load(external.0.join("missing"), &workspace.0).is_err());
        assert!(
            Attachment::load(external.write("ok.txt", "ok"), &workspace.0.join("missing")).is_err()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn unix_source_open_rejects_fifos_and_final_symlinks() {
        use rustix::fs::{Mode, mkfifoat};
        use std::os::unix::fs::symlink;

        let dir = TestDir::new();
        let directory = File::open(&dir.0).unwrap();
        mkfifoat(&directory, "pipe", Mode::from_raw_mode(0o600)).unwrap();
        assert_eq!(
            scoped_files::open_regular(&dir.0.join("pipe"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        dir.write("target", "content");
        symlink(dir.0.join("target"), dir.0.join("link")).unwrap();
        assert!(scoped_files::open_regular(&dir.0.join("link")).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn unix_staging_cleans_up_a_failed_copy() {
        let workspace = TestDir::new();
        let external = TestDir::new();
        let mut write_only = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(external.0.join("source.txt"))
            .unwrap();
        assert!(stage_file(&mut write_only, "source.txt", &workspace.0).is_err());
        let entries: Vec<_> = fs::read_dir(workspace.0.join(".husk/attachments"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [std::ffi::OsString::from(".gitignore")]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_workspace_or_redirect_staging() {
        use std::os::unix::fs::symlink;
        let workspace = TestDir::new();
        let external = TestDir::new();
        let source = external.write("outside.txt", "secret");
        let link = workspace.0.join("linked.txt");
        symlink(&source, &link).unwrap();
        let attachment = Attachment::load(link.clone(), &workspace.0).unwrap();
        assert_eq!(attachment.source_path, link);
        assert_ne!(attachment.path, link);
        assert!(
            fs::canonicalize(&attachment.path)
                .unwrap()
                .starts_with(&workspace.0)
        );
        let blocked = TestDir::new();
        symlink(&external.0, blocked.0.join(".husk")).unwrap();
        assert!(Attachment::load(source.clone(), &blocked.0).is_err());
        assert!(!external.0.join("attachments").exists());
        let blocked = TestDir::new();
        fs::create_dir(blocked.0.join(".husk")).unwrap();
        symlink(&external.0, blocked.0.join(".husk/attachments")).unwrap();
        assert!(Attachment::load(source.clone(), &blocked.0).is_err());
        assert_eq!(fs::read_to_string(source).unwrap(), "secret");
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_source_workspace_and_resolved_paths_are_rejected() {
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        let workspace = TestDir::new();
        let invalid = workspace
            .0
            .join(std::ffi::OsString::from_vec(b"bad-\xff".to_vec()));
        fs::write(&invalid, "untouched").unwrap();
        assert!(Attachment::load(invalid.clone(), &workspace.0).is_err());
        let alias = workspace.0.join("alias.txt");
        symlink(&invalid, &alias).unwrap();
        assert!(Attachment::load(alias, &workspace.0).is_err());
        let invalid_root = workspace
            .0
            .join(std::ffi::OsString::from_vec(b"root-\xff".to_vec()));
        fs::create_dir(&invalid_root).unwrap();
        assert!(Attachment::load(workspace.write("valid.txt", "ok"), &invalid_root).is_err());
        assert_eq!(fs::read_to_string(invalid).unwrap(), "untouched");
    }

    #[test]
    fn display_preserves_prose_and_supports_legacy_binary_marker() {
        let text = "before\n\n<attached-image path=\"/a>b &quot;.png\" name=\"图 &quot;\"/>\n\nmiddle\n\n<attached-file path=\"/raw.bin\" />\n\nafter";
        let (body, chips) = display_payload(text);
        assert_eq!(body, "before\n\nmiddle\n\nafter");
        assert_eq!(chips[0].name, "图 &quot;");
        assert_eq!(chips[0].path, PathBuf::from("/a>b &quot;.png"));
        assert!(chips[1].is_binary);
        let (_, chips) = display_payload("<attached-image path=\"/image.png\"/>");
        assert_eq!(chips[0].name, "image.png");
    }

    #[test]
    fn display_does_not_expose_markdown_code_blocks_from_a_text_attachment() {
        let dir = TestDir::new();
        let markdown = dir.load(
            "notes.md",
            "# Guide\n\n```rust\nfn main() {}\n```\n\nPrivate notes after the code.\n\n```sh\necho ok\n```\n\nMore notes.",
        );
        let image = dir.load("photo.png", b"png");
        let payload = build_payload("Review", &[markdown, image]);
        let (body, attachments) = display_payload(&payload);
        assert_eq!(body, "Review");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].name, "notes.md");
        assert_eq!(attachments[1].name, "photo.png");
    }

    #[test]
    fn damaged_messages_do_not_panic_or_expose_an_open_inline_body() {
        for text in [
            "",
            "普通正文",
            "<attached-image",
            "\n\n<attached-file path=\"",
            "\n\n<attached-image path=\"x\" name=\"broken/>",
            "\n\n[attached binary: missing end",
            "inline <attached-image path=\"x\"/> prose",
            "\n\n<attached-file path=\"x\">not fenced",
            "\n\n<attached-image path=\"x\" path=\"y\"/>",
            "\n\n<attached-image name=\"x\"/>",
        ] {
            assert_eq!(display_payload(text), (text.to_owned(), Vec::new()));
        }
        let (body, chips) =
            display_payload("body\n\n<attached-file path=\"/secret.txt\">\n```\nsecret\nforever");
        assert_eq!(body, "body");
        assert_eq!(chips.len(), 1);
        let text = "before\n\n<attached-file path=\"/file.txt\">\n```\n内文\n```not-a-fence\nhidden\n```\n\nafter";
        assert_eq!(display_payload(text).0, "before\n\nafter");
    }
}
