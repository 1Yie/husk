//! Process logging and panic capture.
//!
//! A packaged GUI has no terminal: without a file sink every `tracing` event
//! (including the session-actor panic the manager already logs) is lost. The
//! log is capped at startup so a long-running app cannot fill the disk with
//! diagnostics.
//!
//! The panic hook writes to the file directly instead of going through
//! `tracing`: a panic inside the logging writer must not deadlock on the same
//! mutex it already holds.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Rotate once at boot when the previous run grew past this size.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Install the file + stderr subscriber. Safe to call once, before GTK/Tauri
/// initializes so early failures are captured.
pub fn init() {
    let dir = crate::kernel::app_paths().data_dir.join("logs");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("husk.log");
    rotate(&path);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    // Keep the path even when the file could not be opened: the panic hook
    // reports the failure by doing nothing rather than panicking again.
    let _ = LOG_PATH.set(path.clone());

    let writer = SharedWriter {
        file: file.map(|file| Arc::new(Mutex::new(file))),
    };
    // `HUSK_LOG=debug|trace` is the escape hatch for a support run; the default
    // stays quiet enough for an always-on desktop log.
    let level = std::env::var("HUSK_LOG")
        .ok()
        .and_then(|level| level.parse().ok())
        .unwrap_or(tracing::Level::INFO);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_target(true)
        .with_writer(MakeShared { writer })
        .finish();
    if let Err(error) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("husk: logging already installed: {error}");
    }
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        log = %path.display(),
        "husk starting"
    );
}

/// The current log file, for diagnostics pages / bug reports.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

/// Record panics that would otherwise vanish into stderr (or abort silently
/// in release builds, where the profile uses `panic = "abort"`).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "<unknown>".into());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".into());
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let line = format!("{seconds} PANIC {location}: {message}\n");
        let _ = io::stderr().write_all(line.as_bytes());
        append_line(&line);
        default(info);
    }));
}

fn rotate(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < MAX_LOG_BYTES {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(path, old);
}

fn append_line(line: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[derive(Clone)]
struct SharedWriter {
    file: Option<Arc<Mutex<File>>>,
}

struct MakeShared {
    writer: SharedWriter,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MakeShared {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.writer.clone()
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(buf);
            }
        }
        // A terminal closed under a dev run must never turn logging into an
        // error path; the file sink above is the durable one.
        let _ = io::stderr().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.flush();
            }
        }
        let _ = io::stderr().flush();
        Ok(())
    }
}
