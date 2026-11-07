//! Public ProviderFactory → real local WebSocket → normalized stream tests.
//! These are protocol fixtures, not live Codex authentication or cache hits.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_llm::sampler::{SampleRequest, Sampler, SamplerEvent};
use agent_llm::{
    ChatMessage, LlmProvider, ModelParams, ProviderConfig, ProviderFactory, ProviderKind,
    StreamChunk,
};
use futures::{SinkExt, StreamExt};
use rig_core::http_client::HeaderMap;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
};

#[derive(Clone)]
enum Step {
    Complete(Value),
    CompleteThenClose(Value, Arc<tokio::sync::Notify>),
    CompleteThenStall(Value, Arc<tokio::sync::Notify>),
    CompleteThenControl(Value),
    Done(Value),
    LateDone(Value),
    Ping(Value),
    Reject(&'static str),
    ServiceError,
    PartialClose,
    CloseWith(u16, &'static str),
    CloseAfterDelta,
    StartedClose(&'static str, bool),
    PartialError,
    Malformed,
    Cancel(Arc<tokio::sync::Notify>),
}

struct Server {
    base: String,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
    frames: Arc<Mutex<Vec<Value>>>,
    probes: Arc<AtomicUsize>,
    http: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn new(connections: Vec<Vec<Step>>) -> Self {
        Self::with_http(connections, Vec::new()).await
    }

    async fn with_http(connections: Vec<Vec<Step>>, replies: Vec<(u16, String)>) -> Self {
        Self::start(connections, replies, None).await
    }

    // Tungstenite fixes Callback's error ABI to Response<Option<String>>;
    // it cannot be boxed by this fixture.
    #[allow(clippy::result_large_err)]
    async fn start(
        connections: Vec<Vec<Step>>,
        replies: Vec<(u16, String)>,
        stalled_http: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let headers = Arc::new(Mutex::new(Vec::new()));
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured_headers = headers.clone();
        let captured_frames = frames.clone();
        let probes = Arc::new(AtomicUsize::new(0));
        let captured_probes = probes.clone();
        let http = Arc::new(Mutex::new(Vec::new()));
        let captured_http = http.clone();
        let task = tokio::spawn(async move {
            let mut stalled_sockets = Vec::new();
            let mut connections = connections.into_iter();
            let mut replies = replies.into_iter();
            let mut http_tasks = tokio::task::JoinSet::new();
            let mut stalled_http = stalled_http;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut method = [0u8; 4];
                assert!(socket.peek(&mut method).await.unwrap() > 0);
                if method[0] == b'P' {
                    let (headers, body) = read_http(&mut socket).await;
                    captured_http.lock().unwrap().push((headers, body));
                    if let Some((ready, closed)) = stalled_http.take() {
                        http_tasks.spawn(async move {
                            ready.notify_one();
                            let mut bytes = [0; 1];
                            assert_eq!(socket.read(&mut bytes).await.unwrap(), 0);
                            closed.notify_one();
                        });
                        continue;
                    }
                    let (status, body) = replies.next().unwrap_or((400, "unexpected retry".into()));
                    socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    continue;
                }
                let turns = connections
                    .next()
                    .unwrap_or(vec![Step::Reject("bad_request")]);
                let h = captured_headers.clone();
                let mut ws = accept_hdr_async(socket, move |req: &Request, response: Response| {
                    assert_eq!(req.uri().path(), "/v1/responses");
                    h.lock().unwrap().push(req.headers().clone());
                    Ok(response)
                })
                .await
                .unwrap();
                let mut keep_open = false;
                for step in turns {
                    let text = loop {
                        match ws.next().await.unwrap().unwrap() {
                            Message::Text(text) => break text,
                            Message::Ping(payload) => {
                                captured_probes.fetch_add(1, Ordering::Relaxed);
                                ws.send(Message::Pong(payload)).await.unwrap();
                            }
                            Message::Pong(_) => {}
                            other => panic!("expected a create event, got {other:?}"),
                        }
                    };
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(frame["type"], "response.create");
                    captured_frames.lock().unwrap().push(frame);
                    let ping = matches!(&step, Step::Ping(_));
                    let done_alias = matches!(&step, Step::Done(_));
                    let late_done = matches!(&step, Step::LateDone(_));
                    let close_after = match &step {
                        Step::CompleteThenClose(_, closed) => Some(closed.clone()),
                        _ => None,
                    };
                    let stall_after = match &step {
                        Step::CompleteThenStall(_, ready) => Some(ready.clone()),
                        _ => None,
                    };
                    let controls_after = matches!(&step, Step::CompleteThenControl(_));
                    match step {
                        Step::Complete(response)
                        | Step::CompleteThenClose(response, _)
                        | Step::CompleteThenStall(response, _)
                        | Step::CompleteThenControl(response)
                        | Step::Done(response)
                        | Step::LateDone(response)
                        | Step::Ping(response) => {
                            if late_done {
                                let mut created = response.clone();
                                created["status"] = "in_progress".into();
                                created["output"] = json!([]);
                                ws.send(Message::text(json!({"type":"response.created","sequence_number":0,"response":created}).to_string())).await.unwrap();
                                let mut previous = response.clone();
                                previous["id"] = "resp_1".into();
                                previous["output"] = json!(output("first"));
                                ws.send(Message::text(json!({"type":"response.done","sequence_number":1,"response":previous}).to_string())).await.unwrap();
                            }
                            if ping {
                                ws.send(Message::Ping(vec![7, 9].into())).await.unwrap();
                                let pong = ws.next().await.unwrap().unwrap();
                                assert_eq!(pong, Message::Pong(vec![7, 9].into()));
                            }
                            for (index, item) in
                                response["output"].as_array().unwrap().iter().enumerate()
                            {
                                for part in item["content"].as_array().into_iter().flatten() {
                                    if part["type"] == "output_text" {
                                        ws.send(Message::text(
                                            delta(part["text"].as_str().unwrap()).to_string(),
                                        ))
                                        .await
                                        .unwrap();
                                    }
                                }
                                ws.send(Message::text(
                                    json!({"type":"response.output_item.done", "sequence_number":2,
                                    "output_index":index, "item":item})
                                    .to_string(),
                                ))
                                .await
                                .unwrap();
                            }
                            let event_type = if done_alias {
                                "response.done"
                            } else {
                                "response.completed"
                            };
                            ws.send(Message::text(
                                json!({"type":event_type,"sequence_number":3,"response":response})
                                    .to_string(),
                            ))
                            .await
                            .unwrap();
                            if let Some(closed) = close_after {
                                ws.close(None).await.unwrap();
                                closed.notify_one();
                            }
                            if controls_after {
                                ws.send(Message::Ping(vec![4, 2].into())).await.unwrap();
                                ws.send(Message::text(
                                    json!({"type":"response.done","sequence_number":4,"response":response}).to_string(),
                                )).await.unwrap();
                            }
                            if let Some(ready) = stall_after {
                                keep_open = true;
                                ready.notify_one();
                                break;
                            }
                        }
                        Step::Reject(code) => {
                            ws.send(Message::text(json!({"type":"error","status":400,"error":{"code":code,"message":"fixture rejection"}}).to_string())).await.unwrap();
                        }
                        Step::ServiceError => {
                            ws.send(Message::text(json!({"type":"error","status":400,"error":{"code":"invalid_request_error","message":"connection reset is an invalid parameter"}}).to_string())).await.unwrap();
                            break;
                        }
                        Step::PartialClose | Step::PartialError | Step::Cancel(_) => {
                            ws.send(Message::text(delta("partial").to_string()))
                                .await
                                .unwrap();
                            match step {
                                Step::PartialClose => {
                                    ws.close(None).await.unwrap();
                                }
                                Step::PartialError => {
                                    ws.send(Message::text(json!({"type":"error","error":{"code":"previous_response_not_found","message":"after output"}}).to_string())).await.unwrap();
                                }
                                Step::Cancel(done) => {
                                    let closed =
                                        tokio::time::timeout(Duration::from_secs(3), ws.next())
                                            .await;
                                    assert!(
                                        closed.is_ok(),
                                        "dropping the consumer must close its unfinished socket"
                                    );
                                    done.notify_one();
                                }
                                _ => unreachable!(),
                            }
                        }
                        Step::Malformed => {
                            ws.send(Message::text("{not-json")).await.unwrap();
                        }
                        Step::StartedClose(reason, partial) => {
                            let event = json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_partial","object":"response","created_at":1,"model":"fixture-gpt","status":"in_progress","output":[],"tools":[]}});
                            ws.send(Message::text(event.to_string())).await.unwrap();
                            if partial {
                                ws.send(Message::text(delta("partial").to_string()))
                                    .await
                                    .unwrap();
                            }
                            ws.send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Normal,
                                reason: reason.into(),
                            })))
                            .await
                            .unwrap();
                            break;
                        }
                        Step::CloseWith(code, reason) => {
                            ws.close(Some(CloseFrame {
                                code: CloseCode::from(code),
                                reason: reason.into(),
                            }))
                            .await
                            .unwrap();
                        }
                        Step::CloseAfterDelta => {
                            ws.send(Message::text(delta("partial").to_string()))
                                .await
                                .unwrap();
                            ws.close(Some(CloseFrame {
                                code: CloseCode::Away,
                                reason: "proxy idle".into(),
                            }))
                            .await
                            .unwrap();
                        }
                    }
                }
                if keep_open {
                    // Model a dead intermediary: TCP stays established, but
                    // the peer stops reading/responding to any WS frames.
                    stalled_sockets.push(ws);
                }
            }
        });
        Self {
            base,
            headers,
            frames,
            probes,
            http,
            task,
        }
    }

    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenaiCodexResponses,
            base_url: self.base.clone(),
            api_key: "fixture-access-token".into(),
            user_agent: Some("husk-wss-fixture".into()),
            headers: [
                ("chatgpt-account-id".into(), "fixture-account".into()),
                ("x-gateway-custom".into(), "kept".into()),
            ]
            .into(),
            ..Default::default()
        }
    }
    fn provider(&self) -> Arc<dyn LlmProvider> {
        ProviderFactory::build_with_conversation(&self.config(), None, Some("fixture-conversation"))
            .unwrap()
    }
    fn frames(&self) -> Vec<Value> {
        self.frames.lock().unwrap().clone()
    }
}

