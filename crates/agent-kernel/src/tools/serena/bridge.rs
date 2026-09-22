//! The MCP stdio bridge — one `serena start-mcp-server` child plus the
//! JSON-RPC request/response plumbing over its stdin/stdout.
//!
//! Everything here is about *one* server process: spawning it, correlating
//! responses by id, noticing when it dies, and keeping enough of its stderr
//! that a failure is diagnosable. Which workspace a bridge belongs to, and
//! when it is replaced, is [`super::manager`]'s job.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};

use super::super::registry::ToolError;

/// `uvx` first-run can take a while (resolve + build + import); the
/// `initialize` handshake waits this long before giving up.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

/// Per-request MCP timeout — Serena's symbol queries can be slow on the first
/// index, so allow more headroom than a file read.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(90);

/// How much of the child's stderr to keep for error messages. A ring buffer,
/// not a log: Serena is chatty on startup and we only ever want the tail.
const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// One tool as the server describes it in `tools/list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SerenaToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Raw JSON Schema — surfaced on demand (`serena_list_tools` for one tool)
    /// rather than dumped for all ~40.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

/// A response slot: the reader resolves it with the server's reply, or with a
/// failure reason when the child dies (so a dead server fails its in-flight
/// requests *now* instead of parking them until the timeout).
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, ToolError>>>>>;

/// What the meta-tool needs from a live server. A trait so the manager's
/// scoping and restart behaviour can be tested without `uvx`, without a
/// network, and without a Python environment.
#[async_trait::async_trait]
pub trait McpBridge: Send + Sync {
    /// `tools/call` — flattened text content.
    async fn call(&self, tool: &str, arguments: Value) -> Result<String, ToolError>;
    /// `tools/list`.
    async fn list_tools(&self) -> Result<Vec<SerenaToolInfo>, ToolError>;
    /// False once the child is gone; the manager must not hand this out again.
    fn is_healthy(&self) -> bool;
    /// Recent stderr for error context (empty when the child never wrote any).
    async fn diagnostics(&self) -> String;
}

/// Spawns one bridge for one workspace root.
#[async_trait::async_trait]
pub trait BridgeFactory: Send + Sync {
    async fn spawn(&self, root: &Path) -> Result<Arc<dyn McpBridge>, ToolError>;
}

/// How to launch Serena. `uvx` by default; overridable so a deployment can pin
/// a release or a fork instead of re-resolving a branch on every cold start
/// (`HUSK_SERENA_SOURCE`), and so tests can point at a stub server
/// (`HUSK_SERENA_CMD`).
#[derive(Debug, Clone)]
pub struct SerenaLaunch {
    pub program: String,
    pub source: String,
    /// Full argv override. Production leaves this `None` and gets
    /// `--from <source> serena start-mcp-server --project <root> …`; tests
    /// point it at a stub MCP server instead of `uvx`.
    pub argv_override: Option<Vec<String>>,
}

impl SerenaLaunch {
    pub fn from_env() -> Self {
        Self {
            program: std::env::var("HUSK_SERENA_CMD").unwrap_or_else(|_| "uvx".into()),
            // Default stays the branch tip: pinning is opt-in, because the
            // tool surface (names/schemas) changes between Serena releases.
            source: std::env::var("HUSK_SERENA_SOURCE")
                .unwrap_or_else(|_| "git+https://github.com/oraios/serena".into()),
            argv_override: None,
        }
    }

    fn command(&self, root: &Path) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.program);
        match &self.argv_override {
            Some(argv) => cmd.args(argv),
            None => cmd.args([
                "--from",
                &self.source,
                "serena",
                "start-mcp-server",
                "--transport",
                "stdio",
                "--project",
                &root.to_string_lossy(),
                "--context",
                "ide",
            ]),
        };
        cmd
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Piped, never null: a startup failure with a nulled stderr is
        // undiagnosable (see `diagnostics`).
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        cmd
    }
}

pub struct SerenaBridge {
    launch: SerenaLaunch,
    /// Root the server was scoped to — for diagnostics and error messages.
    root: std::path::PathBuf,
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    req_id: AtomicU64,
    _child: Mutex<Child>,
    /// Set by the stdout reader when the pipe closes — the single source of
    /// truth for "this bridge is dead". `Arc` because the reader task owns a
    /// clone of it.
    dead: Arc<AtomicBool>,
    stderr_tail: Arc<Mutex<VecDeque<u8>>>,
}

