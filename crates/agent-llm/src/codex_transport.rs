//! Codex Responses JSON-over-WebSocket transport.
//!
//! Requests enter *after* the Responses wire patches. Incoming JSON events are
//! adapted to the existing internal SSE decoder; SSE bytes are never sent over
//! the websocket. A provider belongs to one conversation, with one in-flight
//! turn per connection. Dropping a partial stream drops its socket and chain.
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use rig_core::http_client::{self, HeaderMap, HeaderValue, Request, Response};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_tungstenite::{
    tungstenite::{
        handshake::{client::generate_key, derive_accept_key},
        protocol::Role,
        Message,
    },
    WebSocketStream,
};

type Socket = WebSocketStream<reqwest::Upgraded>;
const MAX_CONNECTION_AGE: Duration = Duration::from_secs(55 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REUSE_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const BETA: &str = "responses_websockets=2026-02-06";

/// Codex uses stateless Responses and requires a top-level instruction. Keep
/// explicit sampling overrides and all native output items intact. Unlike a
/// transport switch on the ordinary API, these defaults are opt-in by API kind.
pub(crate) fn patch_request(body: &mut Value) {
    let Some(body) = body.as_object_mut() else {
        return;
    };
    body.insert("store".into(), false.into());
    let needs_instructions = body
        .get("instructions")
        .is_none_or(|v| v.is_null() || v.as_str() == Some(""));
    if needs_instructions {
        let mut instructions = Vec::new();
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input.iter() {
                if !matches!(
                    item.get("role").and_then(Value::as_str),
                    Some("system" | "developer")
                ) {
                    break;
                }
                let text = if let Some(text) = item["content"].as_str() {
                    Some(text.to_owned())
                } else {
                    item["content"].as_array().and_then(|content| {
                        content
                            .iter()
                            .map(|block| block["text"].as_str())
                            .collect::<Option<Vec<_>>>()
                            .map(|parts| parts.join("\n"))
                    })
                };
                let Some(text) = text else { break };
                instructions.push(text);
            }
            input.drain(..instructions.len());
        }
        let text = instructions.join("\n\n");
        body.insert(
            "instructions".into(),
            if text.is_empty() {
                "You are a helpful assistant.".into()
            } else {
                text.into()
            },
        );
    }
}

pub(crate) struct CodexTransport {
    client: reqwest::Client,
    session_id: String,
    slot: Arc<Mutex<Slot>>,
}

impl std::fmt::Debug for CodexTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexTransport").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Slot {
    connection: Option<Connection>,
    sse_fallback: bool,
}

struct Connection {
    socket: Socket,
    handshake: Handshake,
    opened: Instant,
    previous: Option<Continuation>,
}