async fn read_http(socket: &mut tokio::net::TcpStream) -> (HeaderMap, Value) {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let split = loop {
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
    };
    let mut headers = HeaderMap::new();
    for line in String::from_utf8_lossy(&bytes[..split]).lines().skip(1) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.trim().parse().unwrap(),
        );
    }
    let length = headers["content-length"]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while bytes.len() < split + 4 + length {
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    (
        headers,
        serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap(),
    )
}

fn sse_response(id: &str, text: &str) -> String {
    let event = json!({"type":"response.completed","sequence_number":2,"response":response(id, output(text))});
    format!("data: {}\n\ndata: {event}\n\ndata: [DONE]\n\n", delta(text))
}

async fn sample(
    provider: &Arc<dyn LlmProvider>,
    history: &[ChatMessage],
    params: &ModelParams,
) -> (Result<(), String>, Vec<StreamChunk>, Vec<SamplerEvent>) {
    let mut chunks = Vec::new();
    let mut events = Vec::new();
    let tools = json!([{"name":"read","description":"fixture read","parameters":{"type":"object","properties":{}}}]);
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        Sampler::new(provider.clone()).sample(
            SampleRequest {
                model: "fixture-gpt",
                temperature: 0.0,
                tools: Some(&tools),
                reasoning_effort: Some("medium"),
                params,
            },
            history,
            |chunk| chunks.push(chunk.clone()),
            |event| events.push(event.clone()),
            || {},
        ),
    )
    .await
    .expect("sampler fixture timed out")
    .map_err(|e| e.to_string());
    (result, chunks, events)
}

