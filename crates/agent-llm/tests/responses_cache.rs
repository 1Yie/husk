//! Request-level Responses regressions. A local scripted gateway observes the
//! real HTTP body and returns real SSE frames; these tests do NOT demonstrate
//! a cache hit at a real model or gateway.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use agent_llm::{
    ChatMessage, ModelParams, ProviderCompat, ProviderConfig, ProviderFactory, ProviderKind,
    StreamChunk,
};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

struct Gateway {
    base_url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn encode_event(event: &Value, multiline: bool) -> String {
    let kind = event["type"].as_str().unwrap();
    let payload = event.clone();
    let payload = if multiline {
        serde_json::to_string_pretty(&payload).unwrap()
    } else {
        payload.to_string()
    };
    let mut frame = format!("event: {kind}\n");
    for line in payload.lines() {
        frame.push_str(&format!("data: {line}\n"));
    }
    frame.push('\n');
    frame
}

impl Gateway {
    async fn new(replies: Vec<Value>) -> Self {
        Self::framed(replies, false).await
    }

    async fn framed(replies: Vec<Value>, multiline: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let mut replies = VecDeque::from(replies);
        let task = tokio::spawn(async move {
            while let Some(reply) = replies.pop_front() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 8192];
                let split = loop {
                    if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break split;
                    }
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0, "incomplete HTTP headers");
                    bytes.extend_from_slice(&chunk[..n]);
                };
                let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
                let path = head
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_string();
                let headers: HashMap<String, String> = head
                    .lines()
                    .skip(1)
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        Some((name.to_ascii_lowercase(), value.trim().to_string()))
                    })
                    .collect();
                let length: usize = headers["content-length"].parse().unwrap();
                while bytes.len() < split + 4 + length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0, "incomplete HTTP body");
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let body = serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap();
                seen.lock().unwrap().push(Request {
                    path,
                    headers,
                    body,
                });
                let mut data = String::new();
                let mut sequence = 1;
                for (index, item) in reply["output"].as_array().unwrap().iter().enumerate() {
                    for (content_index, block) in
                        item["content"].as_array().into_iter().flatten().enumerate()
                    {
                        if block["type"] == "output_text" {
                            let delta = json!({
                                "type":"response.output_text.delta", "item_id":item["id"],
                                "output_index":index, "content_index":content_index,
                                "sequence_number":sequence, "delta":block["text"],
                            });
                            sequence += 1;
                            data.push_str(&encode_event(&delta, multiline));
                        }
                    }
                    let event = json!({
                        "type": "response.output_item.done", "output_index": index,
                        "sequence_number": sequence, "item": item,
                    });
                    sequence += 1;
                    serde_json::from_value::<
                        rig_core::providers::openai::responses_api::streaming::ItemChunk,
                    >(event.clone())
                    .expect("the fixture output item must be valid");
                    data.push_str(&encode_event(&event, multiline));
                }
                let event = json!({
                    "type": "response.completed", "sequence_number": sequence,
                    "response": reply,
                });
                serde_json::from_value::<
                    rig_core::providers::openai::responses_api::streaming::ResponseChunk,
                >(event.clone())
                .expect("the fixture must be a valid Responses SSE event");
                data.push_str(&encode_event(&event, multiline));
                data.push_str("data: [DONE]\n\n");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    data.len(), data,
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            base_url,
            requests,
            task,
        }
    }

    fn config(&self) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenaiResponses,
            base_url: self.base_url.clone(),
            api_key: "fixture-not-a-secret".into(),
            ..Default::default()
        }
    }

    fn captured(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn response(id: &str, output: Vec<Value>, cached_tokens: u64) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": 1,
        "model": "fixture-gpt",
        "status": "completed",
        "tools": [],
        "output": output,
        "usage": {
            "input_tokens": 2048,
            "output_tokens": 12,
            "total_tokens": 2060,
            "input_tokens_details": { "cached_tokens": cached_tokens },
            "output_tokens_details": { "reasoning_tokens": 4 },
        },
    })
}

fn tools() -> Value {
    json!([
        {"name":"read", "description":"Read a file", "input_schema":{
            "type":"object", "properties":{"path":{"type":"string"}}, "required":["path"]
        }},
        {"name":"run", "description":"Run a command", "input_schema":{
            "type":"object", "properties":{"command":{"type":"string"}}, "required":["command"]
        }}
    ])
}