impl SerenaBridge {
    /// Spawn the server scoped to `root`, pump its streams, then run the
    /// `initialize` handshake.
    pub async fn spawn(root: &Path) -> Result<Arc<Self>, ToolError> {
        Self::spawn_with(root, SerenaLaunch::from_env()).await
    }

    pub async fn spawn_with(root: &Path, launch: SerenaLaunch) -> Result<Arc<Self>, ToolError> {
        let mut child = launch
            .command(root)
            .spawn()
            .map_err(|e| ToolError::Failed(format!("spawn serena ({} missing?): {e}", launch.program)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::Failed("serena child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::Failed("serena child has no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::Failed("serena child has no stderr".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let stderr_tail: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let bridge = Arc::new(Self {
            launch,
            root: root.to_path_buf(),
            stdin: Mutex::new(stdin),
            pending: pending.clone(),
            req_id: AtomicU64::new(1),
            _child: Mutex::new(child),
            dead: Arc::new(AtomicBool::new(false)),
            stderr_tail: stderr_tail.clone(),
        });

        // stderr → ring buffer. Chunked, not line-based: uv/pip progress and a
        // Python traceback both arrive without useful line structure.
        let stderr_ring = stderr_tail.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buf = [0u8; 4096];
            let tail = stderr_ring;
            while let Ok(n) = reader.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut ring = tail.lock().await;
                ring.extend(&buf[..n]);
                while ring.len() > STDERR_TAIL_BYTES {
                    ring.pop_front();
                }
            }
        });

        // stdout → pending. On EOF the child is gone: mark the bridge dead and
        // fail every in-flight request *with a reason* instead of letting them
        // sit until the 90s timeout.
        {
            let pending = pending.clone();
            let dead = bridge.dead.clone();
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                        if let Some(tx) = pending.lock().await.remove(&id) {
                            let _ = tx.send(Ok(v));
                        }
                    }
                }
                dead.store(true, Ordering::Relaxed);
                let tail_text = String::from_utf8_lossy(
                    &tail.lock().await.iter().copied().collect::<Vec<u8>>(),
                )
                .trim()
                .to_string();
                let reason = if tail_text.is_empty() {
                    "serena server exited before answering".to_string()
                } else {
                    format!("serena server exited before answering — stderr tail:\n{tail_text}")
                };
                for (_, tx) in pending.lock().await.drain() {
                    let _ = tx.send(Err(ToolError::Failed(reason.clone())));
                }
            });
        }

        match bridge.initialize().await {
            Ok(()) => Ok(bridge),
            Err(e) => Err(ToolError::Failed(format!(
                "serena did not come up ({} --from {}): {e}{}",
                bridge.launch.program,
                bridge.launch.source,
                bridge.stderr_suffix().await
            ))),
        }
    }

    async fn stderr_suffix(&self) -> String {
        let tail = self.diagnostics().await;
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!("\nserena stderr (tail):\n{tail}")
        }
    }

    /// `initialize` + `notifications/initialized` handshake.
    async fn initialize(&self) -> Result<(), ToolError> {
        let init = json!({
            "jsonrpc":"2.0","id":0,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{},
                      "clientInfo":{"name":"husk","version":"0.1"}}
        });
        self.request(init, STARTUP_TIMEOUT).await?;
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        self.send(&note).await
    }

    /// Write one JSON-RPC line to the child's stdin.
    async fn send(&self, body: &Value) -> Result<(), ToolError> {
        if !self.is_healthy() {
            return Err(ToolError::Failed(format!(
                "serena server for {} is gone",
                self.root.display()
            )));
        }
        let mut line = serde_json::to_vec(body)
            .map_err(|e| ToolError::Failed(format!("serena encode: {e}")))?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|e| ToolError::Failed(format!("serena stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ToolError::Failed(format!("serena flush: {e}")))
    }

    /// Send a request and wait for its reply.
    ///
    /// The pending slot is removed on **every** exit path — a send failure, a
    /// timeout, or a reply — because the map outlives the call: a leaked entry
    /// is a `Sender` nobody will ever resolve.
    async fn request(&self, body: Value, timeout: Duration) -> Result<Value, ToolError> {
        let id = body.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.send(&body).await {
            self.pending.lock().await.remove(&id);
            return Err(self.enrich(e).await);
        }

        let reply = match tokio::time::timeout(timeout, rx).await {
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(ToolError::Failed(format!(
                    "serena request timed out after {}s",
                    timeout.as_secs()
                )))
            }
            Ok(Err(_)) => Err(ToolError::Failed("serena response channel closed".into())),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Ok(Ok(v))) => Ok(v),
        };
        let v = match reply {
            Ok(v) => v,
            Err(e) => return Err(self.enrich(e).await),
        };
        if let Some(err) = v.get("error") {
            return Err(ToolError::Failed(format!("serena rpc: {err}")));
        }
        Ok(v.get("result").cloned().unwrap_or(v))
    }

    /// Append the stderr tail to a failure — the only window into what the
    /// server process actually said.
    async fn enrich(&self, e: ToolError) -> ToolError {
        ToolError::Failed(format!("{e}{}", self.stderr_suffix().await))
    }
}

