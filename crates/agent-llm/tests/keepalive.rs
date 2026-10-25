//! A model that thinks without showing it sends nothing rig turns into an
//! item: pings, SSE comments, events it does not surface. The connection is
//! alive, but the stream above it is silent, and a sampler that only counts
//! content would call it wedged after five minutes and resend the request.
//! These tests serve a real SSE connection and read what the provider hands up.

use std::time::Duration;

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use agent_llm::provider::LlmProvider;
use agent_llm::types::{ChatMessage, StreamChunk};
use agent_llm::{ModelParams, ProviderConfig, ProviderKind, RigProvider};

const ANSWER: &str = "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"sequence_number\":1,\"delta\":\"hi\"}\n\n";
const ANTHROPIC_ANSWER: &str = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n";

/// Answers one request: `lead` goes out at once, then `beats` copies of
/// `beat` a short while apart, then `tail`; with `beats == 0` the connection
/// just stays open and silent after `lead`.
async fn server(
    lead: &'static str,
    beat: &'static str,
    beats: usize,
    tail: &'static str,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0u8; 16384];
        let mut head = Vec::new();
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            match socket.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => head.extend_from_slice(&buf[..n]),
            }
        }
        let split = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let declared = String::from_utf8_lossy(&head[..split])
            .to_lowercase()
            .lines()
            .find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().parse().unwrap_or(0))
            })
            .unwrap_or(0usize);
        let mut got = head.len() - split - 4;
        while got < declared {
            match socket.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await;
        let send = |s: &'static str| format!("{:x}\r\n{s}\r\n", s.len());
        if !lead.is_empty() {
            let _ = socket.write_all(send(lead).as_bytes()).await;
        }
        for _ in 0..beats {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = socket.write_all(send(beat).as_bytes()).await;
        }
        if beats == 0 {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        }
        let _ = socket.write_all(send(tail).as_bytes()).await;
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    format!("http://{addr}/v1")
}

async fn stream(kind: ProviderKind, base: String) -> agent_llm::provider::BoxStream<StreamChunk> {
    let cfg = ProviderConfig {
        kind,
        base_url: base,
        ..Default::default()
    };
    let provider = RigProvider::new(&cfg, String::new(), reqwest::Client::new(), None, None)
        .unwrap()
        .with_keepalive_every(Duration::from_millis(60));
    provider
        .chat_stream(
            "m",
            &[ChatMessage::user("hi")],
            None,
            0.0,
            None,
            &ModelParams::EMPTY,
        )
        .await
        .expect("stream")
}

/// Everything the provider hands up until `stop` says to quit.
async fn collect(
    mut s: agent_llm::provider::BoxStream<StreamChunk>,
    stop: impl Fn(&[StreamChunk]) -> bool,
) -> Vec<StreamChunk> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(item) = s.next().await {
            if let Ok(chunk) = item {
                out.push(chunk);
            }
            if stop(&out) {
                break;
            }
        }
    })
    .await;
    out
}

fn keepalives(chunks: &[StreamChunk]) -> usize {
    chunks
        .iter()
        .filter(|c| matches!(c, StreamChunk::Keepalive))
        .count()
}

/// Comments keep arriving while nothing is worth showing: the provider says so.
#[tokio::test]
async fn a_silent_but_live_responses_stream_reports_keepalive() {
    let base = server("", ": still thinking\n\n", 12, ANSWER).await;
    let chunks = collect(stream(ProviderKind::OpenaiResponses, base).await, |c| {
        c.iter().any(|x| matches!(x, StreamChunk::ContentDelta(_)))
    })
    .await;
    assert!(keepalives(&chunks) >= 1, "{chunks:?}");
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ContentDelta(t) if t == "hi")),
        "{chunks:?}"
    );
}

/// Anthropic's `ping` is a known no-op in rig: it produces no item at all.
#[tokio::test]
async fn anthropic_pings_count_as_signs_of_life() {
    let ping = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
    let base = server("", ping, 12, ANTHROPIC_ANSWER).await;
    let chunks = collect(stream(ProviderKind::Anthropic, base).await, |c| {
        c.iter().any(|x| matches!(x, StreamChunk::ContentDelta(_)))
    })
    .await;
    assert!(keepalives(&chunks) >= 1, "{chunks:?}");
}

/// The same holds on the plain chat-completions wire.
#[tokio::test]
async fn the_chat_completions_wire_reports_keepalive_too() {
    let base = server("", ": keep-alive\n\n", 12, "data: [DONE]\n\n").await;
    let chunks = collect(stream(ProviderKind::OpenaiCompat, base).await, |c| {
        keepalives(c) >= 1
    })
    .await;
    assert!(keepalives(&chunks) >= 1, "{chunks:?}");
}

/// A connection that goes quiet for real earns nothing: the sampler's own idle
/// ceiling is what ends it, so this must not be papered over.
#[tokio::test]
async fn a_dead_connection_never_reports_keepalive() {
    let base = server("", "", 0, "").await;
    let chunks = collect(stream(ProviderKind::OpenaiResponses, base).await, |_| false).await;
    assert_eq!(keepalives(&chunks), 0, "{chunks:?}");
}

/// A stream that is producing content needs no reminder.
#[tokio::test]
async fn a_chatty_stream_is_not_padded_with_keepalives() {
    let base = server("", ANSWER, 12, ANSWER).await;
    let chunks = collect(stream(ProviderKind::OpenaiResponses, base).await, |c| {
        c.iter()
            .filter(|x| matches!(x, StreamChunk::ContentDelta(_)))
            .count()
            >= 13
    })
    .await;
    assert_eq!(keepalives(&chunks), 0, "{chunks:?}");
}
