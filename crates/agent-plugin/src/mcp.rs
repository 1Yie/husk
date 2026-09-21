//! `mcp.rs` — full MCP JSON-RPC 2.0 client over stdio or streamable HTTP.
//!
//! Transport is picked per manifest `entry`: `{command, args}` spawns a
//! child process (newline-framed stdio), `{url}` POSTs JSON-RPC to a
//! streamable-HTTP endpoint (plain-JSON or SSE response, `Mcp-Session-Id`
//! carried across calls).
//!
//! Contract (plugin-system.md §MCP bridge): protocol `2024-11-05`, the
//! **complete** lifecycle — anything less fails real community servers:
//!
//! ```text
//! spawn → initialize → notifications/initialized
//!   → tools/list → (steady) tools/call + resources/prompts per caps
//!   → notifications/tools/list_changed → re-list → shutdown: drain + kill
//! ```
//!
//! Demultiplexing: `id` → resolve pending oneshot; `method` → notification
//! handler (`tools/list_changed` → re-list, `cancelled`/`progress` → step
//! card). Per-call 30 s timeout. Non-text `content[]` → placeholder marker.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};

use crate::manifest::PluginManifest;

const PROTOCOL_VERSION: &str = "2024-11-05";
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Restarts allowed on transport failure (never mid-request) — Stage 9.5
/// wires the backoff loop; kept as the documented cap constant.
#[allow(dead_code)]
const MAX_RESTARTS: u8 = 3;

/// Server capabilities declared at `initialize`.
#[derive(Debug, Default, Clone)]
pub struct ServerCaps {
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
}

/// How the client reaches its server — picked by the manifest `entry`.
enum Transport {
    /// `{command, args}` — child process, newline-framed stdio.
    Stdio {
        stdin: Arc<Mutex<ChildStdin>>,
        /// Held for kill-on-drop; never touched directly.
        _child: Mutex<Child>,
    },
    /// `{url}` — streamable HTTP: each JSON-RPC message is one POST; the
    /// reply comes back in the POST response body (plain JSON or an SSE
    /// stream we scan for our request id).
    Http {
        url: String,
        client: reqwest::Client,
        session_id: Mutex<Option<String>>,
        headers: HashMap<String, String>,
    },
}

/// One live MCP server: spawned child + demux reader + tool schema cache.
pub struct McpClient {
    pub plugin_id: String,
    transport: Transport,
    req_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    pub caps: std::sync::Mutex<ServerCaps>,
    /// tool name → JSON schema (from `tools/list`).
    pub tools: Mutex<Vec<Value>>,
    /// prompt name → description (from `prompts/list`, when declared).
    pub prompts: Mutex<Vec<Value>>,
    /// resource uri → metadata (from `resources/list`, when declared).
    pub resources: Mutex<Vec<Value>>,
    /// stderr ring buffer (surfaced on error cards) — stdio only.

    pub stderr_log: Arc<Mutex<Vec<String>>>,
}

