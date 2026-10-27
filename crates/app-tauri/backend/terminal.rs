//! Window-owned interactive PTYs. No input is interpreted as a host command.
//!
//! The registry never holds a lock over spawn, I/O, or wait. A dedicated reaper
//! owns each child; closing only sets cancellation and waits for that reaper.
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

const MAX_DIMENSION: u32 = 1000;
const MAX_CELLS: u32 = 250_000;
const MAX_INPUT: usize = 64 * 1024;
const MAX_TERMINALS_PER_WINDOW: usize = 8;
const OUTPUT_CHUNK: usize = 8 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

type EventSink = Arc<dyn Fn(TerminalEvent) -> Result<(), String> + Send + Sync>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalEvent {
    Data {
        data: Vec<u8>,
    },
    Exit {
        code: Option<u32>,
        signal: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Serialize)]
pub struct OpenedTerminal {
    pub id: String,
    pub cwd: String,
    pub shell: String,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn size(cols: u32, rows: u32) -> Result<PtySize, String> {
    if cols == 0
        || rows == 0
        || cols > MAX_DIMENSION
        || rows > MAX_DIMENSION
        || cols.saturating_mul(rows) > MAX_CELLS
    {
        return Err("terminal dimensions must be 1..=1000 and at most 250000 cells".into());
    }
    Ok(PtySize {
        cols: cols as u16,
        rows: rows as u16,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn working_directory(cwd: Option<String>) -> Result<PathBuf, String> {
    let path = match cwd {
        Some(path) => {
            if path.len() > 16 * 1024 || path.contains('\0') {
                return Err("invalid terminal cwd".into());
            }
            PathBuf::from(path)
        }
        None => std::env::current_dir()
            .ok()
            .filter(|p| p.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir),
    };
    if !path.is_absolute() || !path.is_dir() {
        return Err("terminal cwd must be an existing absolute directory".into());
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("invalid terminal cwd: {e}"))?;
    if path.to_str().is_none() {
        return Err("terminal cwd must be valid Unicode".into());
    }
    Ok(path)
}

fn shell_command() -> (CommandBuilder, String) {
    #[cfg(unix)]
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| Path::new(s).is_absolute() && Path::new(s).is_file())
        .unwrap_or_else(|| "/bin/sh".into());
    #[cfg(windows)]
    let shell = std::env::var("COMSPEC")
        .ok()
        .filter(|s| Path::new(s).is_absolute() && Path::new(s).is_file())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32")
                .join("cmd.exe")
                .to_string_lossy()
                .into_owned()
        });
    let mut cmd = CommandBuilder::new(&shell);
    #[cfg(unix)]
    cmd.arg("-i");
    #[cfg(windows)]
    cmd.args(["/D", "/Q"]);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    (cmd, shell)
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    destroyed_owners: HashSet<String>,
    shutting_down: bool,
}

#[derive(Default, Clone)]
pub struct TerminalState {
    registry: Arc<Mutex<Registry>>,
    shutdown_started: Arc<AtomicBool>,
    shutdown_complete: Arc<AtomicBool>,
}

struct Session {
    owner: String,
    stop: AtomicBool,
    child_done: AtomicBool,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    done: Mutex<bool>,
    completion: Condvar,
}

impl Session {
    fn new(owner: String) -> Self {
        Self {
            owner,
            stop: AtomicBool::new(false),
            child_done: AtomicBool::new(false),
            master: Mutex::new(None),
            writer: Mutex::new(None),
            done: Mutex::new(false),
            completion: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut done = lock(&self.done);
        while !*done {
            done = self
                .completion
                .wait(done)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn finish(&self) {
        *lock(&self.done) = true;
        self.completion.notify_all();
    }
}

// Also covers failed spawns/thread creation: portable-pty's Child does NOT
// kill/wait on Drop. This guard is always dropped on a blocking worker.
struct ReapChild(Option<Box<dyn Child + Send + Sync>>);
impl Drop for ReapChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl TerminalState {
    pub fn open(
        &self,
        owner: String,
        cwd: Option<String>,
        cols: u32,
        rows: u32,
        sink: EventSink,
    ) -> Result<OpenedTerminal, String> {
        let size = size(cols, rows)?;
        let cwd = working_directory(cwd)?;
        let (cmd, shell) = shell_command();
        self.open_command(owner, cwd, size, cmd, shell, sink)
    }

    fn open_command(
        &self,
        owner: String,
        cwd: PathBuf,
        size: PtySize,
        mut cmd: CommandBuilder,
        shell: String,
        sink: EventSink,
    ) -> Result<OpenedTerminal, String> {
        let id = format!("pty-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let session = Arc::new(Session::new(owner.clone()));
        {
            let mut registry = lock(&self.registry);
            if registry.shutting_down || registry.destroyed_owners.contains(&owner) {
                return Err("terminal window is closing".into());
            }
            if registry
                .sessions
                .values()
                .filter(|s| s.owner == owner)
                .count()
                >= MAX_TERMINALS_PER_WINDOW
            {
                return Err("too many terminals in this window (maximum 8)".into());
            }
            // Reserve before spawn: concurrent opens and window destruction
            // include in-flight spawns in their limit/cancellation accounting.
            registry.sessions.insert(id.clone(), session.clone());
        }
        cmd.cwd(&cwd);
        let result = self.spawn(&id, session.clone(), size, cmd, sink);
        if let Err(error) = result {
            lock(&self.registry).sessions.remove(&id);
            session.finish();
            return Err(error);
        }
        Ok(OpenedTerminal {
            id,
            cwd: cwd.to_string_lossy().into_owned(),
            shell,
        })
    }

    fn spawn(
        &self,
        id: &str,
        session: Arc<Session>,
        size: PtySize,
        cmd: CommandBuilder,
        sink: EventSink,
    ) -> Result<(), String> {
        if session.stop.load(Ordering::Acquire) {
            return Err("terminal window is closing".into());
        }
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("open PTY: {e}"))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("read PTY: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("write PTY: {e}"))?;
        #[cfg(unix)]
        let read_fd = pair
            .master
            .as_raw_fd()
            .map(|fd| {
                // SAFETY: pair.master still owns the live descriptor here. Clone
                // immediately; the output worker owns the duplicate independently.
                unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()
            })
            .transpose()
            .map_err(|e| format!("poll PTY: {e}"))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn terminal shell: {e}"))?;
        let mut child = ReapChild(Some(child));
        drop(pair.slave); // Keeping the slave here would prevent EOF forever.
        *lock(&session.master) = Some(pair.master);
        *lock(&session.writer) = Some(writer);
        let read_session = session.clone();
        let read_sink = sink.clone();
        let read_thread = std::thread::Builder::new()
            .name("terminal-output".into())
            .spawn(move || {
                read_output(
                    reader,
                    &read_session,
                    &read_sink,
                    #[cfg(unix)]
                    read_fd,
                );
            });
        let read_thread = match read_thread {
            Ok(thread) => thread,
            Err(error) => {
                drop(child); // Kill before closing handles, including ConPTY.
                lock(&session.writer).take();
                lock(&session.master).take();
                return Err(format!("start terminal reader: {error}"));
            }
        };
        let registry = self.registry.clone();
        let id = id.to_owned();
        let reaper_session = session.clone();
        let reaper = std::thread::Builder::new()
            .name("terminal-reaper".into())
            .spawn(move || {
                let session = reaper_session;
                let process = child.0.as_mut().expect("owned child");
                let status = loop {
                    match process.try_wait() {
                        Ok(Some(status)) => break Ok(status),
                        Err(error) => {
                            let _ = process.kill();
                            let _ = process.wait();
                            break Err(error);
                        }
                        Ok(None) => {}
                    }
                    if session.stop.load(Ordering::Acquire) {
                        // Child::kill includes a hard-kill fallback on Unix; the
                        // cloned killer only sends SIGHUP and is not sufficient.
                        let _ = process.kill();
                        break process.wait();
                    }
                    std::thread::sleep(Duration::from_millis(10));
                };
                child.0.take(); // Already reaped; never signal a reused PID.
                session.child_done.store(true, Ordering::Release);
                // No registry lock, and no lock used by the child waiter. A blocked
                // write is released by termination of the slave process.
                lock(&session.writer).take();
                #[cfg(windows)]
                lock(&session.master).take(); // Close ConPTY while reader drains it.
                let _ = read_thread.join();
                #[cfg(unix)]
                lock(&session.master).take();
                match status {
                    Ok(status) => {
                        let signal = status.signal().map(str::to_owned);
                        let code = if signal.is_some() {
                            None
                        } else {
                            Some(status.exit_code())
                        };
                        let _ = sink(TerminalEvent::Exit { code, signal });
                    }
                    Err(error) => {
                        if !session.stop.load(Ordering::Acquire) {
                            let _ = sink(TerminalEvent::Error {
                                message: format!("wait for terminal: {error}"),
                            });
                        }
                        let _ = sink(TerminalEvent::Exit {
                            code: None,
                            signal: None,
                        });
                    }
                }
                lock(&registry).sessions.remove(&id);
                session.finish();
            });
        if let Err(error) = reaper {
            // The rejected closure drops ReapChild (kill + wait). The reader
            // observes child_done and releases its cloned handle on Unix.
            session.stop.store(true, Ordering::Release);
            session.child_done.store(true, Ordering::Release);
            lock(&session.writer).take();
            lock(&session.master).take();
            return Err(format!("start terminal reaper: {error}"));
        }
        Ok(())
    }

    fn owned(&self, owner: &str, id: &str) -> Result<Arc<Session>, String> {
        let registry = lock(&self.registry);
        let session = registry.sessions.get(id).ok_or("terminal not found")?;
        if session.owner != owner {
            return Err("terminal belongs to another window".into());
        }
        Ok(session.clone())
    }

    pub fn write(&self, owner: &str, id: &str, data: &str) -> Result<(), String> {
        let session = self.owned(owner, id)?;
        if data.len() > MAX_INPUT {
            return Err("terminal input exceeds 64 KiB".into());
        }
        // Reject concurrent writes instead of parking unbounded IPC workers.
        let mut writer = session
            .writer
            .try_lock()
            .map_err(|_| "terminal input is busy")?;
        if session.stop.load(Ordering::Acquire) || session.child_done.load(Ordering::Acquire) {
            return Err("terminal is closed".into());
        }
        let writer = writer.as_mut().ok_or("terminal is not ready")?;
        writer
            .write_all(data.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| format!("write terminal: {e}"))
    }

    pub fn resize(&self, owner: &str, id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let session = self.owned(owner, id)?;
        let size = size(cols, rows)?;
        let master = lock(&session.master);
        if session.stop.load(Ordering::Acquire) || session.child_done.load(Ordering::Acquire) {
            return Err("terminal is closed".into());
        }
        master
            .as_ref()
            .ok_or("terminal is not ready")?
            .resize(size)
            .map_err(|e| format!("resize terminal: {e}"))
    }

    pub fn close(&self, owner: &str, id: &str) -> Result<(), String> {
        let session = {
            let registry = lock(&self.registry);
            let Some(session) = registry.sessions.get(id) else {
                return Ok(());
            };
            if session.owner != owner {
                return Err("terminal belongs to another window".into());
            }
            session.clone()
        };
        session.stop.store(true, Ordering::Release);
        session.wait();
        Ok(())
    }

    /// Call immediately on Destroyed, not on CloseRequested/hide-to-tray.
    /// It tombstones the label before a concurrent spawn can reserve a slot.
    pub fn destroy_owner(&self, owner: &str) {
        let mut registry = lock(&self.registry);
        registry.destroyed_owners.insert(owner.to_owned());
        for session in registry.sessions.values().filter(|s| s.owner == owner) {
            session.stop.store(true, Ordering::Release);
        }
    }

    pub fn start_shutdown(&self) -> bool {
        if self.shutdown_started.swap(true, Ordering::AcqRel) {
            return false;
        }
        let mut registry = lock(&self.registry);
        registry.shutting_down = true;
        for session in registry.sessions.values() {
            session.stop.store(true, Ordering::Release);
        }
        true
    }

    pub fn shutdown(&self) {
        self.start_shutdown();
        let sessions: Vec<_> = lock(&self.registry).sessions.values().cloned().collect();
        for session in sessions {
            session.wait();
        }
        self.shutdown_complete.store(true, Ordering::Release);
    }

    pub fn shutdown_complete(&self) -> bool {
        self.shutdown_complete.load(Ordering::Acquire)
    }
}

fn read_output(
    mut reader: Box<dyn Read + Send>,
    session: &Session,
    sink: &EventSink,
    #[cfg(unix)] fd: Option<OwnedFd>,
) {
    let mut buffer = [0; OUTPUT_CHUNK];
    loop {
        // PTY descendants can keep the slave open after the shell exits. Poll
        // the native master so cancellation/exit never leaves a stuck reader.
        #[cfg(unix)]
        if let Some(fd) = fd.as_ref() {
            match readable(fd.as_raw_fd()) {
                Ok(false) => {
                    if session.child_done.load(Ordering::Acquire)
                        || session.stop.load(Ordering::Acquire)
                    {
                        break;
                    }
                    continue;
                }
                Ok(true) => {}
                Err(error) => {
                    report_read_error(session, sink, error);
                    break;
                }
            }
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if !session.stop.load(Ordering::Acquire)
                    && sink(TerminalEvent::Data {
                        data: buffer[..count].to_vec(),
                    })
                    .is_err()
                {
                    session.stop.store(true, Ordering::Release);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // Linux PTYs report EIO, not read(0), when their last slave closes.
            #[cfg(target_os = "linux")]
            Err(error) if error.raw_os_error() == Some(5) => break,
            Err(error) => {
                report_read_error(session, sink, error);
                break;
            }
        }
        #[cfg(unix)]
        if session.stop.load(Ordering::Acquire) {
            break;
        }
    }
}

fn report_read_error(session: &Session, sink: &EventSink, error: std::io::Error) {
    if !session.stop.load(Ordering::Acquire) && !session.child_done.load(Ordering::Acquire) {
        let _ = sink(TerminalEvent::Error {
            message: format!("read terminal: {error}"),
        });
        session.stop.store(true, Ordering::Release);
    }
}

/// Borrow the master's fd; it stays open until the reader has joined. Use
/// libc's platform-specific poll ABI (nfds_t differs across Unix targets).
#[cfg(unix)]
fn readable(fd: std::os::fd::RawFd) -> std::io::Result<bool> {
    let mut fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: valid single-element pollfd array, borrowed live descriptor.
    let result = unsafe { libc::poll(&mut fd, 1, 25) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(result > 0)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Instant;

    struct Cleanup(TerminalState);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.shutdown();
        }
    }

    fn script(
        state: &TerminalState,
        root: &Path,
        source: &str,
    ) -> (OpenedTerminal, Receiver<TerminalEvent>) {
        let (tx, rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |event| tx.send(event).map_err(|e| e.to_string()));
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", source]);
        let info = state
            .open_command(
                "main".into(),
                root.to_path_buf(),
                size(80, 24).unwrap(),
                command,
                "/bin/sh".into(),
                sink,
            )
            .unwrap();
        (info, rx)
    }

    fn text_until(rx: &Receiver<TerminalEvent>, needle: &str) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(TerminalEvent::Data { data }) => {
                    bytes.extend(data);
                    if String::from_utf8_lossy(&bytes).contains(needle) {
                        return bytes;
                    }
                }
                Ok(event) => panic!("unexpected PTY event: {event:?}"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("PTY channel closed: {error}"),
            }
        }
        panic!(
            "PTY did not output {needle:?}: {:?}",
            String::from_utf8_lossy(&bytes)
        )
    }

    #[test]
    fn dimensions_and_cwd_are_validated() {
        assert!(size(80, 24).is_ok());
        for (cols, rows) in [(0, 24), (80, 0), (1001, 1), (1, 1001), (1000, 1000)] {
            assert!(size(cols, rows).is_err());
        }
        assert!(working_directory(Some("relative/path".into())).is_err());
        assert!(working_directory(Some("invalid\0path".into())).is_err());
        let directory = tempfile::tempdir().unwrap();
        let resolved =
            working_directory(Some(directory.path().to_string_lossy().into_owned())).unwrap();
        assert_eq!(resolved, directory.path().canonicalize().unwrap());
    }

    #[test]
    fn real_pty_accepts_input_resizes_and_isolates_windows() {
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (info, rx) = script(
            &state,
            directory.path(),
            "test -t 0 && test -t 1 && printf '\\033[31mpty-ready\\033[0m\\n'; cat",
        );
        let bytes = text_until(&rx, "pty-ready");
        assert!(bytes.windows(5).any(|part| part == b"\x1b[31m"));
        assert!(state
            .write("another-window", &info.id, "not allowed\n")
            .is_err());
        assert!(state.resize("another-window", &info.id, 100, 30).is_err());
        assert!(state.close("another-window", &info.id).is_err());
        assert!(state
            .write("main", &info.id, &"x".repeat(MAX_INPUT + 1))
            .is_err());
        state.resize("main", &info.id, 100, 30).unwrap();
        {
            let session = state.owned("main", &info.id).unwrap();
            let master = lock(&session.master);
            let actual = master.as_ref().unwrap().get_size().unwrap();
            assert_eq!((actual.cols, actual.rows), (100, 30));
        }
        state.write("main", &info.id, "hello-pty\n").unwrap();
        text_until(&rx, "hello-pty");
        state.close("main", &info.id).unwrap();
        assert!(state.owned("main", &info.id).is_err());
        assert!(state.write("main", &info.id, "closed").is_err());
        state.close("main", &info.id).unwrap();
    }

    #[test]
    fn exit_drains_bytes_before_reporting_exit_code() {
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (_, rx) = script(&state, directory.path(), "printf 'finished-pty'; exit 7");
        let mut bytes = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TerminalEvent::Data { data } => bytes.extend(data),
                TerminalEvent::Exit { code, signal } => {
                    assert_eq!(code, Some(7));
                    assert!(signal.is_none());
                    break;
                }
                TerminalEvent::Error { message } => panic!("{message}"),
            }
        }
        assert!(String::from_utf8_lossy(&bytes).contains("finished-pty"));
    }