async fn request(
    cfg: &ProviderConfig,
    messages: &[ChatMessage],
    effort: Option<&str>,
    params: &ModelParams,
) -> Vec<StreamChunk> {
    request_model(cfg, "fixture-gpt", messages, effort, params, 0.0).await
}

async fn request_model(
    cfg: &ProviderConfig,
    model: &str,
    messages: &[ChatMessage],
    effort: Option<&str>,
    params: &ModelParams,
    temperature: f32,
) -> Vec<StreamChunk> {
    let provider =
        ProviderFactory::build_with_conversation(cfg, None, Some("fixture-session")).unwrap();
    let mut stream = provider
        .chat_stream(model, messages, Some(tools()), temperature, effort, params)
        .await
        .unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("the fixture SSE must decode"));
    }
    assert!(
        !chunks.iter().any(|c| matches!(c, StreamChunk::Error(_))),
        "{chunks:?}"
    );
    assert!(
        chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })),
        "missing terminal usage: {chunks:?}"
    );
    chunks
}

fn stored_assistant(chunks: &[StreamChunk]) -> ChatMessage {
    let mut calls = agent_llm::types::ToolCallAssembler::new();
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut state = None;
    for chunk in chunks {
        calls.feed(chunk);
        match chunk {
            StreamChunk::ContentDelta(part) => text.push_str(part),
            StreamChunk::ReasoningDelta(part) => reasoning.push_str(part),
            StreamChunk::ProviderState(value) => state = Some(value.clone()),
            _ => {}
        }
    }
    let mut message = ChatMessage::assistant(text)
        .with_reasoning(Some(reasoning))
        .with_provider_state(state);
    let calls = calls.finish();
    message.tool_calls = (!calls.is_empty()).then_some(calls);
    message
}

fn tool_output() -> Vec<Value> {
    vec![
        json!({"type":"reasoning", "id":"rs_fixture", "summary":[{"type":"summary_text", "text":"inspect safely"}], "encrypted_content":"opaque-completed-fixture"}),
        json!({"type":"message", "id":"msg_fixture", "role":"assistant", "phase":"commentary", "status":"completed", "content":[{"type":"output_text", "text":"I will inspect.", "annotations":[]}]}),
        json!({"type":"function_call", "id":"fc_fixture", "call_id":"call_fixture", "name":"read", "arguments":"{\"path\":\"README.md\"}", "status":"completed"}),
    ]
}

