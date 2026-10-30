//! Owner-scoped interactive PTYs. No input is interpreted as a host command.
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
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

const MAX_DIMENSION: u32 = 1000;
const MAX_CELLS: u32 = 250_000;
const MAX_INPUT: usize = 64 * 1024;
pub const MAX_TERMINALS_PER_WINDOW: usize = 8;
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

/// Optional, bounded backpressure for background UI workers. This waits only
/// for an owner slot, not for process spawn or terminal I/O.
#[derive(Clone, Copy)]
pub struct CapacityWait<'a> {
    pub cancelled: &'a AtomicBool,
    pub timeout: Duration,
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

#[cfg(unix)]
fn select_unix_shell(
    environment_shell: Option<String>,
    account_shell: impl FnOnce() -> Option<String>,
) -> String {
    environment_shell
        .filter(|shell| is_executable_shell(shell))
        .or_else(|| account_shell().filter(|shell| is_executable_shell(shell)))
        .unwrap_or_else(|| "/bin/sh".into())
}

#[cfg(unix)]
fn is_executable_shell(shell: &str) -> bool {
    if !Path::new(shell).is_absolute() || !Path::new(shell).is_file() {
        return false;
    }
    let Ok(path) = std::ffi::CString::new(shell) else {
        return false;
    };
    // SAFETY: path is a live, NUL-terminated C string. Check this user's access,
    // not just permission bits (which would miss ACLs and other restrictions).
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(unix)]
fn account_login_shell() -> Option<String> {
    // portable-pty's get_shell uses non-reentrant getpwuid and also accepts
    // relative paths. Resolve the account ourselves, without shared passwd data.
    const MAX_BUFFER: usize = 1024 * 1024;
    // SAFETY: sysconf/getuid take no pointers and have no preconditions.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let uid = unsafe { libc::getuid() };
    let mut length = if suggested > 0 {
        (suggested as usize).clamp(1024, MAX_BUFFER)
    } else {
        1024
    };
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0u8; length];
        let mut result = std::ptr::null_mut();
        // SAFETY: all output pointers are valid for the call; buffer has the
        // advertised length. No passwd fields are read until success is checked.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && length < MAX_BUFFER {
            length = (length * 2).min(MAX_BUFFER);
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a successful getpwuid_r with a non-null result initialized entry.
        let entry = unsafe { entry.assume_init() };
        if entry.pw_shell.is_null() {
            return None;
        }
        // SAFETY: pw_shell is a NUL-terminated string in the still-live buffer.
        // Copy it before dropping the buffer; the IPC shell field is UTF-8.
        return unsafe { std::ffi::CStr::from_ptr(entry.pw_shell) }
            .to_str()
            .ok()
            .map(str::to_owned);
    }
}

fn shell_command() -> (CommandBuilder, String) {
    #[cfg(unix)]
    let shell = select_unix_shell(std::env::var("SHELL").ok(), account_login_shell);
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
    shell_command_for(shell)
}

fn shell_command_for(shell: String) -> (CommandBuilder, String) {
    let mut cmd = CommandBuilder::new(&shell);
    #[cfg(unix)]
    {
        // Only known shells get -l: /bin/sh may be dash, and wrappers/other
        // shells must keep their existing -i invocation rather than guess flags.
        if matches!(
            Path::new(&shell).file_name().and_then(|name| name.to_str()),
            Some("zsh" | "bash" | "fish")
        ) {
            cmd.arg("-l");
        }
        cmd.arg("-i");
        cmd.env("SHELL", &shell);
    }
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
    capacity_changed: Arc<Condvar>,
    shutdown_started: Arc<AtomicBool>,
    shutdown_complete: Arc<AtomicBool>,
}

struct Reservation {
    id: String,
    session: Arc<Session>,
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

    /// Like `open`, but wait for closing/in-flight terminals to release a slot.
    /// Call only on a blocking worker. Cancellation is checked at most 50 ms
    /// apart while waiting; owner retirement and shutdown wake the waiter.
    pub fn open_when_available(
        &self,
        owner: String,
        cwd: Option<String>,
        cols: u32,
        rows: u32,
        sink: EventSink,
        wait: CapacityWait<'_>,
    ) -> Result<OpenedTerminal, String> {
        let size = size(cols, rows)?;
        let cwd = working_directory(cwd)?;
        let (cmd, shell) = shell_command();
        let reservation = self.reserve(&owner, Some(wait))?;
        self.open_reserved(reservation, cwd, size, cmd, shell, sink)
    }

