//! Public ProviderFactory → real local WebSocket → normalized stream tests.
//! These are protocol fixtures, not live Codex authentication or cache hits.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_llm::{
    ChatMessage, LlmProvider, ModelParams, ProviderConfig, ProviderFactory, ProviderKind,
    StreamChunk,
};
use futures::{SinkExt, StreamExt, TryStreamExt};
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
    Done(Value),
    LateDone(Value),
    Ping(Value),
    Reject(&'static str),
    PartialClose,
    CloseWith(u16, &'static str),
    CloseAfterDelta,
    PartialError,
    Malformed,
    Cancel(Arc<tokio::sync::Notify>),
}

struct Server {
    base: String,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
    frames: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    // Tungstenite fixes Callback's error ABI to Response<Option<String>>;
    // it cannot be boxed by this fixture.
    #[allow(clippy::result_large_err)]
    async fn new(connections: Vec<Vec<Step>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let headers = Arc::new(Mutex::new(Vec::new()));
        let frames = Arc::new(Mutex::new(Vec::new()));
        let captured_headers = headers.clone();
        let captured_frames = frames.clone();
        let task = tokio::spawn(async move {
            for turns in connections {
                let (socket, _) = listener.accept().await.unwrap();
                let h = captured_headers.clone();
                let mut ws = accept_hdr_async(socket, move |req: &Request, response: Response| {
                    assert_eq!(req.uri().path(), "/v1/responses");
                    h.lock().unwrap().push(req.headers().clone());
                    Ok(response)
                })
                .await
                .unwrap();
                for step in turns {
                    let next = ws.next().await.unwrap().unwrap();
                    let text = next.into_text().unwrap();
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(frame["type"], "response.create");
                    captured_frames.lock().unwrap().push(frame);
                    let ping = matches!(&step, Step::Ping(_));
                    let done_alias = matches!(&step, Step::Done(_));
                    let late_done = matches!(&step, Step::LateDone(_));
                    match step {
                        Step::Complete(response)
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
                        }
                        Step::Reject(code) => {
                            ws.send(Message::text(json!({"type":"error","status":400,"error":{"code":code,"message":"fixture rejection"}}).to_string())).await.unwrap();
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
            }
        });
        Self {
            base,
            headers,
            frames,
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
        provider
            .chat_stream("fixture-gpt", history, None, 0.0, Some("medium"), params)
            .await?
            .try_collect()
            .await
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
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let p = ProviderFactory::build(&cfg).unwrap();
    let chunks = run(&p, &history(), &ModelParams::EMPTY).await.unwrap();
    assert!(failure(&chunks).contains("401"));
    task.await.unwrap();
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