#[tokio::test]
async fn responses_replays_opaque_reasoning_item_ids_and_message_phase_after_reload() {
    let output = tool_output();
    let gateway = Gateway::new(vec![
        response("resp_original", output.clone(), 0),
        response("resp_followup", vec![], 1024),
    ])
    .await;
    let mut cfg = gateway.config();
    cfg.compat = Some(ProviderCompat {
        supports_store: Some(false),
        ..Default::default()
    });
    let mut history = vec![
        ChatMessage::system("stable instructions"),
        ChatMessage::user("inspect the repo"),
    ];
    let chunks = request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    let assistant = stored_assistant(&chunks);
    assert!(
        assistant.provider_state.is_some(),
        "Responses must preserve replay state, not just visible reasoning: {chunks:?}"
    );
    assert_eq!(assistant.content.as_deref(), Some("I will inspect."));
    history.push(assistant);
    history.push(ChatMessage::tool_result("call_fixture", "project notes"));
    // Rebuild the provider and reload the actual persisted message shape.
    let reloaded: Vec<ChatMessage> =
        serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
    request(&cfg, &reloaded, Some("medium"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    let input = requests[1].body["input"].as_array().unwrap();
    for item in &output {
        assert!(
            input.contains(item),
            "a complete original item must replay unchanged: {item}; input={input:?}"
        );
    }
    assert_eq!(requests[1].body["store"], false);
    assert!(input
        .iter()
        .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_fixture"));
}

#[tokio::test]
async fn multiline_sse_preserves_completed_replay_items() {
    let output = tool_output();
    let gateway = Gateway::framed(
        vec![
            response("resp_multiline_1", output.clone(), 0),
            response("resp_multiline_2", vec![], 0),
        ],
        true,
    )
    .await;
    let cfg = gateway.config();
    let mut history = vec![ChatMessage::user("inspect")];
    let chunks = request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    let assistant = stored_assistant(&chunks);
    assert!(
        assistant.provider_state.is_some(),
        "valid SSE data may span multiple data lines"
    );
    history.push(assistant);
    history.push(ChatMessage::tool_result("call_fixture", "notes"));
    request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    let input = requests[1].body["input"].as_array().unwrap();
    for item in &output {
        assert!(
            input.contains(item),
            "lost completed multiline item: {item}"
        );
    }
}

#[tokio::test]
async fn an_explicit_thinking_as_text_dialect_is_not_overridden_by_raw_replay() {
    let gateway = Gateway::new(vec![
        response("resp_text_thinking_1", tool_output(), 0),
        response("resp_text_thinking_2", vec![], 0),
    ])
    .await;
    let mut cfg = gateway.config();
    cfg.compat = Some(ProviderCompat {
        requires_thinking_as_text: Some(true),
        supports_mid_convo_effort: Some(true),
        ..Default::default()
    });
    let mut history = vec![ChatMessage::user("inspect")];
    let chunks = request(&cfg, &history, Some("high"), &ModelParams::EMPTY).await;
    history.push(stored_assistant(&chunks));
    history.push(ChatMessage::tool_result("call_fixture", "notes"));
    request(&cfg, &history, Some("high"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    let body = &requests[1].body;
    assert!(
        body.to_string().contains("inspect safely"),
        "this explicit dialect replays visible reasoning as assistant text"
    );
    assert!(!body.to_string().contains("opaque-completed-fixture"));
    assert!(!body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "reasoning" || item["type"] == "configuration_update"));
    assert_eq!(body["reasoning"]["effort"], "high");
}

#[tokio::test]
async fn legacy_reasoning_repairs_do_not_rewrite_original_encrypted_items() {
    let output = tool_output();
    let gateway = Gateway::new(vec![
        response("resp_legacy_1", output.clone(), 0),
        response("resp_legacy_2", vec![], 0),
    ])
    .await;
    let mut cfg = gateway.config();
    cfg.compat = Some(ProviderCompat {
        requires_reasoning_content_on_assistant_messages: Some(true),
        supports_store: Some(false),
        ..Default::default()
    });
    let mut history = vec![ChatMessage::user("inspect")];
    let chunks = request(&cfg, &history, Some("high"), &ModelParams::EMPTY).await;
    history.push(stored_assistant(&chunks));
    history.push(ChatMessage::tool_result("call_fixture", "notes"));
    request(&cfg, &history, Some("high"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    let input = requests[1].body["input"].as_array().unwrap();
    let reasoning = input
        .iter()
        .find(|item| item["id"] == "rs_fixture")
        .unwrap();
    assert_eq!(reasoning, &output[0], "legacy repair may fix synthetic history but must not add fields to an original encrypted provider item");
}

#[tokio::test]
async fn changing_effort_uses_append_only_updates_only_when_declared() {
    for enabled in [true, false] {
        let gateway = Gateway::new(vec![
            response("resp_low", tool_output(), 0),
            response("resp_high", vec![], 1024),
        ])
        .await;
        let mut cfg = gateway.config();
        cfg.compat = Some(ProviderCompat {
            supports_mid_convo_effort: Some(enabled),
            ..Default::default()
        });
        let mut history = vec![ChatMessage::system("stable"), ChatMessage::user("inspect")];
        let chunks = request(&cfg, &history, Some("low"), &ModelParams::EMPTY).await;
        history.push(stored_assistant(&chunks));
        history.push(ChatMessage::tool_result("call_fixture", "notes"));
        request(&cfg, &history, Some("high"), &ModelParams::EMPTY).await;
        let requests = gateway.captured();
        let (first, next) = (&requests[0].body, &requests[1].body);
        let input = next["input"].as_array().unwrap();
        if enabled {
            assert_eq!(first["reasoning"]["effort"], "medium");
            assert_eq!(next["reasoning"]["effort"], "medium");
            let prefix = first["input"].as_array().unwrap();
            assert_eq!(
                prefix,
                &input[..prefix.len()],
                "changing effort must not rewrite earlier input"
            );
            let updates: Vec<_> = input
                .iter()
                .filter(|item| item["type"] == "configuration_update")
                .collect();
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0]["reasoning"]["effort"], "low");
            assert_eq!(updates[1]["reasoning"]["effort"], "high");
        } else {
            assert_eq!(first["reasoning"]["effort"], "low");
            assert_eq!(next["reasoning"]["effort"], "high");
            assert!(!input
                .iter()
                .any(|item| item["type"] == "configuration_update"));
        }
        assert!(
            next.get("previous_response_id").is_none(),
            "full-history replay must not also chain server history by default"
        );
    }
}

#[tokio::test]
async fn replay_never_crosses_model_or_account_boundaries_or_undoes_history_edits() {
    let gateway = Gateway::new(vec![
        response("resp_source", tool_output(), 0),
        response("resp_edited", vec![], 0),
        response("resp_trimmed", vec![], 0),
        response("resp_other_model", vec![], 0),
        response("resp_other_account", vec![], 0),
        response("resp_other_client", vec![], 0),
    ])
    .await;
    let cfg = gateway.config();
    let mut history = vec![ChatMessage::user("inspect")];
    let chunks = request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    history.push(stored_assistant(&chunks));
    history.push(ChatMessage::tool_result("call_fixture", "notes"));
    let mut edited = history.clone();
    edited[1].content = Some("redacted by a hook".into());
    request(&cfg, &edited, Some("medium"), &ModelParams::EMPTY).await;
    let mut trimmed = history.clone();
    trimmed[1].tool_calls = None;
    request(&cfg, &trimmed, Some("medium"), &ModelParams::EMPTY).await;
    request_model(
        &cfg,
        "other-model",
        &history,
        Some("medium"),
        &ModelParams::EMPTY,
        0.0,
    )
    .await;
    let mut other_account = cfg.clone();
    other_account.api_key = "other-fixture-account".into();
    request(
        &other_account,
        &history,
        Some("medium"),
        &ModelParams::EMPTY,
    )
    .await;
    let mut other_client = cfg.clone();
    other_client.user_agent = Some("other-client/1.0".into());
    request(&other_client, &history, Some("medium"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    assert_eq!(requests.len(), 6);
    for request in &requests[1..] {
        assert!(
            !request
                .body
                .to_string()
                .contains("opaque-completed-fixture"),
            "opaque reasoning escaped its scope"
        );
        assert!(!request.body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.get("phase").is_some()));
    }
    assert!(requests[1].body.to_string().contains("redacted by a hook"));
    assert!(!requests[1].body.to_string().contains("I will inspect."));
    assert!(!requests[2].body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "function_call"));
}

#[tokio::test]
async fn explicit_sampling_parameters_survive_typed_conversion_and_null_omits_fields() {
    let gateway = Gateway::new(vec![response("resp_params", vec![], 0)]).await;
    let mut cfg = gateway.config();
    cfg.compat = Some(ProviderCompat {
        supports_mid_convo_effort: Some(true),
        supports_long_cache_retention: Some(true),
        ..Default::default()
    });
    let params = ModelParams {
        max_tokens: Some(256),
        sampling_params: Some(json!({
            "max_output_tokens":512, "temperature":null,
            "prompt_cache_options":{"mode":"explicit", "ttl":"30m"},
            "reasoning":{"effort":"low"}, "text":{"verbosity":"low"}
        })),
        ..ModelParams::EMPTY
    };
    request_model(
        &cfg,
        "fixture-gpt",
        &[ChatMessage::user("hi")],
        Some("high"),
        &params,
        0.7,
    )
    .await;
    let body = &gateway.captured()[0].body;
    assert_eq!(body["max_output_tokens"], 512);
    assert!(body.get("temperature").is_none());
    assert_eq!(
        body["prompt_cache_options"],
        json!({"mode":"explicit", "ttl":"30m"})
    );
    assert!(
        body.get("prompt_cache_retention").is_none(),
        "new cache options must replace, not conflict with, the generated legacy retention field"
    );
    assert_eq!(body["text"]["verbosity"], "low");
    assert_eq!(
        body["reasoning"]["effort"], "low",
        "an explicit request-level override must not be replaced by the fixed update baseline"
    );
    assert!(!body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "configuration_update"));
}

#[tokio::test]
async fn consecutive_user_inputs_do_not_rewrite_the_previous_input_item() {
    let gateway = Gateway::new(vec![
        response("resp_user_1", vec![], 0),
        response("resp_user_2", vec![], 0),
    ])
    .await;
    let cfg = gateway.config();
    let mut history = vec![
        ChatMessage::system("stable"),
        ChatMessage::user("initial request"),
    ];
    request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    history.push(ChatMessage::user_hidden(
        "persistent follow-up from the host",
    ));
    request(&cfg, &history, Some("medium"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    let prefix = requests[0].body["input"].as_array().unwrap();
    let next = requests[1].body["input"].as_array().unwrap();
    assert!(next.len() > prefix.len());
    assert_eq!(
        prefix,
        &next[..prefix.len()],
        "merging consecutive Responses messages must not rewrite an already cached input item"
    );
}

#[tokio::test]
async fn conflicting_explicit_cache_lifetimes_are_rejected_before_http() {
    let gateway = Gateway::new(vec![response("unused", vec![], 0)]).await;
    let provider = ProviderFactory::build(&gateway.config()).unwrap();
    let params = ModelParams {
        sampling_params: Some(
            json!({"prompt_cache_retention":"24h", "prompt_cache_options":{"ttl":"12h"}}),
        ),
        ..ModelParams::EMPTY
    };
    let error = match provider
        .chat_stream(
            "fixture-gpt",
            &[ChatMessage::user("hi")],
            None,
            0.0,
            None,
            &params,
        )
        .await
    {
        Ok(_) => panic!("ambiguous cache lifetime configuration must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("mutually exclusive"));
    assert!(gateway.captured().is_empty());
}

#[tokio::test]
async fn a_non_reasoning_model_cannot_receive_an_effort_hint_from_its_caller() {
    let gateway = Gateway::new(vec![response("resp_plain_model", vec![], 0)]).await;
    let mut cfg = gateway.config();
    cfg.models = serde_json::from_value(json!([{"id":"plain-model", "reasoning":false}])).unwrap();
    cfg.compat = Some(ProviderCompat {
        supports_mid_convo_effort: Some(true),
        ..Default::default()
    });
    let mut params = ProviderFactory::model_params(&cfg, "plain-model");
    params.sampling_params = Some(json!({"reasoning":{"effort":"high"}}));
    request_model(
        &cfg,
        "plain-model",
        &[ChatMessage::user("hi")],
        Some("high"),
        &params,
        0.7,
    )
    .await;
    let body = &gateway.captured()[0].body;
    assert!(body.get("reasoning").is_none(), "reasoning:false must narrow the protocol baseline even if a caller provides an effort hint");
    assert!(!body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "configuration_update"));
}

#[tokio::test]
async fn a_declared_temperature_capability_overrides_hints_and_sampling_values() {
    let gateway = Gateway::new(vec![
        response("resp_temperature", vec![], 0),
        response("resp_model_on", vec![], 0),
        response("resp_model_off", vec![], 0),
    ])
    .await;
    let mut value = serde_json::to_value(gateway.config()).unwrap();
    value["compat"] = json!({"supportsTemperature":false});
    let mut cfg: ProviderConfig = serde_json::from_value(value).unwrap();
    let mut params = ModelParams {
        sampling_params: Some(json!({"temperature":0.6})),
        ..ModelParams::EMPTY
    };
    request_model(
        &cfg,
        "no-temperature-model",
        &[ChatMessage::user("hi")],
        Some("high"),
        &params,
        0.7,
    )
    .await;
    params.compat = Some(ProviderCompat {
        supports_temperature: Some(true),
        ..Default::default()
    });
    request_model(
        &cfg,
        "temperature-model",
        &[ChatMessage::user("hi")],
        Some("high"),
        &params,
        0.7,
    )
    .await;
    cfg.compat.as_mut().unwrap().supports_temperature = Some(true);
    params.compat.as_mut().unwrap().supports_temperature = Some(false);
    request_model(
        &cfg,
        "no-temperature-model",
        &[ChatMessage::user("hi")],
        Some("high"),
        &params,
        0.7,
    )
    .await;
    let requests = gateway.captured();
    assert!(
        requests[0].body.get("temperature").is_none(),
        "a model capability must suppress unsupported temperature even when a caller supplies it"
    );
    assert_eq!(
        requests[1].body["temperature"], 0.6,
        "an explicit model capability overrides its provider"
    );
    assert!(requests[2].body.get("temperature").is_none());
}

#[tokio::test]
async fn a_provider_that_rejects_strict_tool_fields_receives_none() {
    let gateway = Gateway::new(vec![response("resp_strict", vec![], 0)]).await;
    let mut cfg = gateway.config();
    cfg.compat = Some(ProviderCompat {
        supports_strict_mode: Some(false),
        ..Default::default()
    });
    request(&cfg, &[ChatMessage::user("hi")], None, &ModelParams::EMPTY).await;
    let body = &gateway.captured()[0].body;
    for tool in body["tools"].as_array().unwrap() {
        assert!(
            tool.get("strict").is_none(),
            "a capability opt-out must omit strict, not send strict:false: {tool}"
        );
    }
}

#[tokio::test]
async fn unchanged_tool_sets_have_a_canonical_order() {
    let gateway = Gateway::new(vec![
        response("resp_tools_1", vec![], 0),
        response("resp_tools_2", vec![], 0),
    ])
    .await;
    let cfg = gateway.config();
    let provider =
        ProviderFactory::build_with_conversation(&cfg, None, Some("fixture-session")).unwrap();
    let mut reversed = tools();
    reversed.as_array_mut().unwrap().reverse();
    for definitions in [tools(), reversed] {
        let mut stream = provider
            .chat_stream(
                "fixture-gpt",
                &[ChatMessage::user("hi")],
                Some(definitions),
                0.0,
                None,
                &ModelParams::EMPTY,
            )
            .await
            .unwrap();
        while let Some(chunk) = stream.next().await {
            chunk.unwrap();
        }
    }
    let requests = gateway.captured();
    assert_eq!(
        requests[0].body["tools"], requests[1].body["tools"],
        "registry/plugin enumeration order must not change the cached tool prefix"
    );
}

#[tokio::test]
async fn responses_uses_its_native_output_cap_and_store_policy() {
    for store in [false, true] {
        let gateway = Gateway::new(vec![response("resp_cap", vec![], 0)]).await;
        let mut cfg = gateway.config();
        cfg.compat = Some(ProviderCompat {
            supports_store: Some(store),
            // These Chat Completions settings must not rename Responses fields.
            max_tokens_field: Some("max_completion_tokens".into()),
            thinking_format: Some("deepseek".into()),
            ..Default::default()
        });
        let params = ModelParams {
            max_tokens: Some(512),
            ..ModelParams::EMPTY
        };
        request(&cfg, &[ChatMessage::user("hi")], Some("high"), &params).await;
        let body = &gateway.captured()[0].body;
        assert_eq!(
            body["max_output_tokens"], 512,
            "Responses output caps must not be lost in additional_params"
        );
        assert_eq!(
            body["store"], store,
            "the configured storage policy must reach Responses"
        );
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
        assert_eq!(body["reasoning"]["effort"], "high");
    }
}

#[tokio::test]
async fn stable_context_reuses_the_gateway_cache_key_and_input_prefix() {
    let gateway = Gateway::new(vec![
        response("resp_1", vec![], 0),
        response("resp_2", vec![], 1024),
    ])
    .await;
    let cfg = gateway.config();
    let mut messages = vec![
        ChatMessage::system("Stable coding instructions. ".repeat(400)),
        ChatMessage::user("Read the project."),
    ];
    request(&cfg, &messages, Some("medium"), &ModelParams::EMPTY).await;
    messages.push(ChatMessage::assistant("I read the project."));
    messages.push(ChatMessage::user("Now run its tests."));
    let chunks = request(&cfg, &messages, Some("medium"), &ModelParams::EMPTY).await;
    let requests = gateway.captured();
    assert_eq!(requests.len(), 2);
    let (first, next) = (&requests[0], &requests[1]);
    assert_eq!(first.path, "/v1/responses");
    assert_eq!(first.body["instructions"], next.body["instructions"]);
    assert_eq!(first.body["tools"], next.body["tools"]);
    assert_eq!(first.body["reasoning"], next.body["reasoning"]);
    let prefix = first.body["input"].as_array().unwrap();
    assert_eq!(
        prefix,
        &next.body["input"].as_array().unwrap()[..prefix.len()]
    );
    assert_eq!(
        first.headers["x-session-affinity"],
        next.headers["x-session-affinity"]
    );
    assert!(first.body["prompt_cache_key"].is_string(), "a Responses gateway needs the standard cache routing key, not only a vendor-specific header: {}", first.body);
    assert_eq!(
        first.body["prompt_cache_key"],
        next.body["prompt_cache_key"]
    );
    assert!(
        chunks.iter().any(|c| matches!(
            c,
            StreamChunk::Done {
                cached_tokens: Some(1024),
                ..
            }
        )),
        "Responses input_tokens_details.cached_tokens was lost: {chunks:?}"
    );
}