    fn open_command(
        &self,
        owner: String,
        cwd: PathBuf,
        size: PtySize,
        cmd: CommandBuilder,
        shell: String,
        sink: EventSink,
    ) -> Result<OpenedTerminal, String> {
        let reservation = self.reserve(&owner, None)?;
        self.open_reserved(reservation, cwd, size, cmd, shell, sink)
    }

    fn reserve(&self, owner: &str, wait: Option<CapacityWait<'_>>) -> Result<Reservation, String> {
        let deadline = wait
            .map(|wait| {
                Instant::now()
                    .checked_add(wait.timeout)
                    .ok_or("invalid terminal capacity timeout")
            })
            .transpose()?;
        let mut registry = lock(&self.registry);
        loop {
            if wait.is_some_and(|wait| wait.cancelled.load(Ordering::Acquire)) {
                return Err("terminal opening was cancelled".into());
            }
            if registry.shutting_down || registry.destroyed_owners.contains(owner) {
                return Err("terminal window is closing".into());
            }
            if registry
                .sessions
                .values()
                .filter(|session| session.owner == owner)
                .count()
                < MAX_TERMINALS_PER_WINDOW
            {
                break;
            }
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_default();
            if remaining.is_zero() {
                return Err("too many terminals in this window (maximum 8)".into());
            }
            (registry, _) = self
                .capacity_changed
                .wait_timeout(registry, remaining.min(Duration::from_millis(50)))
                .unwrap_or_else(|error| error.into_inner());
        }
        let id = format!("pty-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let session = Arc::new(Session::new(owner.to_owned()));
        // Reserve under the same lock as the capacity check, including pending
        // spawns in the limit. The lock is released before process or PTY I/O.
        registry.sessions.insert(id.clone(), session.clone());
        Ok(Reservation { id, session })
    }

    fn open_reserved(
        &self,
        reservation: Reservation,
        cwd: PathBuf,
        size: PtySize,
        mut cmd: CommandBuilder,
        shell: String,
        sink: EventSink,
    ) -> Result<OpenedTerminal, String> {
        let Reservation { id, session } = reservation;
        cmd.cwd(&cwd);
        let result = self.spawn(&id, session.clone(), size, cmd, sink);
        if let Err(error) = result {
            lock(&self.registry).sessions.remove(&id);
            self.capacity_changed.notify_all();
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
        let capacity_changed = self.capacity_changed.clone();
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
                capacity_changed.notify_all();
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

    /// Permanently retire an owner and cancel its sessions.
    /// Tombstone it before a concurrent spawn can reserve a slot.
    pub fn destroy_owner(&self, owner: &str) {
        let mut registry = lock(&self.registry);
        registry.destroyed_owners.insert(owner.to_owned());
        for session in registry.sessions.values().filter(|s| s.owner == owner) {
            session.stop.store(true, Ordering::Release);
        }
        self.capacity_changed.notify_all();
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
        self.capacity_changed.notify_all();
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

#[cfg(test)]
#[path = "capacity_tests.rs"]
mod capacity_tests;

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;
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

    fn fake_shell(root: &Path, name: &str, executable: bool) -> String {
        let path = root.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn valid_environment_shell_wins_without_querying_the_account() {
        let directory = tempfile::tempdir().unwrap();
        let shell = fake_shell(directory.path(), "explicit-shell", true);
        assert_eq!(
            select_unix_shell(Some(shell.clone()), || panic!("must not query the account")),
            shell
        );
    }

    #[test]
    fn missing_or_invalid_environment_shell_uses_the_account_shell() {
        let directory = tempfile::tempdir().unwrap();
        let account = fake_shell(directory.path(), "account-shell", true);
        let not_executable = fake_shell(directory.path(), "not-executable", false);
        for environment in [
            None,
            Some(String::new()),
            Some("relative-shell".into()),
            Some(directory.path().to_str().unwrap().into()),
            Some(directory.path().join("missing").to_str().unwrap().into()),
            Some("/invalid\0shell".into()),
            Some(not_executable),
        ] {
            assert_eq!(
                select_unix_shell(environment.clone(), || Some(account.clone())),
                account,
                "environment shell: {environment:?}"
            );
        }
    }

    #[test]
    fn missing_or_invalid_account_shell_falls_back_to_bin_sh() {
        let directory = tempfile::tempdir().unwrap();
        let not_executable = fake_shell(directory.path(), "not-executable", false);
        for account in [
            None,
            Some(String::new()),
            Some("relative-shell".into()),
            Some(directory.path().to_str().unwrap().into()),
            Some(directory.path().join("missing").to_str().unwrap().into()),
            Some("/invalid\0shell".into()),
            Some(not_executable),
        ] {
            assert_eq!(select_unix_shell(None, || account), "/bin/sh");
        }
    }

    #[test]
    fn known_shells_start_login_and_interactive_but_other_shells_keep_only_i() {
        for shell in ["/bin/zsh", "/opt/homebrew/bin/bash", "/usr/local/bin/fish"] {
            let (command, selected) = shell_command_for(shell.into());
            assert_eq!(selected, shell);
            assert_eq!(command.get_argv(), &[shell, "-l", "-i"]);
        }
        for shell in ["/bin/sh", "/bin/dash", "/usr/local/bin/shell-wrapper"] {
            let (command, _) = shell_command_for(shell.into());
            assert_eq!(command.get_argv(), &[shell, "-i"]);
        }
    }

    #[test]
    fn shell_command_preserves_the_environment_and_sets_child_terminal_values() {
        let directory = tempfile::tempdir().unwrap();
        let shell = fake_shell(directory.path(), "shell-wrapper", true);
        let inherited = CommandBuilder::new("/bin/sh");
        let (command, _) = shell_command_for(shell.clone());
        for (key, value) in inherited.iter_full_env_as_str() {
            if !["SHELL", "TERM", "COLORTERM"].contains(&key) {
                assert_eq!(command.get_env(key), Some(OsStr::new(value)), "{key}");
            }
        }
        assert_eq!(command.get_env("SHELL"), Some(OsStr::new(&shell)));
        assert_eq!(command.get_env("TERM"), Some(OsStr::new("xterm-256color")));
        assert_eq!(command.get_env("COLORTERM"), Some(OsStr::new("truecolor")));
        let mut overrides: Vec<_> = command
            .iter_extra_env_as_str()
            .map(|(key, _)| key)
            .collect();
        overrides.sort_unstable();
        assert_eq!(overrides, ["COLORTERM", "SHELL", "TERM"]);
    }

    #[test]
    fn real_pty_shell_fixture_receives_login_args_and_the_selected_shell_environment() {
        let directory = tempfile::tempdir().unwrap();
        let shell = fake_shell(directory.path(), "bash", true);
        // The interpreter is /bin/sh with a script, never the user's real shell/rc files.
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf 'args:%s,%s\\nshell:%s\\nterm:%s,%s\\npath:%s\\nfixture-ready\\n' \"$1\" \"$2\" \"$SHELL\" \"$TERM\" \"$COLORTERM\" \"$PATH\"\ncat\n",
        )
        .unwrap();
        let (command, selected) = shell_command_for(shell.clone());
        let expected_path = command
            .get_env("PATH")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (tx, rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |event| tx.send(event).map_err(|e| e.to_string()));
        let info = state
            .open_command(
                "main".into(),
                directory.path().into(),
                size(80, 24).unwrap(),
                command,
                selected,
                sink,
            )
            .unwrap();
        let bytes = text_until(&rx, "fixture-ready");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("args:-l,-i"), "{text}");
        assert!(text.contains(&format!("shell:{shell}")), "{text}");
        assert!(text.contains("term:xterm-256color,truecolor"), "{text}");
        assert!(text.contains(&format!("path:{expected_path}")), "{text}");
        state.close("main", &info.id).unwrap();
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn shell_command_keeps_windows_arguments_and_does_not_override_shell() {
        let shell = r"C:\Windows\System32\cmd.exe";
        let (command, selected) = shell_command_for(shell.into());
        assert_eq!(selected, shell);
        assert_eq!(command.get_argv(), &[shell, "/D", "/Q"]);
        assert!(!command
            .iter_extra_env_as_str()
            .any(|(name, _)| name == "SHELL"));
    }
}

#[cfg(all(test, unix))]
mod public_api_tests {
    use crate::{OpenedTerminal, TerminalEvent, TerminalState};
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    struct Cleanup(TerminalState);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.shutdown();
        }
    }

    // Shell selection reads process-wide environment. Isolate it in a child
    // test process so parallel tests never race on SHELL or run user rc files.
    fn in_fixture_process(test: &str) -> bool {
        const CHILD: &str = "HUSK_TERMINAL_PUBLIC_API_CHILD";
        if std::env::var(CHILD).as_deref() == Ok(test) {
            return true;
        }
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell-fixture");
        std::fs::write(
            &shell,
            "#!/bin/sh\n\
             stty -echo\n\
             printf 'fixture-ready\\n'\n\
             while IFS= read -r line; do\n\
               case \"$line\" in\n\
                 input) printf 'input-received\\n' ;;\n\
                 size) stty size ;;\n\
                 exit) printf 'final-output\\n'; exit 7 ;;\n\
               esac\n\
             done\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, test)
            .env("SHELL", &shell)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                break true;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let output = child.wait_with_output().unwrap();
        assert!(
            !timed_out && output.status.success(),
            "PTY fixture test failed (timeout: {timed_out}):\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    fn open(
        state: &TerminalState,
        owner: &str,
        cwd: &std::path::Path,
    ) -> (OpenedTerminal, Receiver<TerminalEvent>) {
        let (tx, rx) = mpsc::channel();
        let info = state
            .open(
                owner.into(),
                Some(cwd.to_str().unwrap().into()),
                80,
                24,
                Arc::new(move |event| tx.send(event).map_err(|error| error.to_string())),
            )
            .unwrap();
        (info, rx)
    }

    fn text_until(rx: &Receiver<TerminalEvent>, needle: &str) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        loop {
            match rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
            {
                TerminalEvent::Data { data } => {
                    bytes.extend(data);
                    if String::from_utf8_lossy(&bytes).contains(needle) {
                        return bytes;
                    }
                }
                event => panic!("expected output {needle:?}, received {event:?}"),
            }
        }
    }

    #[test]
    fn public_open_delivers_input_resize_and_drained_exit() {
        if !in_fixture_process(
            "public_api_tests::public_open_delivers_input_resize_and_drained_exit",
        ) {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (info, rx) = open(&state, "owner-a", directory.path());
        assert_eq!(
            info.cwd,
            directory.path().canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(info.shell, std::env::var("SHELL").unwrap());
        assert!(!info.id.is_empty());
        text_until(&rx, "fixture-ready");
        state.write("owner-a", &info.id, "input\n").unwrap();
        text_until(&rx, "input-received");

        state.resize("owner-a", &info.id, 123, 37).unwrap();
        state.write("owner-a", &info.id, "size\n").unwrap();
        text_until(&rx, "37 123");

        state.write("owner-a", &info.id, "exit\n").unwrap();
        let mut bytes = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TerminalEvent::Data { data } => bytes.extend(data),
                TerminalEvent::Exit { code, signal } => {
                    assert_eq!(code, Some(7));
                    assert_eq!(signal, None);
                    break;
                }
                TerminalEvent::Error { message } => panic!("{message}"),
            }
        }
        assert!(String::from_utf8_lossy(&bytes).contains("final-output"));
        state.close("owner-a", &info.id).unwrap();
        assert!(state.write("owner-a", &info.id, "closed").is_err());
        assert!(state.resize("owner-a", &info.id, 80, 24).is_err());
        state.close("owner-a", &info.id).unwrap();
    }

    #[test]
    fn public_owner_close_destroy_and_shutdown_reap_sessions() {
        if !in_fixture_process(
            "public_api_tests::public_owner_close_destroy_and_shutdown_reap_sessions",
        ) {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let state = TerminalState::default();
        let _cleanup = Cleanup(state.clone());
        let (first, first_rx) = open(&state, "owner-a", directory.path());
        text_until(&first_rx, "fixture-ready");
        let (second, second_rx) = open(&state, "owner-b", directory.path());
        text_until(&second_rx, "fixture-ready");
        assert!(state.write("owner-b", &first.id, "input\n").is_err());
        assert!(state.resize("owner-b", &first.id, 80, 24).is_err());
        assert!(state.close("owner-b", &first.id).is_err());
        assert!(state
            .write("owner-a", &first.id, &"x".repeat(64 * 1024 + 1))
            .is_err());
        assert!(state.resize("owner-a", &first.id, 0, 24).is_err());

        state.destroy_owner("owner-a");
        state.close("owner-a", &first.id).unwrap();
        assert!(matches!(
            first_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            TerminalEvent::Exit { .. }
        ));
        assert!(state.write("owner-a", &first.id, "closed").is_err());
        assert!(state
            .open("owner-a".into(), None, 80, 24, Arc::new(|_| Ok(())))
            .is_err());
        state.write("owner-b", &second.id, "input\n").unwrap();
        text_until(&second_rx, "input-received");
        state.close("owner-b", &second.id).unwrap();
        assert!(matches!(
            second_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            TerminalEvent::Exit { .. }
        ));
        state.close("owner-b", &second.id).unwrap();

        let (third, third_rx) = open(&state, "owner-b", directory.path());
        text_until(&third_rx, "fixture-ready");
        assert!(!state.shutdown_complete());
        assert!(state.start_shutdown());
        assert!(!state.start_shutdown());
        assert!(state
            .open("owner-c".into(), None, 80, 24, Arc::new(|_| Ok(())))
            .is_err());
        state.shutdown();
        assert!(state.shutdown_complete());
        assert!(matches!(
            third_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            TerminalEvent::Exit { .. }
        ));
        assert!(state.write("owner-b", &third.id, "closed").is_err());
        state.shutdown();
    }
}