    #[test]
    fn control_c_interrupts_the_foreground_process() {
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (info, rx) = script(
            &state,
            directory.path(),
            r#"trap 'printf "signal-interrupted\n"; exit 0' INT; printf 'signal-ready\n'; while :; do sleep 1; done"#,
        );
        text_until(&rx, "signal-ready");
        state.write("main", &info.id, "\x03").unwrap();
        text_until(&rx, "signal-interrupted");
        state.close("main", &info.id).unwrap();
    }

    #[test]
    fn destroyed_window_and_shutdown_reject_new_sessions_and_reap_children() {
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (info, rx) = script(&state, directory.path(), "printf 'ready'; cat");
        text_until(&rx, "ready");
        state.destroy_owner("another-window");
        state.resize("main", &info.id, 81, 25).unwrap();
        state.destroy_owner("main");
        state.close("main", &info.id).unwrap();
        let sink: EventSink = Arc::new(|_| Ok(()));
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        assert!(state
            .open_command(
                "main".into(),
                directory.path().into(),
                size(80, 24).unwrap(),
                command,
                "sh".into(),
                sink
            )
            .is_err());
        state.shutdown();
        assert!(state.shutdown_complete());
        assert!(lock(&state.registry).sessions.is_empty());
    }

    #[test]
    fn failed_spawn_releases_the_reserved_slot() {
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let sink: EventSink = Arc::new(|_| Ok(()));
        let command = CommandBuilder::new(directory.path().join("no-such-shell"));
        assert!(state
            .open_command(
                "main".into(),
                directory.path().into(),
                size(80, 24).unwrap(),
                command,
                "missing".into(),
                sink
            )
            .is_err());
        assert!(lock(&state.registry).sessions.is_empty());
    }
}
