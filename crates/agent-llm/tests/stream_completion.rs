//! Exercise the raw provider, bridge and sampler against offline SSE responses.
use std::time::Duration;

use agent_llm::sampler::{SampleRequest, Sampler, SamplerEvent};
use agent_llm::{
    ChatMessage, ModelParams, ProviderConfig, ProviderFactory, ProviderKind, StreamChunk,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn sample(body: String) -> (bool, Vec<StreamChunk>, Vec<SamplerEvent>) {
    sample_wire(ProviderKind::OpenaiCompat, body).await
}

async fn sample_wire(
    kind: ProviderKind,
    body: String,
) -> (bool, Vec<StreamChunk>, Vec<SamplerEvent>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = ProviderConfig {
        kind,
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        ..Default::default()
    };
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 8192];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..split]);
                let length: usize = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    break;
                }
            }
        }
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
    });
    let provider =
        ProviderFactory::build_with_conversation(&cfg, None, Some("offline-fixture")).unwrap();
    let sampler = Sampler::new(provider);
    let mut chunks = Vec::new();
    let mut events = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sampler.sample(
            SampleRequest {
                model: "fixture",
                temperature: 0.0,
                tools: None,
                reasoning_effort: None,
                params: &ModelParams::EMPTY,
            },
            &[ChatMessage::user("fixture")],
            |c| chunks.push(c.clone()),
            |e| events.push(e.clone()),
            || {},
        ),
    )
    .await
    .expect("sampler must finish without retrying");
    server.await.unwrap();
    (result.is_ok(), chunks, events)
}

fn sse(parts: &[&str]) -> String {
    parts.iter().map(|p| format!("data: {p}\n\n")).collect()
}

const TEXT: &str = r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"fixture","choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]}"#;
const TOOL: &str = r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"fixture","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"todo","arguments":"{\"action\":"}}]},"finish_reason":null}]}"#;
const FINISH: &str = r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":2,"total_tokens":11}}"#;

#[tokio::test]
async fn raw_provider_eof_requires_protocol_completion() {
    for body in [String::new(), sse(&[TEXT]), sse(&[TOOL])] {
        let (ok, chunks, events) = sample(body).await;
        assert!(!ok, "unterminated raw provider succeeded: {chunks:?}");
        assert!(!chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
        assert!(!events
            .iter()
            .any(|e| matches!(e, SamplerEvent::Retrying { .. })));
        assert!(
            chunks
                .iter()
                .filter(|c| matches!(c, StreamChunk::ContentDelta(_)))
                .count()
                <= 1
        );
    }
}

#[tokio::test]
async fn raw_provider_malformed_frame_after_text_fails() {
    for malformed in ["{", TOOL] {
        let (ok, chunks, events) = sample(sse(&[TEXT, malformed, FINISH, "[DONE]"])).await;
        assert!(!ok, "malformed provider frame succeeded");
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Error(error) if error.starts_with("provider protocol error:"))));
        assert!(!events
            .iter()
            .any(|e| matches!(e, SamplerEvent::Retrying { .. })));
        assert_eq!(
            chunks
                .iter()
                .filter(|c| matches!(c, StreamChunk::ContentDelta(_)))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn raw_provider_completion_keeps_usage_and_fires_once() {
    let (ok, chunks, _) = sample(sse(&[TEXT, FINISH, "[DONE]"])).await;
    assert!(ok, "{chunks:?}");
    assert_eq!(
        chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Done { .. }))
            .count(),
        1
    );
    assert!(chunks.iter().any(|c| matches!(
        c,
        StreamChunk::Done {
            prompt_tokens: Some(9),
            completion_tokens: Some(2),
            ..
        }
    )));
}

#[tokio::test]
async fn other_raw_providers_require_their_terminal_event() {
    for (kind, partial) in [
        (
            ProviderKind::OpenaiResponses,
            sse(&[r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"sequence_number":1,"delta":"partial"}"#]),
        ),
        (
            ProviderKind::Anthropic,
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n".into(),
        ),
        (
            ProviderKind::Gemini,
            sse(&[r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"partial"}]},"index":0}]}"#]),
        ),
    ] {
        for body in [String::new(), partial] {
            let has_text = !body.is_empty();
            let (ok, chunks, events) = sample_wire(kind, body).await;
            assert!(!ok, "{kind:?}: {chunks:?}");
            assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Error(_))), "{kind:?}: {chunks:?}");
            assert!(!chunks.iter().any(|c| matches!(c, StreamChunk::Done { .. })));
            assert_eq!(chunks.iter().filter(|c| matches!(c, StreamChunk::ContentDelta(t) if t == "partial")).count(), usize::from(has_text), "{kind:?}: {chunks:?}");
            assert!(!events.iter().any(|e| matches!(e, SamplerEvent::Retrying { .. })));
        }
    }
}

#[tokio::test]
async fn completed_tool_call_keeps_arguments_and_usage() {
    let tool = serde_json::json!({
        "id":"c", "object":"chat.completion.chunk", "created":1, "model":"fixture",
        "choices":[{"index":0,"delta":{"tool_calls":[{
            "index":0, "id":"call_1", "type":"function",
            "function":{"name":"todo", "arguments":"{\"action\":\"list\"}"}
        }]},"finish_reason":null}]
    })
    .to_string();
    let (ok, chunks, _) = sample(sse(&[
        &tool,
        &FINISH.replace("stop", "tool_calls"),
        "[DONE]",
    ]))
    .await;
    assert!(ok, "{chunks:?}");
    assert_eq!(
        chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Done { .. }))
            .count(),
        1
    );
    let args: String = chunks
        .iter()
        .filter_map(|c| match c {
            StreamChunk::ToolCallDelta { args_delta, .. } => Some(args_delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(args, r#"{"action":"list"}"#);
}