#[tokio::test]
async fn sampler_tool_round_pong_close_recovers_once_with_full_http_request() {
    let tool = json!({"type":"function_call","id":"fc_fixture","call_id":"call_tool","name":"read","arguments":"{}","status":"completed"});
    let server = Server::with_http(
        vec![
            vec![
                Step::Complete(response(
                    "resp_tool",
                    vec![output("")[0].clone(), tool.clone()],
                )),
                Step::CloseWith(1000, ""),
            ],
            vec![Step::Complete(response("resp_next", output("next")))],
        ],
        vec![(200, sse_response("resp_sse", "recovered"))],
    )
    .await;
    let provider = server.provider();
    let params = ModelParams {
        sampling_params: Some(
            json!({"text":{"verbosity":"low"},"prompt_cache_options":{"mode":"fixture"}}),
        ),
        ..ModelParams::EMPTY
    };
    let mut h = history();
    let (first, chunks, events) = sample(&provider, &h, &params).await;
    first.unwrap();
    assert!(events.is_empty());
    let message = assistant(&chunks);
    assert!(message.provider_state.is_some());
    assert_eq!(message.tool_calls.as_ref().unwrap()[0].id, "call_tool");
    h.push(message);
    h.push(ChatMessage::tool_result("call_tool", "fixture tool result"));
    let (result, chunks, events) = sample(&provider, &h, &params).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(assistant(&chunks).content.as_deref(), Some("recovered"));
    let recovered = assistant(&chunks);
    let state = recovered.provider_state.as_ref().expect("SSE replay state");
    assert_eq!(
        serde_json::from_str::<Value>(&state.data).unwrap()["response_id"],
        "resp_sse"
    );
    let frames = server.frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(server.probes.load(Ordering::SeqCst), 1);
    assert_eq!(frames[1]["previous_response_id"], "resp_tool");
    assert_eq!(
        frames[1]["input"],
        json!([{"type":"function_call_output","call_id":"call_tool","output":"fixture tool result","status":"completed"}])
    );
    {
        let requests = server.http.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let (headers, body) = &requests[0];
        for (name, value) in [
            ("authorization", "Bearer fixture-access-token"),
            ("chatgpt-account-id", "fixture-account"),
            ("x-gateway-custom", "kept"),
            ("user-agent", "husk-wss-fixture"),
        ] {
            assert_eq!(headers[name], value);
        }
        assert!(headers.get("upgrade").is_none());
        assert_eq!(body["stream"], true);
        for field in [
            "type",
            "previous_response_id",
            "stream_id",
            "generate",
            "background",
        ] {
            assert!(body.get(field).is_none(), "{field}: {body}");
        }
        assert_eq!(body["instructions"], "stable instructions");
        assert_eq!(body["reasoning"], frames[0]["reasoning"]);
        assert_eq!(body["prompt_cache_key"], frames[0]["prompt_cache_key"]);
        assert_eq!(body["store"], false);
        assert_eq!(body["tools"], frames[0]["tools"]);
        assert!(!body["tools"].as_array().unwrap().is_empty());
        assert_eq!(body["text"], json!({"verbosity":"low"}));
        assert_eq!(body["prompt_cache_options"], json!({"mode":"fixture"}));
        let input = body["input"].as_array().unwrap();
        assert!(input.contains(&tool));
        assert!(input
            .iter()
            .any(|item| item["encrypted_content"] == "sealed-wss-fixture"));
        assert!(input
            .iter()
            .any(|item| item["output"] == "fixture tool result"));
        assert_eq!(input[0], frames[0]["input"][0]);
    }
    h.push(assistant(&chunks));
    h.push(ChatMessage::user("independent next turn"));
    let (next, chunks, events) = sample(&provider, &h, &params).await;
    next.unwrap();
    assert_eq!(assistant(&chunks).content.as_deref(), Some("next"));
    assert!(events.is_empty());
    assert_eq!(server.headers.lock().unwrap().len(), 2);
    assert_eq!(server.http.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sampler_started_close_preserves_partial_without_reason_based_replay() {
    for partial in [false, true] {
        for reason in ["", "connection reset"] {
            let server = Server::with_http(
                vec![vec![Step::StartedClose(reason, partial)]],
                vec![(200, sse_response("unexpected", "duplicated"))],
            )
            .await;
            let (result, chunks, events) =
                sample(&server.provider(), &history(), &ModelParams::EMPTY).await;
            assert!(result.unwrap_err().contains("fatal:"));
            assert!(events.is_empty(), "{events:?}");
            assert_eq!(server.frames().len(), 1);
            assert!(server.http.lock().unwrap().is_empty());
            assert_eq!(
                assistant(&chunks).content.as_deref(),
                Some(if partial { "partial" } else { "" })
            );
            assert!(!chunks.iter().any(|chunk| matches!(
                chunk,
                StreamChunk::Done { .. } | StreamChunk::ProviderState(_)
            )));
        }
    }
}

#[tokio::test]
async fn sampler_failed_sse_recovery_does_not_restart_ws_or_http() {
    for (status, body, partial) in [
        (503, "connection reset upstream error".to_string(), false),
        (200, format!("data: {}\n\n", delta("partial")), true),
        (200, "data: invalid\n\n".to_string(), false),
    ] {
        let server = Server::with_http(
            vec![vec![Step::CloseWith(1000, "connection reset")]],
            vec![(status, body)],
        )
        .await;
        let (result, chunks, events) =
            sample(&server.provider(), &history(), &ModelParams::EMPTY).await;
        assert!(result.is_err());
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(server.frames().len(), 1);
        assert_eq!(server.http.lock().unwrap().len(), 1);
        assert_eq!(
            assistant(&chunks).content.as_deref(),
            Some(if partial { "partial" } else { "" })
        );
    }
}

#[tokio::test]
async fn sampler_pre_event_transport_classification_is_independent_of_close_reason() {
    for reason in ["", "connection reset"] {
        let server = Server::with_http(
            vec![vec![Step::CloseWith(1000, reason)]],
            vec![(200, sse_response("resp_sse", "recovered"))],
        )
        .await;
        let mut config = server.config();
        if reason.is_empty() {
            config.base_url = config.base_url.replacen("http://", "ws://", 1);
        }
        let provider = ProviderFactory::build(&config).unwrap();
        let (result, chunks, events) = sample(&provider, &history(), &ModelParams::EMPTY).await;
        result.unwrap();
        assert_eq!(assistant(&chunks).content.as_deref(), Some("recovered"));
        assert!(events.is_empty());
        assert_eq!(server.frames().len(), 1);
        assert_eq!(server.http.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn sampler_service_and_protocol_failures_never_fall_back_or_replay() {
    for step in [
        Step::ServiceError,
        Step::Malformed,
        Step::CloseWith(1008, "connection reset"),
    ] {
        let server = Server::with_http(
            vec![vec![step]],
            vec![(200, sse_response("unexpected", "duplicated"))],
        )
        .await;
        let (result, chunks, events) =
            sample(&server.provider(), &history(), &ModelParams::EMPTY).await;
        assert!(result.unwrap_err().contains("fatal:"));
        assert!(events.is_empty());
        assert_eq!(server.frames().len(), 1);
        assert!(server.http.lock().unwrap().is_empty());
        assert!(assistant(&chunks).content.as_deref().unwrap().is_empty());
    }
}

#[tokio::test]
async fn cancelling_sse_recovery_releases_socket_and_allows_next_ws_request() {
    let ready = Arc::new(tokio::sync::Notify::new());
    let closed = Arc::new(tokio::sync::Notify::new());
    let server = Server::start(
        vec![
            vec![Step::CloseWith(1000, "")],
            vec![Step::Complete(response("resp_next", output("next")))],
        ],
        Vec::new(),
        Some((ready.clone(), closed.clone())),
    )
    .await;
    let provider = server.provider();
    let pending_provider = provider.clone();
    let pending =
        tokio::spawn(
            async move { sample(&pending_provider, &history(), &ModelParams::EMPTY).await },
        );
    tokio::time::timeout(Duration::from_secs(3), ready.notified())
        .await
        .unwrap();
    // The WS Reader's mutex must be available while the SSE open is pending.
    let (result, chunks, events) = sample(&provider, &history(), &ModelParams::EMPTY).await;
    result.unwrap();
    assert_eq!(assistant(&chunks).content.as_deref(), Some("next"));
    assert!(events.is_empty());
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), closed.notified())
        .await
        .unwrap();
    assert_eq!(server.frames().len(), 2);
    assert_eq!(server.http.lock().unwrap().len(), 1);
}

fn delta(text: &str) -> Value {
    json!({"type":"response.output_text.delta","sequence_number":1,"item_id":"msg_fixture", "output_index":0,"content_index":0,"delta":text})
}
fn response(id: &str, output: Vec<Value>) -> Value {
    json!({"id":id,"object":"response","created_at":1,"model":"fixture-gpt","status":"completed","tools":[],"output":output,
        "usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":2}}})
}
fn output(text: &str) -> Vec<Value> {
    vec![
        json!({"type":"reasoning","id":"rs_fixture","summary":[],"encrypted_content":"sealed-wss-fixture"}),
        json!({"type":"message","id":"msg_fixture","role":"assistant","phase":"commentary","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]}),
    ]
}
fn history() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("stable instructions"),
        ChatMessage::user("first request"),
    ]
}
async fn run(
    provider: &Arc<dyn LlmProvider>,
    history: &[ChatMessage],
    params: &ModelParams,
) -> anyhow::Result<Vec<StreamChunk>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = provider
            .chat_stream("fixture-gpt", history, None, 0.0, Some("medium"), params)
            .await?;
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => chunks.push(chunk),
                Err(error) => {
                    chunks.push(StreamChunk::Error(error.to_string()));
                    break;
                }
            }
        }
        Ok(chunks)
    })
    .await
    .expect("protocol test timed out")
}
fn failure(chunks: &[StreamChunk]) -> &str {
    assert!(
        !chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Done { .. } | StreamChunk::ProviderState(_))),
        "a failed turn cannot be completed or persisted: {chunks:?}"
    );
    chunks
        .iter()
        .find_map(|c| match c {
            StreamChunk::Error(message) => Some(message.as_str()),
            _ => None,
        })
        .expect("normalized provider error")
}

