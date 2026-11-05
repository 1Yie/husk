//! Offline Gemini SSE -> bridge -> persisted message -> next HTTP request.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_llm::sampler::{SampleRequest, Sampler};
use agent_llm::types::ToolCallAssembler;
use agent_llm::{
    ChatMessage, ModelParams, ProviderConfig, ProviderFactory, ProviderKind, StreamChunk,
};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Default)]
struct TraceCapture {
    entries: Arc<Mutex<Vec<String>>>,
    next_span: Arc<std::sync::atomic::AtomicU64>,
}

struct LogFields<'a>(&'a mut String);

impl tracing::field::Visit for LogFields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        write!(self.0, " {}={value:?}", field.name()).unwrap();
    }
}

impl tracing::Subscriber for TraceCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn new_span(&self, attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut entry = attributes.metadata().target().to_string();
        attributes.record(&mut LogFields(&mut entry));
        self.entries.lock().unwrap().push(entry);
        tracing::span::Id::from_u64(
            self.next_span
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1,
        )
    }

    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut entry = String::new();
        values.record(&mut LogFields(&mut entry));
        self.entries.lock().unwrap().push(entry);
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut entry = event.metadata().target().to_string();
        event.record(&mut LogFields(&mut entry));
        self.entries.lock().unwrap().push(entry);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

struct Gateway {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Gateway {
    async fn start(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let mut replies: VecDeque<_> = replies.into();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 8192];
                let (split, length) = loop {
                    if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..split]);
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= split + 4 + length {
                            break (split, length);
                        }
                    }
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                };
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap());
                let body = replies.pop_front().unwrap_or_else(text_reply);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::Gemini,
            base_url: self.url.clone(),
            api_key: "fixture-account".into(),
            ..Default::default()
        }
    }

    fn last_request(&self) -> Value {
        self.requests.lock().unwrap().last().unwrap().clone()
    }
}

fn sse(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn text_reply() -> String {
    sse(json!({"candidates":[{
        "content":{"role":"model","parts":[{"text":"done"}]},
        "finishReason":"STOP","index":0
    }],"usageMetadata":{
        "promptTokenCount":9, "candidatesTokenCount":2, "totalTokenCount":11,
        "cachedContentTokenCount":3
    }}))
}

fn tool_reply(round: &str) -> String {
    sse(json!({"candidates":[{
        "content":{"role":"model","parts":[
            {"functionCall":{"name":"read","args":{"path":format!("{round}-a")}},
             "thoughtSignature":format!("fixture-signature-{round}-a")},
            {"functionCall":{"id":format!("wire-{round}"),"name":"read","args":{"path":format!("{round}-b")}},
             "thoughtSignature":format!("fixture-signature-{round}-b")}
        ]},
        "finishReason":"STOP","index":0
    }]}))
}

async fn request(cfg: &ProviderConfig, model: &str, history: &[ChatMessage]) -> Vec<StreamChunk> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let provider =
            ProviderFactory::build_with_conversation(cfg, None, Some("fixture-session")).unwrap();
        let mut stream = provider
            .chat_stream(model, history, None, 0.0, None, &ModelParams::EMPTY)
            .await
            .unwrap();
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(
                !matches!(chunk, StreamChunk::Error(_)),
                "fixture request failed"
            );
            chunks.push(chunk);
        }
        chunks
    })
    .await
    .unwrap()
}

fn stored_assistant(chunks: &[StreamChunk]) -> ChatMessage {
    let mut calls = ToolCallAssembler::new();
    let mut state = None;
    for chunk in chunks {
        calls.feed(chunk);
        if let StreamChunk::ProviderState(value) = chunk {
            state = Some(value.clone());
        }
    }
    let mut message = ChatMessage::assistant("").with_provider_state(state);
    message.tool_calls = Some(calls.finish());
    message
}

fn add_round(history: &mut Vec<ChatMessage>, assistant: ChatMessage) {
    let calls = assistant.tool_calls.clone().unwrap();
    history.push(assistant);
    for call in calls {
        history.push(ChatMessage::tool_result(call.id, "fixture-result"));
    }
}

fn reload(history: &[ChatMessage]) -> Vec<ChatMessage> {
    serde_json::from_slice(&serde_json::to_vec(history).unwrap()).unwrap()
}