impl McpClient {
    /// Spawn + handshake + initial `tools/list` (+ resources/prompts when
    /// the server declares them).
    pub async fn start(manifest: &PluginManifest) -> Result<Arc<Self>> {
        let entry = &manifest.entry;

        // HTTP transport — `entry.url` means streamable HTTP instead of a
        // spawned child. No env/stdin machinery applies.
        if let Some(url) = entry["url"].as_str() {
            let headers: HashMap<String, String> = entry["headers"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), resolve_indirect(v.as_str().unwrap_or(""), "header")))
                        .collect()
                })
                .unwrap_or_default();
            let client = Arc::new(Self {
                plugin_id: manifest.id.clone(),
                transport: Transport::Http {
                    url: url.to_string(),
                    client: reqwest::Client::builder()
                        .timeout(CALL_TIMEOUT)
                        .build()
                        .context("reqwest build")?,
                    session_id: Mutex::new(None),
                    headers,
                },
                req_id: AtomicU64::new(1),
                pending: Arc::new(Mutex::new(HashMap::new())),
                caps: std::sync::Mutex::new(ServerCaps::default()),
                tools: Mutex::new(Vec::new()),
                prompts: Mutex::new(Vec::new()),
                resources: Mutex::new(Vec::new()),
                stderr_log: Arc::new(Mutex::new(Vec::new())),
            });
            return client.handshake().await;
        }

        let command = entry["command"].as_str().context("mcp entry.command")?;
        let args: Vec<String> = entry["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        // Child env = manifest env only — never inherit host env.
        // `env:VAR` indirection resolves from host env, but a manifest can
        // name ANY host variable — including `AWS_SECRET_ACCESS_KEY` — and
        // exfiltrate it into the plugin process. Gate that: secret-shaped
        // host vars are refused (empty value + warn) unless the variable
        // name is on a small allowlist of obviously-non-secret handles.
        let env: HashMap<String, String> = entry["env"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        (k.clone(), resolve_indirect(v.as_str().unwrap_or(""), "env"))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let stderr_log = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args)
            .env_clear()
            .envs(&env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Linux: PR_SET_PDEATHSIG — an MCP node server can't outlive the host.
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn MCP `{command} {}`", args.join(" ")))?;

        let stdin = child.stdin.take().context("child stdin")?;
        let stdout = child.stdout.take().context("child stdout")?;
        let mut stderr = child.stderr.take().context("child stderr")?;

        // stderr → ring buffer.
        let log = stderr_log.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(&mut stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut g = log.lock().await;
                g.push(l);
                if g.len() > 200 {
                    g.remove(0);
                }
            }
        });

        let client = Arc::new(Self {
            plugin_id: manifest.id.clone(),
            transport: Transport::Stdio {
                stdin: Arc::new(Mutex::new(stdin)),
                _child: Mutex::new(child),
            },
            req_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            caps: std::sync::Mutex::new(ServerCaps::default()),
            tools: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
            resources: Mutex::new(Vec::new()),
            stderr_log,
        });

        // ---- demux reader task ----
        {
            let pending = client.pending.clone();
            let client_weak = Arc::downgrade(&client);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if v.get("id").is_some() && (v.get("result").is_some() || v.get("error").is_some()) {
                        // Response → resolve the pending oneshot.
                        let id = v["id"].as_u64().unwrap_or(0);
                        let tx = pending.lock().await.remove(&id);
                        if let Some(tx) = tx {
                            let res = if let Some(err) = v.get("error") {
                                Err(err["message"].as_str().unwrap_or("rpc error").to_string())
                            } else {
                                Ok(v["result"].clone())
                            };
                            let _ = tx.send(res);
                        }
                    } else if v.get("method").is_some() {
                        // Notification → internal handler.
                        if let Some(c) = client_weak.upgrade() {
                            c.handle_notification(&v).await;
                        }
                    }
                }
            });
        }

        client.handshake().await
    }

    /// initialize → initialized → list — shared by both transports (the
    /// wire differs, the sequence doesn't).
    async fn handshake(self: &Arc<Self>) -> Result<Arc<Self>> {
        let init = self
            .call_rpc("initialize", json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "husk", "version": env!("CARGO_PKG_VERSION")},
            }))
            .await
            .context("MCP initialize")?;

        // Capture declared caps — gate every later call on them.
        let caps = init.get("capabilities").cloned().unwrap_or_default();
        *self.caps.lock().unwrap() = ServerCaps {
            tools: caps.get("tools").is_some(),
            resources: caps.get("resources").is_some(),
            prompts: caps.get("prompts").is_some(),
        };
        self.notify("notifications/initialized", json!({})).await?;
        self.refresh_lists().await?;

        info!(plugin = %self.plugin_id, "MCP ready");
        Ok(self.clone())
    }

    /// Re-pull `tools/list` + `resources/list` + `prompts/list` per caps.
    async fn refresh_lists(&self) -> Result<()> {
        let caps = self.caps.lock().unwrap().clone();
        if caps.tools {
            let r = self.call_rpc("tools/list", json!({})).await?;
            let tools = r["tools"].as_array().cloned().unwrap_or_default();
            *self.tools.lock().await = tools;
        }
        if caps.resources {
            let r = self.call_rpc("resources/list", json!({})).await?;
            *self.resources.lock().await =
                r["resources"].as_array().cloned().unwrap_or_default();
        }
        if caps.prompts {
            let r = self.call_rpc("prompts/list", json!({})).await?;
            *self.prompts.lock().await =
                r["prompts"].as_array().cloned().unwrap_or_default();
        }
        Ok(())
    }

    async fn handle_notification(&self, v: &Value) {
        match v["method"].as_str() {
            Some("notifications/tools/list_changed") => {
                // Hot-reload: re-list without restart.
                let _ = self.refresh_lists().await;
            }
            Some("notifications/cancelled") | Some("notifications/progress") => {
                // Forward to the step card — Stage 9.5 wires UiEvent.
            }
            _ => {}
        }
    }

    /// JSON-RPC call — allocate id → oneshot → write line → await ≤30 s
    /// (stdio) or POST and read the response body (HTTP).
    pub async fn call_rpc(&self, method: &str, params: Value) -> Result<Value> {
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let id = self.req_id.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = oneshot::channel();
                self.pending.lock().await.insert(id, tx);

                let line = serde_json::to_string(&json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params,
                }))?;
                {
                    let mut stdin = stdin.lock().await;
                    stdin.write_all(line.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await?;
                }

                tokio::time::timeout(CALL_TIMEOUT, rx)
                    .await
                    .context("MCP call timeout")??
                    .map_err(|e| anyhow::anyhow!(e))
            }
            Transport::Http {
                url,
                client,
                session_id,
                headers,
            } => {
                let id = self.req_id.fetch_add(1, Ordering::SeqCst);
                let body = json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params,
                });
                let msg = self
                    .http_roundtrip(client, url, session_id, headers, &body, Some(id))
                    .await?;
                // Same demux contract as stdio: result or error.message.
                if let Some(err) = msg.get("error") {
                    return Err(anyhow::anyhow!(
                        err["message"].as_str().unwrap_or("rpc error").to_string()
                    ));
                }
                Ok(msg.get("result").cloned().unwrap_or(Value::Null))
            }
        }
    }

    /// One HTTP POST — shared by `call_rpc` (expects a response for `id`)
    /// and `notify` (fire-and-forget). Handles `Mcp-Session-Id` capture,
    /// static headers, and both reply shapes (plain JSON / SSE stream).
    async fn http_roundtrip(
        &self,
        client: &reqwest::Client,
        url: &str,
        session_id: &Mutex<Option<String>>,
        headers: &HashMap<String, String>,
        body: &Value,
        want_id: Option<u64>,
    ) -> Result<Value> {
        let mut req = client
            .post(url)
            .header("Accept", "application/json, text/event-stream")
            .json(body);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(sid) = session_id.lock().await.clone() {
            req = req.header("Mcp-Session-Id", sid);
        }
        let resp = req.send().await.context("MCP http send")?;
        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *session_id.lock().await = Some(sid.to_string());
        }
        let status = resp.status();
        // 202/204 = accepted, no response body (the notification path).
        if status.as_u16() == 202 || status.as_u16() == 204 {
            return Ok(Value::Null);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("MCP http {status}: {text}"));
        }
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await?;
        if ctype.contains("text/event-stream") {
            // SSE — scan `data:` lines for the JSON-RPC message with our id.
            for line in text.lines() {
                let Some(d) = line.strip_prefix("data:") else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(d.trim()) else {
                    continue;
                };
                if want_id.is_none() || v["id"].as_u64() == want_id {
                    return Ok(v);
                }
            }
            return Err(anyhow::anyhow!("MCP SSE stream had no matching response"));
        }
        let v: Value = serde_json::from_str(text.trim())
            .context("MCP http response parse")?;
        Ok(v)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        }))?;
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let mut stdin = stdin.lock().await;
                stdin.write_all(line.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
            }
            Transport::Http {
                url,
                client,
                session_id,
                headers,
            } => {
                let body: Value = serde_json::from_str(&line)?;
                // Notification — server replies 202/204 with no body.
                self.http_roundtrip(client, url, session_id, headers, &body, None)
                    .await?;
            }
        }
        Ok(())
    }

    /// `tools/call` — concatenate `type=="text"` content; mark non-text.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        let r = self
            .call_rpc("tools/call", json!({"name": name, "arguments": args}))
            .await?;
        let mut out = String::new();
        for item in r["content"].as_array().into_iter().flatten() {
            match item["type"].as_str() {
                Some("text") => out.push_str(item["text"].as_str().unwrap_or("")),
                Some(t) => out.push_str(&format!("[mcp: {t} content omitted]")),
                _ => {}
            }
            out.push('\n');
        }
        if r["isError"].as_bool().unwrap_or(false) {
            warn!(tool = name, "MCP tool error");
        }
        Ok(out)
    }

    /// `resources/read` — text resource content.
    pub async fn read_resource(&self, uri: &str) -> Result<String> {
        let r = self.call_rpc("resources/read", json!({"uri": uri})).await?;
        let mut out = String::new();
        for item in r["contents"].as_array().into_iter().flatten() {
            if let Some(t) = item["text"].as_str() {
                out.push_str(t);
            } else {
                out.push_str("[mcp: non-text resource omitted]");
            }
        }
        Ok(out)
    }

    /// `prompts/get` — render a prompt template.
    pub async fn get_prompt(&self, name: &str, args: Value) -> Result<String> {
        let r = self
            .call_rpc("prompts/get", json!({"name": name, "arguments": args}))
            .await?;
        let mut out = String::new();
        for m in r["messages"].as_array().into_iter().flatten() {
            if let Some(t) = m["content"]["text"].as_str() {
                out.push_str(t);
                out.push('\n');
            }
        }
        Ok(out)
    }
}