fn assistant(chunks: &[StreamChunk]) -> ChatMessage {
    let mut calls = agent_llm::types::ToolCallAssembler::new();
    let mut text = String::new();
    let mut state = None;
    for chunk in chunks {
        calls.feed(chunk);
        match chunk {
            StreamChunk::ContentDelta(s) => text.push_str(s),
            StreamChunk::ProviderState(s) => state = Some(s.clone()),
            _ => {}
        }
    }
    let mut message = ChatMessage::assistant(text).with_provider_state(state);
    let calls = calls.finish();
    message.tool_calls = (!calls.is_empty()).then_some(calls);
    message
}

#[tokio::test]
async fn codex_handshake_and_json_preserve_final_wire_parameters() {
    let server = Server::new(vec![vec![Step::Complete(response(
        "resp_1",
        output("hello"),
    ))]])
    .await;
    let provider = server.provider();
    assert_eq!(provider.id(), "openai_codex_responses");
    let params = ModelParams {
        max_tokens: Some(321),
        sampling_params: Some(
            json!({"text":{"verbosity":"low"},"prompt_cache_options":{"mode":"fixture"}}),
        ),
        ..ModelParams::EMPTY
    };
    let chunks = run(&provider, &history(), &params).await.unwrap();
    assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
    assert!(assistant(&chunks).provider_state.is_some());
    let headers = server.headers.lock().unwrap();
    assert_eq!(headers[0]["authorization"], "Bearer fixture-access-token");
    assert_eq!(headers[0]["chatgpt-account-id"], "fixture-account");
    assert_eq!(headers[0]["user-agent"], "husk-wss-fixture");
    assert_eq!(headers[0]["x-gateway-custom"], "kept");
    assert_eq!(headers[0]["openai-beta"], "responses_websockets=2026-02-06");
    assert!(headers[0].contains_key("session_id"));
    let frame = &server.frames()[0];
    assert_eq!(frame["store"], false);
    assert_eq!(frame["instructions"], "stable instructions");
    assert_eq!(frame["max_output_tokens"], 321);
    assert_eq!(frame["text"]["verbosity"], "low");
    assert_eq!(frame["prompt_cache_options"]["mode"], "fixture");
    assert!(frame.get("stream").is_none());
    assert!(frame.get("background").is_none());
    assert!(frame.get("prompt_cache_key").is_some());
}

