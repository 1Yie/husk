//! `serena` — bridges Serena's semantic code tools into a native tool.
//!
//! Mirrors pi-serena-tools: spawn a per-session `serena start-mcp-server`
//! scoped to the workspace, then forward `tools/call` over the child's
//! stdio (newline-delimited JSON-RPC — the MCP stdio transport, so the
//! kernel keeps its "no reqwest" constraint). The agent calls Serena's
//! `find_symbol` / `read_file` / `replace_symbol_body` … through one
//! `serena` meta-tool — IDE-level symbol editing, not text grep.
//!
//! Graceful degradation: `uvx`/serena missing or the server not coming up
//! simply means the tool errors on use — everything else still works.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex, OnceCell};

use super::registry::{schema_for, ExecFn, ToolError, ToolResult, ToolSpec};

/// `uvx` first-run can take a while (build + dep download); the
/// `initialize` handshake waits this long before giving up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

/// Per-request MCP timeout — Serena's symbol queries can be slow on first
/// index, so allow more headroom than a file read.
const CALL_TIMEOUT: Duration = Duration::from_secs(90);

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// The spawned `serena start-mcp-server --transport stdio` child plus the
/// MCP request/response plumbing over its stdin/stdout.
pub struct SerenaBridge {
    stdin: Mutex<ChildStdin>,
    /// Pending request id → the one-shot the reader task resolves.
    pending: Pending,
    req_id: AtomicU64,
    _child: Mutex<Child>,
}

impl SerenaBridge {
    /// Spawn `uvx … serena start-mcp-server --transport stdio` scoped to
    /// `root`, pump stdout into `pending`, then `initialize` +
    /// `notifications/initialized`.
    pub async fn spawn(root: &std::path::Path) -> Result<Arc<Self>, ToolError> {
        let mut child = tokio::process::Command::new("uvx")
            .args([
                "--from",
                "git+https://github.com/oraios/serena",
                "serena",
                "start-mcp-server",
                "--transport",
                "stdio",
                "--project",
                &root.to_string_lossy(),
                "--context",
                "ide",
            ])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ToolError::Failed(format!("spawn serena (uvx missing?): {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::Failed("serena child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::Failed("serena child has no stdout".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let bridge = Arc::new(Self {
            stdin: Mutex::new(stdin),
            pending: pending.clone(),
            req_id: AtomicU64::new(1),
            _child: Mutex::new(child),
        });

        // Reader task — every stdout line is one JSON-RPC message; match it
        // to a pending request by `id` and resolve the one-shot.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let _ = tx.send(v);
                    }
                }
            }
        });

        bridge.initialize().await?;
        Ok(bridge)
    }

    /// `initialize` + `notifications/initialized` handshake.
    async fn initialize(&self) -> Result<(), ToolError> {
        let init = json!({
            "jsonrpc":"2.0","id":0,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{},
                      "clientInfo":{"name":"agent-rs","version":"0.1"}}
        });
        self.request(init, STARTUP_TIMEOUT).await?;
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        self.send(&note).await
    }

    /// Write one JSON-RPC line to the child's stdin.
    async fn send(&self, body: &Value) -> Result<(), ToolError> {
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

    /// Send a request, register a pending response, wait for the reader
    /// task to resolve it (with timeout).
    async fn request(&self, body: Value, timeout: Duration) -> Result<Value, ToolError> {
        let id = body.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.send(&body).await?;
        let v = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| ToolError::Failed("serena request timed out".into()))?
            .map_err(|_| ToolError::Failed("serena response channel closed".into()))?;
        if let Some(err) = v.get("error") {
            return Err(ToolError::Failed(format!("serena rpc: {err}")));
        }
        Ok(v.get("result").cloned().unwrap_or(v))
    }

    /// `tools/call` — returns the flattened text content.
    pub async fn call(&self, tool: &str, arguments: Value) -> Result<String, ToolError> {
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

    /// `tools/list` — the server's tool names for `serena_list_tools`.
    pub async fn list_tools(&self) -> Result<Vec<String>, ToolError> {
        let id = self.req_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":{}});
        let result = self.request(body, CALL_TIMEOUT).await?;
        Ok(result
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|ts| {
                ts.iter()
                    .filter_map(|t| t.get("name")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// `serena` meta-tool args — call one Serena tool by name.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SerenaCallArgs {
    /// The Serena tool name (e.g. `find_symbol`, `read_file`,
    /// `replace_symbol_body`, `search_for_pattern`, `list_dir`).
    tool: String,
    /// The tool's arguments as a JSON object.
    arguments: Option<serde_json::Value>,
}

/// The shared bridge — spawned lazily on the first `serena` call and
/// cached for the process, so sessions that never use Serena never pay
/// the startup.
fn shared_bridge() -> &'static OnceCell<Option<Arc<SerenaBridge>>> {
    static CELL: OnceCell<Option<Arc<SerenaBridge>>> = OnceCell::const_new();
    &CELL
}

/// Register the `serena` meta-tool — one passthrough that calls any Serena
/// tool by name. Registering ~40 Serena tools 1:1 would bloat the `tools`
/// schema; a meta-tool keeps the surface small while exposing the whole
/// suite (`tool: "serena_list_tools"` to discover them).
pub fn spec() -> ToolSpec {
    let exec: ExecFn = Arc::new(|args, ctx| {
        let a: SerenaCallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return async move { Err(ToolError::Args(format!("serena args: {e}"))) }.boxed()
            }
        };
        let root = ctx.workspace_root.clone();
        async move {
            let bridge = shared_bridge()
                .get_or_init(|| async { SerenaBridge::spawn(&root).await.ok() })
                .await;
            let Some(bridge) = bridge else {
                return Err(ToolError::Failed(
                    "serena unavailable — `uvx` missing or the MCP server \
                     failed to start (needs `uvx` on PATH + network for the \
                     first `uvx --from git+…` resolve)"
                        .into(),
                ));
            };
            // `serena_list_tools` is a bridge method, not a Serena tool —
            // intercept it here.
            if a.tool == "serena_list_tools" || a.tool == "list_tools" {
                let tools = bridge.list_tools().await?;
                let text = tools
                    .iter()
                    .map(|n| format!("• {n}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(ToolResult::text(if text.is_empty() {
                    "No serena tools.".into()
                } else {
                    text
                }));
            }
            let arguments = a.arguments.unwrap_or(json!({}));
            let out = bridge.call(&a.tool, arguments).await?;
            Ok(ToolResult::text(out))
        }
        .boxed()
    });

    ToolSpec {
        name: "serena",
        schema: schema_for::<SerenaCallArgs>(
            "Call a Serena semantic code tool (IDE-level symbol/file ops —\n\
             find_symbol, read_file, replace_symbol_body, rename_symbol,\n\
             search_for_pattern, list_dir, …). Pass `tool` = the Serena tool\n\
             name and `arguments` = its JSON args. Call with\n\
             `tool: \"serena_list_tools\"` to discover the available tools.",
        ),
        // Serena file tools write — treat the suite as non-readonly so the
        // permission gate applies.
        readonly: false,
        exec,
    }
}