/// Whether a host env-var name looks like credential material — used to
/// gate the `env:VAR` indirection so a plugin manifest can't exfiltrate
/// host secrets into its child process. Mirrors the same denylist shape as
/// the sandbox env-sanitizer: suffix/prefix match, not substring (a var
/// literally *named* `API_KEY` or ending `_TOKEN` is refused; `NOTES_KEY`
/// would also match — erring toward refuse is the safe side).
/// `env:VAR` indirection → resolve from the host environment, refusing
/// secret-shaped names (same policy as manifest env vars). Plain values
/// pass through untouched.
fn resolve_indirect(val: &str, kind: &str) -> String {
    match val.strip_prefix("env:") {
        Some(name) => {
            if host_env_is_secret(name) {
                tracing::warn!(
                    host_var = %name,
                    "MCP {kind} indirection to a secret-shaped host variable \
                     refused (would exfiltrate credentials into the plugin)"
                );
                String::new()
            } else {
                std::env::var(name).unwrap_or_default()
            }
        }
        None => val.to_string(),
    }
}

fn host_env_is_secret(name: &str) -> bool {
    let u = name.to_uppercase();
    const SECRET_SUFFIXES: &[&str] = &[
        "_KEY", "_SECRET", "_TOKEN", "_PASSWORD", "_PASSWD", "_CREDENTIAL",
        "_CREDENTIALS", "_AUTH", "_PRIVATE", "_CERT", "_PEM",
    ];
    const SECRET_EXACT: &[&str] = &[
        "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "GITHUB_TOKEN",
        "GH_TOKEN", "GITLAB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY",
        "XAI_API_KEY", "DEEPSEEK_API_KEY", "GOOGLE_API_KEY", "GEMINI_API_KEY",
        "NPM_TOKEN", "CARGO_REGISTRY_TOKEN", "DOCKER_PASSWORD",
        "KUBECONFIG", "PRIVATE_KEY", "SECRET_KEY",
    ];
    SECRET_EXACT.contains(&u.as_str())
        || SECRET_SUFFIXES.iter().any(|s| u.ends_with(s))
}