#[async_trait::async_trait]
impl McpBridge for SerenaBridge {
    async fn call(&self, tool: &str, arguments: Value) -> Result<String, ToolError> {
        let id = self.req_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":tool,"arguments":arguments}
        });
        let result = self.request(body, CALL_TIMEOUT).await?;
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            let text = content
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Ok(text);
            }
        }
        Ok(result.to_string())
    }

    async fn list_tools(&self) -> Result<Vec<SerenaToolInfo>, ToolError> {
        let id = self.req_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":{}});
        let result = self.request(body, CALL_TIMEOUT).await?;
        Ok(result
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| serde_json::from_value::<SerenaToolInfo>(t.clone()).ok())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn is_healthy(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }

    async fn diagnostics(&self) -> String {
        let ring = self.stderr_tail.lock().await;
        let bytes: Vec<u8> = ring.iter().copied().collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests that steer the stub through the process env: the
    /// stub mode is per *process*, so two of them must not overlap.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        GUARD.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Full path libtest needs to run exactly this test in the child.
    const STUB_TEST: &str = "tools::serena::bridge::tests::stub_mcp_server";
    const STUB_MODE: &str = "HUSK_SERENA_STUB";

    /// Launch the current test binary as a stub MCP server. The stub is the
    /// same executable re-entered with `--exact <stub test>` — no `uvx`, no
    /// network, no Python, and it exercises the *real* reader/writer plumbing
    /// (pending map, death detection, stderr capture).
    fn stub_launch() -> SerenaLaunch {
        SerenaLaunch {
            program: std::env::current_exe().expect("test binary path").display().to_string(),
            source: String::new(),
            argv_override: Some(vec![
                "--exact".into(),
                STUB_TEST.into(),
                "--nocapture".into(),
            ]),
        }
    }

    /// The stub server. Only active in the re-executed child; in the parent
    /// run it is a no-op test.
    ///
    /// Both conditions matter: the env var is process-global and libtest runs
    /// tests in parallel, so in the parent another test's `set_var` could be
    /// visible here — and then this test would read the *parent's* stdin and
    /// hang (or `exit` the parent's test binary mid-run). The `--exact` argv is
    /// only ever present in the child.
    #[test]
    fn stub_mcp_server() {
        let reexecuted = std::env::args().any(|a| a == "--exact");
        if std::env::var(STUB_MODE).is_err() || !reexecuted {
            return;
        }
        stub_main();
        std::process::exit(0);
    }

    fn stub_main() {
        use std::io::{BufRead, Write};
        let mode = std::env::var(STUB_MODE).unwrap_or_default();
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { return };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let id = v.get("id").cloned();
            let reply = |result: Value| {
                let mut out = std::io::stdout();
                let _ = writeln!(
                    out,
                    "{}",
                    json!({"jsonrpc":"2.0","id":id,"result":result})
                );
                let _ = out.flush();
            };
            match method {
                "initialize" => reply(json!({"protocolVersion":"2024-11-05","capabilities":{}})),
                "tools/list" => reply(json!({"tools":[{
                    "name":"find_symbol",
                    "description":"Find symbols.\nMore detail.",
                    "inputSchema":{"type":"object","properties":{"name_path":{"type":"string"}},"required":["name_path"]}
                }]})),
                "tools/call" => match mode.as_str() {
                    // Exit without answering — the "serena died mid-call" case.
                    "die-on-call" => {
                        eprintln!("serena stub: fatal: index corrupt");
                        return;
                    }
                    // Never answer — drives the timeout path.
                    "silent" => {}
                    _ => reply(json!({"content":[{"type":"text","text":"stub-ok"}]})),
                },
                _ => {}
            }
        }
    }

    async fn stub_session(
        mode: &str,
        root: &Path,
    ) -> (Arc<SerenaBridge>, std::sync::MutexGuard<'static, ()>) {
        let guard = env_guard();
        std::env::set_var(STUB_MODE, mode);
        // `.await`, never `block_on`: the bridge's reader tasks live on this
        // runtime, and blocking the current-thread runtime here would stall the
        // very task that answers `initialize`.
        let bridge = SerenaBridge::spawn_with(root, stub_launch())
            .await
            .expect("stub server should come up");
        (bridge, guard)
    }

    #[tokio::test]
    async fn initialize_call_and_list_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, _guard) = stub_session("ok", dir.path()).await;
        assert!(bridge.is_healthy());
        assert_eq!(bridge.call("find_symbol", json!({"name_path":"x"})).await.unwrap(), "stub-ok");

        let tools = bridge.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "find_symbol");
        assert!(tools[0].input_schema.as_ref().unwrap().get("properties").is_some());
    }

    /// A child that exits mid-request must fail that request with the reason
    /// (and the stderr tail) *immediately* — the old reader loop just ended,
    /// leaving the caller parked until the 90s timeout and the dead bridge
    /// cached forever.
    #[tokio::test]
    async fn dying_child_fails_in_flight_calls_with_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, _guard) = stub_session("die-on-call", dir.path()).await;

        let err = tokio::time::timeout(
            Duration::from_secs(10),
            bridge.call("replace_symbol_body", json!({})),
        )
        .await
        .expect("a dead child must not park the caller until CALL_TIMEOUT")
        .expect_err("the call cannot succeed");
        let msg = format!("{err}");
        assert!(msg.contains("exited"), "{msg}");
        assert!(msg.contains("index corrupt"), "stderr tail missing: {msg}");
        assert!(!bridge.is_healthy(), "death must mark the bridge unhealthy");
        assert!(bridge.diagnostics().await.contains("index corrupt"));
    }

    /// Timeouts and send failures must not leave entries in `pending` — the
    /// map outlives the call, so a leaked slot is a sender nobody resolves.
    #[tokio::test]
    async fn failures_leave_no_pending_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (bridge, _guard) = stub_session("silent", dir.path()).await;

        // Timeout path.
        let body = json!({"jsonrpc":"2.0","id":77,"method":"tools/call","params":{}});
        let err = bridge
            .request(body, Duration::from_millis(150))
            .await
            .expect_err("no reply must time out");
        assert!(format!("{err}").contains("timed out"), "{err}");
        assert!(bridge.pending.lock().await.is_empty(), "timeout leaked a pending slot");

        // Send-failure path: the stub is still alive, so kill it by closing
        // the pipe — `send` then errors before a reply can arrive.
        bridge.dead.store(true, Ordering::Relaxed);
        let body = json!({"jsonrpc":"2.0","id":78,"method":"tools/call","params":{}});
        bridge.request(body, Duration::from_millis(150)).await.expect_err("dead bridge");
        assert!(bridge.pending.lock().await.is_empty(), "send failure leaked a pending slot");
    }

    #[tokio::test]
    async fn spawn_reports_a_missing_program_with_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let launch = SerenaLaunch {
            program: "/nonexistent/uvx-for-husk-test".into(),
            source: "git+https://example.invalid/serena".into(),
            argv_override: None,
        };
        let err = SerenaBridge::spawn_with(dir.path(), launch)
            .await
            .err()
            .expect("a missing uvx must surface, not panic");
        let msg = format!("{err}");
        assert!(msg.contains("spawn serena"), "{msg}");
        assert!(msg.contains("uvx-for-husk-test"), "{msg}");
    }
}