fn model_parts(body: &Value) -> Vec<&Value> {
    body["contents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|content| content["role"] == "model")
        .flat_map(|content| content["parts"].as_array().unwrap())
        .filter(|part| part.get("functionCall").is_some())
        .collect()
}

#[tokio::test]
async fn signed_calls_survive_serde_and_two_assistant_rounds() {
    let gateway = Gateway::start(vec![tool_reply("one"), tool_reply("two")]).await;
    let cfg = gateway.config();
    let mut history = vec![ChatMessage::user("inspect")];
    for round in ["one", "two"] {
        let chunks = request(&cfg, "fixture-model", &reload(&history)).await;
        let assistant = stored_assistant(&chunks);
        let state = assistant
            .provider_state
            .as_ref()
            .expect("signed calls need replay state");
        assert_eq!(state.model, "fixture-model");
        assert!(state.origin.starts_with("gemini:"));
        let saved: Value = serde_json::from_str(&state.data).unwrap();
        assert_eq!(saved.as_array().unwrap().len(), 2);
        for (index, suffix) in ["a", "b"].iter().enumerate() {
            let call = &assistant.tool_calls.as_ref().unwrap()[index];
            assert_eq!(saved[index]["id"], call.id);
            assert_eq!(saved[index]["name"], call.name);
            assert_eq!(saved[index]["arguments"], call.arguments);
            assert_eq!(
                saved[index]["signature"],
                format!("fixture-signature-{round}-{suffix}")
            );
        }
        assert!(
            assistant.content.as_deref().unwrap_or_default().is_empty(),
            "opaque state must not become visible text"
        );
        assert!(!format!("{assistant:?}").contains("fixture-signature"));
        assert_eq!(assistant.tool_calls.as_ref().unwrap().len(), 2);
        add_round(&mut history, assistant);
        if round == "two" {
            history.push(ChatMessage::user("finish"));
        }
    }
    request(&cfg, "fixture-model", &reload(&history)).await;
    let body = gateway.last_request();
    let parts = model_parts(&body);
    assert_eq!(parts.len(), 4);
    for (part, expected) in parts.iter().zip(["one-a", "one-b", "two-a", "two-b"]) {
        assert_eq!(part["functionCall"]["args"]["path"], expected);
        assert_eq!(
            part["thoughtSignature"],
            format!("fixture-signature-{expected}")
        );
    }
}

#[tokio::test]
async fn replay_rejects_changed_source_or_model() {
    let gateway = Gateway::start(vec![tool_reply("one")]).await;
    let cfg = gateway.config();
    let mut history = vec![ChatMessage::user("inspect")];
    let assistant = stored_assistant(&request(&cfg, "fixture-model", &history).await);
    assert!(assistant.provider_state.is_some());
    add_round(&mut history, assistant);
    for change in [
        "model",
        "account",
        "routing",
        "user_agent",
        "auth_header",
        "endpoint",
    ] {
        let mut changed = cfg.clone();
        let mut model = "fixture-model";
        let other = Gateway::start(vec![]).await;
        match change {
            "model" => model = "other-model",
            "account" => changed.api_key = "other-fixture-account".into(),
            "routing" => {
                changed
                    .headers
                    .insert("x-fixture-account".into(), "other".into());
            }
            "user_agent" => changed.user_agent = Some("other-fixture-agent".into()),
            "auth_header" => changed.auth_header = Some(false),
            "endpoint" => changed.base_url = other.url.clone(),
            _ => unreachable!(),
        }
        request(&changed, model, &reload(&history)).await;
        let body = if change == "endpoint" {
            other.last_request()
        } else {
            gateway.last_request()
        };
        let parts = model_parts(&body);
        assert_eq!(parts.len(), 2);
        assert!(
            parts
                .iter()
                .all(|part| part.get("thoughtSignature").is_none()),
            "{change}"
        );
    }
}

#[tokio::test]
async fn unsigned_history_and_edited_calls_do_not_borrow_signatures() {
    let gateway = Gateway::start(vec![tool_reply("one")]).await;
    let cfg = gateway.config();
    let input = vec![ChatMessage::user("inspect")];
    let assistant = stored_assistant(&request(&cfg, "fixture-model", &input).await);
    assert!(assistant.provider_state.is_some());
    for change in ["unsigned", "id", "name", "arguments"] {
        let mut edited = assistant.clone();
        let call = &mut edited.tool_calls.as_mut().unwrap()[0];
        match change {
            "unsigned" => edited.provider_state = None,
            "id" => call.id = "edited-id".into(),
            "name" => call.name = "edited-name".into(),
            "arguments" => call.arguments = r#"{"path":"edited"}"#.into(),
            _ => unreachable!(),
        }
        let mut history = input.clone();
        add_round(&mut history, edited);
        request(&cfg, "fixture-model", &reload(&history)).await;
        let body = gateway.last_request();
        let parts = model_parts(&body);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].get("thoughtSignature").is_none(), "{change}");
        if change == "unsigned" {
            assert!(parts[1].get("thoughtSignature").is_none());
        } else {
            assert_eq!(parts[1]["thoughtSignature"], "fixture-signature-one-b");
        }
    }
}