#[tokio::test]
async fn fast_priority_and_custom_overlays_survive_responses_websocket_conversion() {
    let server = Server::new(vec![vec![Step::Complete(response(
        "resp_fast",
        output("fast answer"),
    ))]])
    .await;
    let mut cfg = server.config();
    cfg.models = vec![serde_json::from_value::<agent_llm::ModelConfig>(json!({
        "id":"fixture-gpt",
        "samplingParams":{"service_tier":"auto","top_p":0.8},
        "fastMode":{"mode":"priority","params":{"gateway_fast":{"enabled":true}},
            "headers":{"x-fast":"enabled"}}
    }))
    .unwrap()
    .into()];
    let provider =
        ProviderFactory::build_with_fast_mode(&cfg, None, Some("fixture-conversation"), true)
            .unwrap();
    let params = ProviderFactory::model_params(&cfg, "fixture-gpt");
    let chunks = run(&provider, &history(), &params).await.unwrap();
    assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
    let frame = &server.frames()[0];
    assert_eq!(frame["service_tier"], "priority");
    assert_eq!(frame["top_p"], 0.8);
    assert_eq!(frame["gateway_fast"], json!({"enabled":true}));
    assert_eq!(frame["reasoning"]["effort"], "medium");
    assert!(frame.get("fast_mode").is_none());
    assert!(frame.get("fastMode").is_none());
    assert_eq!(server.headers.lock().unwrap()[0]["x-fast"], "enabled");
}

#[tokio::test]
async fn fast_wire_model_is_the_replay_identity_and_off_drops_other_model_items() {
    assert_fast_replay_isolation(
        json!({"mode":"model","model":"configured-highspeed"}),
        "configured-highspeed",
    )
    .await;
}

#[tokio::test]
async fn fast_route_headers_isolate_opaque_replay_when_the_model_stays_the_same() {
    assert_fast_replay_isolation(
        json!({"mode":"custom","headers":{"x-gateway-account":"fast"}}),
        "fixture-gpt",
    )
    .await;
}

async fn assert_fast_replay_isolation(profile: Value, fast_model: &str) {
    let server = Server::new(vec![
        vec![
            Step::Complete(response("resp_fast_1", output("first"))),
            Step::Complete(response("resp_fast_2", output("second"))),
        ],
        vec![Step::Complete(response("resp_normal", output("normal")))],
    ])
    .await;
    let mut cfg = server.config();
    cfg.models = vec![serde_json::from_value::<agent_llm::ModelConfig>(json!({
        "id":"fixture-gpt", "fastMode":profile
    }))
    .unwrap()
    .into()];
    let provider =
        ProviderFactory::build_with_fast_mode(&cfg, None, Some("fixture-conversation"), true)
            .unwrap();
    let mut h = history();
    let first = assistant(&run(&provider, &h, &ModelParams::EMPTY).await.unwrap());
    assert_eq!(first.provider_state.as_ref().unwrap().model, fast_model);
    h.push(first);
    h.push(ChatMessage::user("continue fast"));
    let second = assistant(&run(&provider, &h, &ModelParams::EMPTY).await.unwrap());
    assert_eq!(second.provider_state.as_ref().unwrap().model, fast_model);
    h.push(second);
    h.push(ChatMessage::user("now use normal"));
    let off =
        ProviderFactory::build_with_fast_mode(&cfg, None, Some("fixture-conversation"), false)
            .unwrap();
    let normal = assistant(&run(&off, &h, &ModelParams::EMPTY).await.unwrap());
    assert_eq!(normal.provider_state.as_ref().unwrap().model, "fixture-gpt");
    let frames = server.frames();
    assert_eq!(frames[0]["model"], fast_model);
    assert_eq!(frames[1]["model"], fast_model);
    assert_eq!(frames[1]["previous_response_id"], "resp_fast_1");
    assert_eq!(frames[2]["model"], "fixture-gpt");
    assert!(frames[2].get("previous_response_id").is_none());
    assert!(!frames[2]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["encrypted_content"] == "sealed-wss-fixture"));
}