impl Connection {
    /// A successful turn does not guarantee its socket is still usable. Check
    /// the peer before submitting the next generation: a failed ping/pong is
    /// safe to abandon, unlike a failed response.create write.
    async fn ready_for_reuse(&mut self) -> bool {
        let nonce = generate_key().into_bytes();
        let probe = async {
            if self
                .socket
                .send(Message::Ping(nonce.clone().into()))
                .await
                .is_err()
            {
                return false;
            }
            loop {
                match self.socket.next().await {
                    Some(Ok(Message::Pong(payload))) if payload[..] == nonce[..] => {
                        return true;
                    }
                    Some(Ok(Message::Pong(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Ping(payload))) => {
                        if self.socket.send(Message::Pong(payload)).await.is_err() {
                            return false;
                        }
                    }
                    // A previous response.done can still be queued after the
                    // terminal event. It is not an answer to this fresh nonce.
                    Some(Ok(Message::Text(text)))
                        if self.is_previous_response_event(text.as_bytes()) => {}
                    Some(Ok(Message::Binary(bytes))) if self.is_previous_response_event(&bytes) => {
                    }
                    _ => return false,
                }
            }
        };
        tokio::time::timeout(REUSE_PROBE_TIMEOUT, probe)
            .await
            .unwrap_or(false)
    }

    fn is_previous_response_event(&self, bytes: &[u8]) -> bool {
        let Some(previous) = &self.previous else {
            return false;
        };
        serde_json::from_slice::<Value>(bytes).is_ok_and(|event| {
            event.pointer("/response/id").and_then(Value::as_str) == Some(previous.id.as_str())
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Handshake {
    url: String,
    headers: HeaderMap,
}

struct Continuation {
    id: String,
    parameters: Value,
    input: Vec<Value>,
}

struct Reader {
    guard: OwnedMutexGuard<Slot>,
    connection: Option<Connection>,
    client: reqwest::Client,
    handshake: Handshake,
    full: Value,
    received: bool,
    recovered: bool,
    terminal: bool,
    diagnostics: Diagnostics,
}

#[derive(Default)]
struct Diagnostics {
    events: usize,
    last_event: Option<String>,
    saw_output: bool,
    reused: bool,
    continued: bool,
}

impl Reader {
    /// Report protocol state, never request content or credentials. A close
    /// frame's reason is untrusted and may echo an authentication header.
    fn interrupted(&self, detail: &str) -> http_client::Error {
        let stage = if self.diagnostics.events == 0 {
            "awaiting_first_event"
        } else if self.diagnostics.saw_output {
            "receiving_output"
        } else {
            "awaiting_output"
        };
        let message = format!(
            "Codex WebSocket {detail} before a terminal response (stage={stage}, last_event={:?}, events={}, reused={}, continued={}, recovered={}, temperature={}, max_output_tokens={})",
            self.diagnostics.last_event.as_deref().unwrap_or("none"),
            self.diagnostics.events, self.diagnostics.reused,
            self.diagnostics.continued, self.recovered,
            self.full.get("temperature").is_some(),
            self.full.get("max_output_tokens").is_some(),
        );
        error(redact_diagnostic(&message, &self.handshake.headers))
    }
}

fn redact_diagnostic(message: &str, headers: &HeaderMap) -> String {
    let mut redacted = message.to_owned();
    for (name, value) in headers {
        let name = name.as_str();
        if !(value.is_sensitive()
            || name.contains("authorization")
            || name.contains("cookie")
            || name.contains("token")
            || name.contains("key")
            || name.contains("secret")
            || name.contains("account"))
        {
            continue;
        }
        let Ok(value) = value.to_str() else { continue };
        if value.len() >= 6 {
            redacted = redacted.replace(value, "<REDACTED>");
        }
        if name.contains("authorization") {
            if let Some((_, credential)) = value.split_once(' ') {
                if credential.len() >= 6 {
                    redacted = redacted.replace(credential, "<REDACTED>");
                }
            }
        }
    }
    redacted
}

fn error(message: impl Into<String>) -> http_client::Error {
    http_client::Error::Instance(std::io::Error::other(message.into()).into())
}

fn instance<E: std::error::Error + Send + Sync + 'static>(e: E) -> http_client::Error {
    http_client::Error::Instance(Box::new(e))
}

impl CodexTransport {
    pub(crate) fn new(session_id: String, user_agent: &str) -> anyhow::Result<Self> {
        // Reqwest performs the HTTP upgrade, retaining its rustls trust roots,
        // proxy support and timeouts. Do not bypass a configured HTTPS proxy by
        // opening a raw TcpStream. WebSocket upgrades require HTTP/1.1.
        let client = reqwest::Client::builder()
            .http1_only()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30 * 60))
            .build()?;
        Ok(Self {
            client,
            session_id,
            slot: Arc::new(Mutex::new(Slot::default())),
        })
    }

    /// `None` means a handshake explicitly rejected websocket support. The
    /// caller may send the original repaired HTTP/SSE request. No inference
    /// request has been submitted at that point. Authentication, rate-limit,
    /// transport, and post-submit failures must not silently take this path.
    pub(crate) async fn send(
        &self,
        request: &Request<Bytes>,
    ) -> http_client::Result<Option<http_client::StreamingResponse>> {
        let mut full: Value = serde_json::from_slice(request.body()).map_err(instance)?;
        let body = full
            .as_object_mut()
            .ok_or_else(|| error("Codex request is not an object"))?;
        for field in ["previous_response_id", "stream_id", "generate"] {
            if body.get(field).is_some_and(|v| !v.is_null()) {
                return Err(error(format!(
                    "Codex {field} is managed by the conversation transport"
                )));
            }
            body.remove(field);
        }
        if body.get("background").and_then(Value::as_bool) == Some(true) {
            return Err(error("Codex WebSocket background mode is unsupported"));
        }
        body.remove("background");
        body.remove("stream");
        body.insert("type".into(), Value::String("response.create".into()));
        let handshake = self.handshake(request)?;
        let mut guard = self.slot.clone().lock_owned().await;
        if guard.sse_fallback {
            return Ok(None);
        }
        let mut connection = guard
            .connection
            .take()
            .filter(|c| c.handshake == handshake && c.opened.elapsed() < MAX_CONNECTION_AGE);
        if let Some(cached) = connection.as_mut() {
            if !cached.ready_for_reuse().await {
                tracing::debug!("Discarding inactive Codex WebSocket before response.create");
                connection = None;
            }
        }
        let reused = connection.is_some();
        let mut connection = match connection {
            Some(connection) => connection,
            None => match tokio::time::timeout(CONNECT_TIMEOUT, connect(&self.client, &handshake))
                .await
            {
                Ok(Ok(Some(connection))) => connection,
                Ok(Ok(None)) => {
                    tracing::warn!("Codex endpoint rejected WebSocket support; using HTTP/SSE for this provider");
                    guard.sse_fallback = true;
                    return Ok(None);
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(error("Codex WebSocket handshake timed out")),
            },
        };
        let frame = continuation_frame(&full, connection.previous.as_ref());
        // A send error can mean that bytes were already accepted. Do not
        // blindly replay on another socket or switch to SSE.
        connection
            .socket
            .send(Message::text(frame.to_string()))
            .await
            .map_err(instance)?;
        let state = Reader {
            guard,
            connection: Some(connection),
            client: self.client.clone(),
            handshake,
            full,
            received: false,
            recovered: false,
            terminal: false,
            diagnostics: Diagnostics {
                reused,
                continued: frame.get("previous_response_id").is_some(),
                ..Default::default()
            },
        };
        let stream = futures::stream::try_unfold(state, |mut state| async move {
            if state.terminal {
                return Ok(None);
            }
            loop {
                let next = state
                    .connection
                    .as_mut()
                    .expect("active socket")
                    .socket
                    .next()
                    .await;
                let message = match next {
                    Some(Ok(message)) => message,
                    Some(Err(cause)) => {
                        return Err(state.interrupted(&format!("transport failed: {cause}")))
                    }
                    None => return Err(state.interrupted("ended without a close frame")),
                };
                let bytes = match message {
                    Message::Text(text) => text.as_bytes().to_vec(),
                    Message::Binary(bytes) => bytes.to_vec(),
                    Message::Ping(payload) => {
                        state
                            .connection
                            .as_mut()
                            .expect("active socket")
                            .socket
                            .send(Message::Pong(payload))
                            .await
                            .map_err(instance)?;
                        return Ok(Some((Bytes::from_static(b": websocket ping\n\n"), state)));
                    }
                    Message::Pong(_) => {
                        return Ok(Some((Bytes::from_static(b": websocket pong\n\n"), state)));
                    }
                    Message::Close(frame) => {
                        let detail = match frame {
                            Some(frame) => format!(
                                "closed (code={}, reason={:?})",
                                u16::from(frame.code),
                                redact_diagnostic(frame.reason.as_str(), &state.handshake.headers)
                            ),
                            None => "closed (code=none, reason=none)".into(),
                        };
                        return Err(state.interrupted(&detail));
                    }
                    Message::Frame(_) => continue,
                };
                let mut event: Value = serde_json::from_slice(&bytes).map_err(instance)?;
                let kind = event
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("Codex WebSocket event has no type"))?
                    .to_owned();
                // Some endpoints send response.done after response.completed.
                // The decoder may finish before reading it; never let a late
                // terminal event end the following turn.
                let prior_id = state
                    .connection
                    .as_ref()
                    .and_then(|c| c.previous.as_ref())
                    .map(|p| p.id.as_str());
                let event_id = event.pointer("/response/id").and_then(Value::as_str);
                if prior_id.is_some() && event_id == prior_id {
                    continue;
                }
                state.diagnostics.events = state.diagnostics.events.saturating_add(1);
                state.diagnostics.last_event = Some(kind.chars().take(80).collect());
                state.diagnostics.saw_output |= kind.ends_with(".delta")
                    || matches!(
                        kind.as_str(),
                        "response.output_item.added" | "response.output_item.done"
                    );
                if kind == "error" {
                    let code = event
                        .pointer("/error/code")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if !state.received
                        && !state.recovered
                        && matches!(
                            code,
                            "previous_response_not_found" | "websocket_connection_limit_reached"
                        )
                    {
                        // Explicit pre-generation rejections are safe to retry
                        // once, on a fresh connection and with FULL history.
                        state.connection.take();
                        let new = tokio::time::timeout(
                            CONNECT_TIMEOUT,
                            connect(&state.client, &state.handshake),
                        )
                        .await
                        .map_err(|_| error("Codex WebSocket recovery handshake timed out"))??;
                        let mut new = new.ok_or_else(|| {
                            error("Codex WebSocket support disappeared during recovery")
                        })?;
                        new.socket
                            .send(Message::text(state.full.to_string()))
                            .await
                            .map_err(instance)?;
                        state.connection = Some(new);
                        state.recovered = true;
                        state.diagnostics = Diagnostics::default();
                        continue;
                    }
                    let message = event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("request failed");
                    return Err(error(format!("Codex WebSocket {code}: {message}")));
                }
                state.received = true;
                let terminal = match kind.as_str() {
                    "response.completed" => true,
                    "response.done" => {
                        match event.pointer("/response/status").and_then(Value::as_str) {
                            Some("completed") => {
                                event["type"] = "response.completed".into();
                                true
                            }
                            Some("incomplete") => {
                                event["type"] = "response.incomplete".into();
                                true
                            }
                            _ => {
                                return Err(error(
                                    "Codex response.done did not contain a completed response body",
                                ))
                            }
                        }
                    }
                    "response.incomplete" => true,
                    "response.failed" => {
                        let message = event
                            .pointer("/response/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("response failed");
                        return Err(error(format!("Codex WebSocket response.failed: {message}")));
                    }
                    _ => false,
                };
                if terminal {
                    let response = &event["response"];
                    let expected_status = if event["type"] == "response.completed" {
                        "completed"
                    } else {
                        "incomplete"
                    };
                    if response["status"].as_str() != Some(expected_status)
                        || response["id"].as_str().is_none_or(str::is_empty)
                        || !response["output"].is_array()
                    {
                        return Err(error(
                            "Codex WebSocket terminal response has invalid status, id or output",
                        ));
                    }
                    state.terminal = true;
                    let completed = event["type"] == "response.completed"
                        && event.pointer("/response/status").and_then(Value::as_str)
                            == Some("completed");
                    if completed {
                        let response = &event["response"];
                        if let (Some(id), Some(output), Some(input)) = (
                            response["id"].as_str(),
                            response["output"].as_array(),
                            state.full["input"].as_array(),
                        ) {
                            let mut expected = input.clone();
                            expected.extend(output.iter().cloned());
                            let connection = state.connection.as_mut().expect("active socket");
                            connection.previous = Some(Continuation {
                                id: id.to_owned(),
                                parameters: parameters(&state.full),
                                input: expected,
                            });
                        }
                        // Restore before yielding the terminal event: Rig may
                        // stop polling its body immediately after this event.
                        state.guard.connection = state.connection.take();
                    }
                }
                let data = format!("data: {}\n\n", event);
                return Ok(Some((Bytes::from(data), state)));
            }
        });
        let stream: http_client::sse::BoxedStream = Box::pin(stream);
        Ok(Some(
            Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(stream)?,
        ))
    }

    fn handshake(&self, request: &Request<Bytes>) -> http_client::Result<Handshake> {
        let mut url = reqwest::Url::parse(&request.uri().to_string()).map_err(instance)?;
        let scheme = match url.scheme() {
            "ws" | "http" => "http",
            "wss" | "https" => "https",
            _ => return Err(error("Codex WebSocket endpoint must use http(s) or ws(s)")),
        };
        url.set_scheme(scheme)
            .map_err(|_| error("Invalid Codex WebSocket URL"))?;
        let mut headers = request.headers().clone();
        for name in [
            "content-type",
            "content-length",
            "accept",
            "connection",
            "upgrade",
            "sec-websocket-key",
            "sec-websocket-version",
            "sec-websocket-accept",
            "sec-websocket-protocol",
            "sec-websocket-extensions",
        ] {
            headers.remove(name);
        }
        if headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|v| v.trim() == "Bearer")
        {
            headers.remove("authorization");
        }
        headers
            .entry("openai-beta")
            .or_insert(HeaderValue::from_static(BETA));
        headers
            .entry("originator")
            .or_insert(HeaderValue::from_static("husk"));
        let session = HeaderValue::from_str(&self.session_id)?;
        headers.entry("session_id").or_insert(session.clone());
        headers.entry("session-id").or_insert(session);
        Ok(Handshake {
            url: url.into(),
            headers,
        })
    }
}

fn parameters(body: &Value) -> Value {
    let mut p = body.clone();
    p.as_object_mut().expect("request object").remove("input");
    p
}

fn continuation_frame(full: &Value, previous: Option<&Continuation>) -> Value {
    let mut frame = full.clone();
    if let (Some(previous), Some(input)) = (previous, full["input"].as_array()) {
        if previous.parameters == parameters(full) && input.starts_with(&previous.input) {
            frame["previous_response_id"] = previous.id.clone().into();
            frame["input"] = Value::Array(input[previous.input.len()..].to_vec());
        }
    }
    frame
}

async fn connect(
    client: &reqwest::Client,
    handshake: &Handshake,
) -> http_client::Result<Option<Connection>> {
    let key = generate_key();
    let response = client
        .get(&handshake.url)
        .headers(handshake.headers.clone())
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", &key)
        .send()
        .await
        .map_err(instance)?;
    let status = response.status();
    if matches!(status.as_u16(), 404 | 405 | 426 | 501) {
        return Ok(None);
    }
    if status.as_u16() != 101 {
        let headers = Box::new(response.headers().clone());
        // An unexpected 200 can itself be an unbounded streaming body.
        // Keep diagnostics bounded while the outer handshake deadline applies.
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(instance)?;
            let remaining = 16_384_usize.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if bytes.len() == 16_384 {
                break;
            }
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        return Err(http_client::Error::InvalidStatusCodeWithDetails {
            status,
            body,
            headers,
        });
    }
    let headers = response.headers();
    let token = |name: &str, wanted: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(',').any(|s| s.trim().eq_ignore_ascii_case(wanted)))
    };
    if !token("connection", "upgrade")
        || !token("upgrade", "websocket")
        || headers
            .get("sec-websocket-accept")
            .and_then(|h| h.to_str().ok())
            != Some(derive_accept_key(key.as_bytes()).as_str())
        || headers.contains_key("sec-websocket-extensions")
        || headers.contains_key("sec-websocket-protocol")
    {
        return Err(error("Invalid Codex WebSocket upgrade response"));
    }
    let upgraded = response.upgrade().await.map_err(instance)?;
    let socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;
    Ok(Some(Connection {
        socket,
        handshake: handshake.clone(),
        opened: Instant::now(),
        previous: None,
    }))
}