#[tokio::test]
async fn unsigned_and_unfinished_streams_emit_no_signature_state() {
    for complete in [true, false] {
        let reply = tool_reply("isolation");
        let mut value: Value =
            serde_json::from_str(reply.trim_start_matches("data: ").trim()).unwrap();
        if complete {
            for part in value["candidates"][0]["content"]["parts"]
                .as_array_mut()
                .unwrap()
            {
                part.as_object_mut().unwrap().remove("thoughtSignature");
            }
        } else {
            value["candidates"][0]
                .as_object_mut()
                .unwrap()
                .remove("finishReason");
        }
        let gateway = Gateway::start(vec![sse(value)]).await;
        let provider = ProviderFactory::build(&gateway.config()).unwrap();
        let chunks: Vec<_> = tokio::time::timeout(Duration::from_secs(5), async {
            provider
                .chat_stream(
                    "fixture-model",
                    &[ChatMessage::user("inspect")],
                    None,
                    0.0,
                    None,
                    &ModelParams::EMPTY,
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
        })
        .await
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
        assert!(!chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ProviderState(_))));
        assert_eq!(
            chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })),
            complete
        );
        assert_eq!(
            chunks.iter().any(|c| matches!(c, StreamChunk::Error(_))),
            !complete
        );
    }
}

#[tokio::test]
async fn trace_hides_gemini_payloads_without_silencing_other_logs() {
    let capture = TraceCapture::default();
    let gemini = Gateway::start(vec![
        tool_reply("trace"),
        text_reply(),
        "data: {\n\n".into(),
    ])
    .await;
    let other = Gateway::start(vec![sse(json!({
        "id":"fixture", "object":"chat.completion.chunk", "created":1, "model":"fixture-model",
        "choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]
    }))])
    .await;
    async {
        tracing::trace!(target: "agent_llm::fixture", "husk-before-native-stream");
        let cfg = gemini.config();
        let mut history = vec![ChatMessage::user("gemini-payload-marker")];
        let chunks = request(&cfg, "fixture-model", &history).await;
        add_round(&mut history, stored_assistant(&chunks));
        let replayed = request(&cfg, "fixture-model", &reload(&history)).await;
        assert!(replayed.iter().any(|chunk| matches!(
            chunk,
            StreamChunk::Done {
                prompt_tokens: Some(9),
                completion_tokens: Some(2),
                cached_tokens: Some(3),
            }
        )));
        tracing::trace!(target: "agent_llm::fixture", "husk-after-native-stream");
        let sampler = Sampler::new(ProviderFactory::build(&cfg).unwrap());
        let mut saw_error = false;
        let result = sampler
            .sample(
                SampleRequest {
                    model: "fixture-model",
                    temperature: 0.0,
                    tools: None,
                    reasoning_effort: None,
                    params: &ModelParams::EMPTY,
                },
                &reload(&history),
                |chunk| {
                    if matches!(chunk, StreamChunk::Error(_)) {
                        saw_error = true;
                        tracing::warn!(target: "agent_llm::fixture", "husk-provider-error");
                    }
                },
                |_| {},
                || {},
            )
            .await;
        assert!(result.is_err());
        assert!(saw_error);
        tracing::error!(target: "agent_llm::fixture", "husk-sampler-error");

        let mut other_cfg = other.config();
        other_cfg.kind = ProviderKind::OpenaiCompat;
        request(
            &other_cfg,
            "fixture-model",
            &[ChatMessage::user("other-wire-payload-marker")],
        )
        .await;
    }
    .with_subscriber(capture.clone())
    .await;

    let body = gemini.last_request();
    let parts = model_parts(&body);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["thoughtSignature"], "fixture-signature-trace-a");
    assert_eq!(parts[1]["thoughtSignature"], "fixture-signature-trace-b");

    let logs = capture.entries.lock().unwrap().join("\n");
    assert!(logs.contains("husk-before-native-stream"));
    assert!(logs.contains("husk-after-native-stream"));
    assert!(logs.contains("other-wire-payload-marker"));
    assert!(logs.contains("husk-provider-error"));
    assert!(logs.contains("husk-sampler-error"));
    assert!(
        !logs.contains("fixture-signature"),
        "signature leaked to TRACE"
    );
    assert!(
        !logs.contains("gemini-payload-marker"),
        "Gemini payload leaked to TRACE"
    );
}