#[tokio::test]
async fn reuses_socket_and_only_sends_appended_history_after_persisted_reload() {
    let server = Server::new(vec![vec![
        Step::Complete(response("resp_1", output("hello"))),
        Step::Complete(response("resp_2", output("again"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next request"));
    let h: Vec<ChatMessage> = serde_json::from_value(serde_json::to_value(h).unwrap()).unwrap();
    run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    let frames = server.frames();
    assert_eq!(server.headers.lock().unwrap().len(), 1);
    assert_eq!(frames[1]["previous_response_id"], "resp_1");
    assert_eq!(frames[1]["input"].as_array().unwrap().len(), 1);
    assert_eq!(frames[1]["input"][0]["role"], "user");
    assert_eq!(server.probes.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn completed_connection_closed_by_peer_reconnects_before_next_create() {
    let closed = Arc::new(tokio::sync::Notify::new());
    let server = Server::new(vec![
        vec![Step::CompleteThenClose(
            response("resp_1", output("first")),
            closed.clone(),
        )],
        vec![Step::Complete(response("resp_2", output("second")))],
    ])
    .await;
    let provider = server.provider();
    let mut h = history();
    let first = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    h.push(assistant(&first));
    h.push(ChatMessage::user("next"));
    tokio::time::timeout(Duration::from_secs(2), closed.notified())
        .await
        .unwrap();

    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    assert!(
        chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::Done { .. })),
        "next message must not require a manual retry after an idle peer close: {chunks:?}"
    );
    assert_eq!(assistant(&chunks).content.as_deref(), Some("second"));
    let frames = server.frames();
    assert_eq!(
        frames.len(),
        2,
        "each generation must be submitted only once"
    );
    assert_eq!(server.headers.lock().unwrap().len(), 2);
    assert!(frames[1].get("previous_response_id").is_none());
    assert!(frames[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["encrypted_content"] == "sealed-wss-fixture" }));
}

#[tokio::test]
async fn completed_connection_silently_stalled_reconnects_with_full_context() {
    let stalled = Arc::new(tokio::sync::Notify::new());
    let server = Server::new(vec![
        vec![Step::CompleteThenStall(
            response("resp_1", output("first")),
            stalled.clone(),
        )],
        vec![Step::Complete(response("resp_2", output("second")))],
    ])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    tokio::time::timeout(Duration::from_secs(2), stalled.notified())
        .await
        .unwrap();

    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    assert_eq!(
        assistant(&chunks).content.as_deref(),
        Some("second"),
        "{chunks:?}"
    );
    assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
    let frames = server.frames();
    assert_eq!(
        frames.len(),
        2,
        "a probe must not submit a generation to the stalled socket"
    );
    assert_eq!(server.headers.lock().unwrap().len(), 2);
    assert!(frames[1].get("previous_response_id").is_none());
    assert!(frames[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["encrypted_content"] == "sealed-wss-fixture" }));
}

#[tokio::test]
async fn reuse_probe_handles_prior_terminal_and_server_ping_without_losing_chain() {
    let server = Server::new(vec![vec![
        Step::CompleteThenControl(response("resp_1", output("first"))),
        Step::Complete(response("resp_2", output("second"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));

    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    assert_eq!(assistant(&chunks).content.as_deref(), Some("second"));
    assert_eq!(server.headers.lock().unwrap().len(), 1);
    assert_eq!(server.probes.load(Ordering::Relaxed), 1);
    let frames = server.frames();
    assert_eq!(frames[1]["previous_response_id"], "resp_1");
    assert_eq!(frames[1]["input"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn cancelling_stalled_probe_releases_connection_and_next_request_starts_fresh() {
    let stalled = Arc::new(tokio::sync::Notify::new());
    let server = Server::new(vec![
        vec![Step::CompleteThenStall(
            response("resp_1", output("first")),
            stalled.clone(),
        )],
        vec![Step::Complete(response("resp_2", output("second")))],
    ])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    tokio::time::timeout(Duration::from_secs(2), stalled.notified())
        .await
        .unwrap();

    assert!(tokio::time::timeout(
        Duration::from_millis(50),
        run(&provider, &h, &ModelParams::EMPTY),
    )
    .await
    .is_err());
    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    assert_eq!(assistant(&chunks).content.as_deref(), Some("second"));
    let frames = server.frames();
    assert_eq!(
        frames.len(),
        2,
        "cancelled probe must not have submitted response.create"
    );
    assert!(frames[1].get("previous_response_id").is_none());
}

#[tokio::test]
async fn tool_round_continues_with_only_function_output() {
    let items = vec![
        output("")[0].clone(),
        json!({"type":"function_call","id":"fc_fixture","call_id":"call_fixture","name":"read","arguments":"{}","status":"completed"}),
    ];
    let server = Server::new(vec![vec![
        Step::Complete(response("resp_tool", items)),
        Step::Complete(response("resp_done", output("done"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    let stored = assistant(&chunks);
    assert!(stored.tool_calls.is_some());
    h.push(stored);
    h.push(ChatMessage::tool_result("call_fixture", "file content"));
    run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    let frames = server.frames();
    assert_eq!(frames[1]["previous_response_id"], "resp_tool");
    assert_eq!(frames[1]["input"][0]["type"], "function_call_output");
    assert_eq!(frames[1]["input"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn edited_history_does_not_reference_cached_unedited_text() {
    let server = Server::new(vec![vec![
        Step::Complete(response("resp_1", output("hello"))),
        Step::Complete(response("resp_2", output("again"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h[2].content = Some("edited".into());
    h.push(ChatMessage::user("next"));
    run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    let f = server.frames();
    assert!(f[1].get("previous_response_id").is_none());
    assert!(f[1]["input"].as_array().unwrap().len() > 1);
}

#[tokio::test]
async fn changed_parameters_disable_delta_continuation() {
    let server = Server::new(vec![vec![
        Step::Complete(response("resp_1", output("hello"))),
        Step::Complete(response("resp_2", output("again"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    run(
        &provider,
        &h,
        &ModelParams {
            sampling_params: Some(json!({"text":{"verbosity":"high"}})),
            ..ModelParams::EMPTY
        },
    )
    .await
    .unwrap();
    assert!(server.frames()[1].get("previous_response_id").is_none());
}

#[tokio::test]
async fn reconstructed_provider_replays_full_encrypted_native_history() {
    let server = Server::new(vec![
        vec![Step::Complete(response("resp_1", output("hello")))],
        vec![Step::Complete(response("resp_2", output("again")))],
    ])
    .await;
    let mut h = history();
    h.push(assistant(
        &run(&server.provider(), &h, &ModelParams::EMPTY)
            .await
            .unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    run(&server.provider(), &h, &ModelParams::EMPTY)
        .await
        .unwrap();
    let frames = server.frames();
    assert!(frames[1].get("previous_response_id").is_none());
    let items = frames[1]["input"].as_array().unwrap();
    assert!(items
        .iter()
        .any(|i| i["encrypted_content"] == "sealed-wss-fixture"));
    assert!(items.iter().any(|i| i["phase"] == "commentary"));
}

#[tokio::test]
async fn missing_previous_response_recovers_once_with_full_context() {
    let server = Server::new(vec![
        vec![
            Step::Complete(response("resp_1", output("hello"))),
            Step::Reject("previous_response_not_found"),
        ],
        vec![Step::Complete(response("resp_2", output("again")))],
    ])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    let frames = server.frames();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[1]["previous_response_id"], "resp_1");
    assert!(frames[2].get("previous_response_id").is_none());
    assert!(frames[2]["input"].as_array().unwrap().len() > 1);
}

#[tokio::test]
async fn connection_limit_reconnects_once_before_generation() {
    let server = Server::new(vec![
        vec![Step::Reject("websocket_connection_limit_reached")],
        vec![Step::Complete(response("resp_1", output("hello")))],
    ])
    .await;
    run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    assert_eq!(server.frames().len(), 2);
    assert!(server.frames()[1].get("previous_response_id").is_none());
}

#[tokio::test]
async fn malformed_event_is_an_error_not_an_empty_done() {
    let server = Server::new(vec![vec![Step::Malformed]]).await;
    let chunks = run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    assert!(failure(&chunks).contains("key must be a string"));
    assert_eq!(server.frames().len(), 1);
}

#[tokio::test]
async fn pings_are_answered_and_done_alias_is_normalized() {
    let server = Server::new(vec![vec![
        Step::Ping(response("resp_1", output("hello"))),
        Step::Done(response("resp_2", output("again"))),
    ]])
    .await;
    let provider = server.provider();
    let mut h = history();
    h.push(assistant(
        &run(&provider, &h, &ModelParams::EMPTY).await.unwrap(),
    ));
    h.push(ChatMessage::user("next"));
    let chunks = run(&provider, &h, &ModelParams::EMPTY).await.unwrap();
    assert!(assistant(&chunks).provider_state.is_some());
}

#[tokio::test]
async fn output_started_errors_and_disconnects_are_not_replayed() {
    for step in [Step::PartialClose, Step::PartialError] {
        let server = Server::new(vec![vec![step]]).await;
        let provider = server.provider();
        let mut stream = provider
            .chat_stream(
                "fixture-gpt",
                &history(),
                None,
                0.0,
                Some("medium"),
                &ModelParams::EMPTY,
            )
            .await
            .unwrap();
        let mut text = String::new();
        let mut failed = false;
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
        {
            match chunk {
                Ok(StreamChunk::ContentDelta(s)) => text.push_str(&s),
                Ok(StreamChunk::Done { .. } | StreamChunk::ProviderState(_)) => {
                    panic!("failed partial turn cannot be completed")
                }
                Ok(StreamChunk::Error(_)) | Err(_) => {
                    failed = true;
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(text, "partial");
        assert!(failed);
        assert_eq!(server.frames().len(), 1);
    }
}

#[tokio::test]
async fn dropping_partial_stream_closes_socket_and_next_turn_starts_fresh() {
    let closed = Arc::new(tokio::sync::Notify::new());
    let server = Server::new(vec![
        vec![Step::Cancel(closed.clone())],
        vec![Step::Complete(response("resp_2", output("again")))],
    ])
    .await;
    let provider = server.provider();
    let mut stream = provider
        .chat_stream(
            "fixture-gpt",
            &history(),
            None,
            0.0,
            Some("medium"),
            &ModelParams::EMPTY,
        )
        .await
        .unwrap();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
    {
        if matches!(chunk.unwrap(), StreamChunk::ContentDelta(_)) {
            break;
        }
    }
    drop(stream);
    tokio::time::timeout(Duration::from_secs(5), closed.notified())
        .await
        .unwrap();
    run(&provider, &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    assert!(server.frames()[1].get("previous_response_id").is_none());
}

#[tokio::test]
async fn unsupported_handshake_falls_back_to_sse_stickily() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let methods = Arc::new(Mutex::new(Vec::new()));
    let seen = methods.clone();
    let task = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let split = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..split]);
            let method = headers.split_whitespace().next().unwrap().to_string();
            let length = headers
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            seen.lock().unwrap().push(method.clone());
            while bytes.len() < split + 4 + length {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            if method == "GET" {
                socket.write_all(b"HTTP/1.1 426 Upgrade Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            } else {
                let event = json!({"type":"response.completed","sequence_number":1,"response":response("resp_sse",vec![])});
                let body = format!("data: {event}\n\ndata: [DONE]\n\n");
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        }
    });
    let cfg = ProviderConfig {
        kind: ProviderKind::OpenaiCodexResponses,
        base_url: base,
        api_key: "fixture".into(),
        ..Default::default()
    };
    let p = ProviderFactory::build(&cfg).unwrap();
    run(&p, &history(), &ModelParams::EMPTY).await.unwrap();
    run(&p, &history(), &ModelParams::EMPTY).await.unwrap();
    task.await.unwrap();
    assert_eq!(*methods.lock().unwrap(), ["GET", "POST", "POST"]);
}

#[tokio::test]
async fn handshake_auth_failure_is_not_silently_retried_as_sse() {
    for status in [401, 403] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = ProviderConfig {
            kind: ProviderKind::OpenaiCodexResponses,
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            api_key: "invalid-fixture".into(),
            ..Default::default()
        };
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0u8; 4096];
            let read = socket.read(&mut bytes).await.unwrap();
            assert!(read > 0);
            socket
            .write_all(format!("HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        });
        let p = ProviderFactory::build(&cfg).unwrap();
        let (result, chunks, events) = sample(&p, &history(), &ModelParams::EMPTY).await;
        assert!(result.unwrap_err().contains(&status.to_string()));
        assert!(events.is_empty());
        assert!(chunks.is_empty());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn explicit_recovery_is_bounded_to_one_retry() {
    let server = Server::new(vec![
        vec![Step::Reject("websocket_connection_limit_reached")],
        vec![Step::Reject("websocket_connection_limit_reached")],
    ])
    .await;
    let chunks = run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    assert!(failure(&chunks).contains("websocket_connection_limit_reached"));
    assert_eq!(server.frames().len(), 2);
}

#[tokio::test]
async fn ws_scheme_and_configured_headers_are_preserved_without_bearer_auth() {
    let server = Server::new(vec![vec![Step::Complete(response(
        "resp_1",
        output("hello"),
    ))]])
    .await;
    let mut cfg = server.config();
    cfg.base_url = cfg.base_url.replacen("http://", "ws://", 1);
    cfg.api_key.clear();
    cfg.auth_header = Some(false);
    cfg.headers
        .insert("openai-beta".into(), "fixture-explicit-beta".into());
    cfg.headers
        .insert("originator".into(), "fixture-client".into());
    let p = ProviderFactory::build(&cfg).unwrap();
    let chunks = run(&p, &history(), &ModelParams::EMPTY).await.unwrap();
    assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
    let headers = server.headers.lock().unwrap();
    assert!(headers[0].get("authorization").is_none());
    assert_eq!(headers[0]["openai-beta"], "fixture-explicit-beta");
    assert_eq!(headers[0]["originator"], "fixture-client");
}

#[tokio::test]
async fn invalid_websocket_accept_cannot_fall_back_or_complete() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = ProviderConfig {
        kind: ProviderKind::OpenaiCodexResponses,
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        api_key: "fixture".into(),
        ..Default::default()
    };
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = [0u8; 4096];
        let read = socket.read(&mut data).await.unwrap();
        assert!(read > 0);
        socket.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid-fixture\r\n\r\n").await.unwrap();
    });
    let chunks = run(
        &ProviderFactory::build(&cfg).unwrap(),
        &history(),
        &ModelParams::EMPTY,
    )
    .await
    .unwrap();
    assert!(failure(&chunks).to_lowercase().contains("upgrade"));
    task.await.unwrap();
}

#[tokio::test]
async fn contradictory_terminal_status_is_not_success() {
    let mut bad = response("resp_bad", output("partial"));
    bad["status"] = "in_progress".into();
    let server = Server::new(vec![vec![Step::Complete(bad)]]).await;
    let chunks = run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    failure(&chunks);
}

#[test]
fn codex_api_aliases_deserialize_without_changing_old_defaults() {
    for kind in ["openai_codex_responses", "openai-codex-responses"] {
        let cfg: ProviderConfig =
            serde_json::from_value(json!({"kind":kind,"base_url":"https://example.invalid/v1"}))
                .unwrap();
        assert_eq!(cfg.kind, ProviderKind::OpenaiCodexResponses);
    }
    assert_eq!(ProviderConfig::default().kind, ProviderKind::OpenaiCompat);
}

#[tokio::test]
async fn close_rejection_reports_protocol_diagnostics() {
    let server = Server::new(vec![vec![Step::CloseWith(
        1008,
        "unsupported parameter: temperature",
    )]])
    .await;
    let chunks = run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    let message = failure(&chunks);
    assert!(message.contains("1008"), "close code lost: {message}");
    assert!(
        message.contains("unsupported parameter: temperature"),
        "close reason lost: {message}"
    );
    assert!(
        message.contains("awaiting_first_event"),
        "request stage lost: {message}"
    );
}

#[tokio::test]
async fn close_diagnostics_redact_credentials_before_escaping() {
    let server = Server::new(vec![vec![Step::CloseWith(
        1008,
        "invalid token fixture-quoted-\"secret\"",
    )]])
    .await;
    let mut cfg = server.config();
    cfg.api_key = "fixture-quoted-\"secret\"".into();
    let provider = ProviderFactory::build(&cfg).unwrap();
    let chunks = run(&provider, &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    let message = failure(&chunks);
    assert!(message.contains("1008"));
    assert!(message.contains("<REDACTED>"), "{message}");
    assert!(!message.contains("fixture-quoted"), "{message}");
}

#[tokio::test]
async fn close_diagnostics_preserve_partial_output_and_stage_without_retry() {
    let server = Server::new(vec![vec![Step::CloseAfterDelta]]).await;
    let chunks = run(&server.provider(), &history(), &ModelParams::EMPTY)
        .await
        .unwrap();
    let message = failure(&chunks);
    assert!(message.contains("1001"), "{message}");
    assert!(message.contains("receiving_output"), "{message}");
    assert!(message.contains("response.output_text.delta"), "{message}");
    assert!(message.contains("reused=false"), "{message}");
    assert!(chunks
        .iter()
        .any(|c| matches!(c, StreamChunk::ContentDelta(s) if s == "partial")));
    assert_eq!(
        server.frames().len(),
        1,
        "partial generation must not be replayed"
    );
}

#[tokio::test]
async fn late_prior_terminal_after_new_created_cannot_finish_new_turn() {
    let server = Server::new(vec![vec![
        Step::Complete(response("resp_1", output("first"))),
        Step::LateDone(response("resp_2", output("second"))),
    ]])
    .await;
    let p = server.provider();
    let mut h = history();
    h.push(assistant(&run(&p, &h, &ModelParams::EMPTY).await.unwrap()));
    h.push(ChatMessage::user("next"));
    let chunks = run(&p, &h, &ModelParams::EMPTY).await.unwrap();
    let message = assistant(&chunks);
    assert_eq!(
        message.content.as_deref(),
        Some("second"),
        "a late prior terminal must not replace the active response"
    );
    let state = message.provider_state.expect("new response replay state");
    assert_eq!(
        serde_json::from_str::<Value>(&state.data).unwrap()["response_id"],
        "resp_2"
    );
}
