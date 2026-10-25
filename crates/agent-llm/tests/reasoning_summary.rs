//! Models that think without showing it leave the user staring at a blank wait.
//! Where a wire can return a reasoning summary, the request asks for one; a
//! backend that refuses the ask is retried once without it. The request body is
//! the only place this is observable, so these tests serve one and read it.

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use agent_llm::types::{ChatMessage, StreamChunk};
use agent_llm::{ModelParams, ProviderCompat, ProviderConfig, ProviderFactory, ProviderKind};

struct Reply {
    status: u16,
    sse: &'static str,
}

const REFUSED: Reply = Reply {
    status: 400,
    sse: "",
};
const UNAUTHORIZED: Reply = Reply {
    status: 401,
    sse: "",
};
const ANSWER: Reply = Reply {
    status: 200,
    sse: "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"sequence_number\":1,\"delta\":\"hi\"}\n\n",
};

/// Records every request body; `reply` decides the answer from the body.
async fn server(
    reply: impl Fn(&serde_json::Value) -> Reply + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (sink, reply) = (sink.clone(), reply.clone());
            tokio::spawn(async move {
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
                let mut body = head[split + 4..].to_vec();
                while body.len() < declared {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&buf[..n]),
                    }
                }
                let value = serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default();
                let Reply { status, sse } = reply(&value);
                sink.lock().unwrap().push(value);
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    sse.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(sse.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}/v1"), seen)
}

fn config(kind: ProviderKind, base_url: String, replay: bool) -> ProviderConfig {
    ProviderConfig {
        kind,
        base_url,
        compat: replay.then(|| ProviderCompat {
            requires_reasoning_content_on_assistant_messages: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Drives one request to the end and returns what the provider streamed.
async fn run(
    kind: ProviderKind,
    base: String,
    effort: Option<&str>,
    replay: bool,
) -> Vec<StreamChunk> {
    let provider = ProviderFactory::build(&config(kind, base, replay)).unwrap();
    let messages = vec![ChatMessage::user("hi")];
    let Ok(mut stream) = provider
        .chat_stream("m", &messages, None, 0.0, effort, &ModelParams::EMPTY)
        .await
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(chunk) = item {
            out.push(chunk);
        }
    }
    out
}

/// The request as first sent. The server refuses everything, so a fallback
/// retry may follow it; that one is checked on its own.
async fn first_request(
    kind: ProviderKind,
    effort: Option<&str>,
    replay: bool,
) -> serde_json::Value {
    let (base, seen) = server(|_| REFUSED).await;
    run(kind, base, effort, replay).await;
    let body = seen.lock().unwrap().first().cloned();
    body.expect("the server saw no request")
}

#[tokio::test]
async fn anthropic_can_explicitly_select_budget_thinking_without_affecting_other_wires() {
    for adaptive in [false, true] {
        let (base, seen) = server(|_| REFUSED).await;
        let mut cfg = config(ProviderKind::Anthropic, base, false);
        cfg.compat = Some(ProviderCompat {
            force_adaptive_thinking: Some(adaptive),
            ..Default::default()
        });
        let provider = ProviderFactory::build(&cfg).unwrap();
        let params = ModelParams {
            max_tokens: Some(4096),
            ..ModelParams::EMPTY
        };
        if let Ok(mut stream) = provider
            .chat_stream(
                "configured-claude",
                &[ChatMessage::user("hi")],
                None,
                0.7,
                Some("high"),
                &params,
            )
            .await
        {
            while stream.next().await.is_some() {}
        }
        let body = seen
            .lock()
            .unwrap()
            .first()
            .cloned()
            .expect("the server saw no request");
        assert_eq!(body["max_tokens"], 4096);
        if adaptive {
            assert_eq!(body["thinking"]["type"], "adaptive");
            assert_eq!(body["output_config"]["effort"], "high");
        } else {
            assert_eq!(
                body["thinking"]["type"], "enabled",
                "explicit non-adaptive models must not receive adaptive thinking"
            );
            let budget = body["thinking"]["budget_tokens"].as_u64().unwrap();
            assert!((1024..4096).contains(&budget));
            assert!(body.get("output_config").is_none());
        }
        assert!(
            body.get("temperature").is_none(),
            "Anthropic thinking still excludes temperature"
        );
        assert!(body.get("reasoning").is_none());
    }
}

#[tokio::test]
async fn an_impossible_anthropic_budget_is_reported_without_disabling_thinking() {
    let (base, seen) = server(|_| REFUSED).await;
    let mut cfg = config(ProviderKind::Anthropic, base, false);
    cfg.compat = Some(ProviderCompat {
        force_adaptive_thinking: Some(false),
        ..Default::default()
    });
    let provider = ProviderFactory::build(&cfg).unwrap();
    let params = ModelParams {
        max_tokens: Some(1024),
        ..ModelParams::EMPTY
    };
    let error = match provider
        .chat_stream(
            "budget-model",
            &[ChatMessage::user("hi")],
            None,
            0.0,
            Some("high"),
            &params,
        )
        .await
    {
        Ok(_) => panic!("an invalid thinking budget must not be sent or silently disabled"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("max_tokens > 1024"));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_temperature_opt_out_is_enforced_on_every_protocol() {
    for kind in [
        ProviderKind::OpenaiCompat,
        ProviderKind::OpenaiResponses,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
    ] {
        let (base, seen) = server(|_| REFUSED).await;
        let mut cfg = config(kind, base, false);
        cfg.compat = Some(ProviderCompat {
            supports_temperature: Some(false),
            force_adaptive_thinking: Some(true),
            supports_mid_convo_effort: Some(true),
            ..Default::default()
        });
        let provider = ProviderFactory::build(&cfg).unwrap();
        let params = ModelParams {
            sampling_params: Some(serde_json::json!({"temperature":0.6})),
            ..ModelParams::EMPTY
        };
        if let Ok(mut stream) = provider
            .chat_stream(
                "configured-model",
                &[ChatMessage::user("hi")],
                None,
                0.7,
                None,
                &params,
            )
            .await
        {
            while stream.next().await.is_some() {}
        }
        let body = seen
            .lock()
            .unwrap()
            .first()
            .cloned()
            .expect("the server saw no request");
        assert!(body.get("temperature").is_none(), "{kind:?}: {body}");
        assert!(
            body["generationConfig"].get("temperature").is_none(),
            "{kind:?}: {body}"
        );
        if kind != ProviderKind::Anthropic {
            assert!(
                body.get("thinking").is_none(),
                "Claude thinking must not leak to {kind:?}"
            );
        }
    }
}

#[tokio::test]
async fn each_wire_asks_for_a_summary_when_thinking_is_on() {
    let body = first_request(ProviderKind::OpenaiResponses, Some("high"), false).await;
    assert_eq!(body["reasoning"]["effort"], "high", "{body}");
    assert_eq!(body["reasoning"]["summary"], "auto", "{body}");

    let body = first_request(ProviderKind::Anthropic, Some("high"), false).await;
    assert_eq!(body["thinking"]["type"], "adaptive", "{body}");
    assert_eq!(body["thinking"]["display"], "summarized", "{body}");
    assert_eq!(body["output_config"]["effort"], "high", "{body}");

    let body = first_request(ProviderKind::Gemini, Some("high"), false).await;
    let config = &body["generationConfig"]["thinkingConfig"];
    assert_eq!(config["includeThoughts"], true, "{body}");
    assert_eq!(config["thinkingBudget"], 24_576, "{body}");
}

#[tokio::test]
async fn nothing_is_asked_when_thinking_is_off() {
    for kind in [
        ProviderKind::OpenaiResponses,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
    ] {
        let body = first_request(kind, None, false).await;
        let text = body.to_string();
        for key in ["\"summary\"", "\"display\"", "includeThoughts"] {
            assert!(!text.contains(key), "{kind:?} sent {key}: {text}");
        }
    }
}

/// A backend that replays reasoning in thinking mode is a strict third-party
/// shim; it gets the request exactly as before.
#[tokio::test]
async fn a_replaying_backend_is_not_asked_for_a_summary() {
    let body = first_request(ProviderKind::OpenaiResponses, Some("high"), true).await;
    assert_eq!(
        body["reasoning"],
        serde_json::json!({"effort": "high"}),
        "{body}"
    );
}

#[tokio::test]
async fn a_refused_summary_ask_is_retried_once_without_it() {
    let (base, seen) = server(|body| {
        if body["reasoning"].get("summary").is_some() {
            REFUSED
        } else {
            ANSWER
        }
    })
    .await;
    let chunks = run(ProviderKind::OpenaiResponses, base, Some("high"), false).await;

    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ContentDelta(t) if t == "hi")),
        "the answer never arrived: {chunks:?}"
    );
    assert!(
        !chunks.iter().any(|c| matches!(c, StreamChunk::Error(_))),
        "the refusal leaked to the caller: {chunks:?}"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(
        seen[1]["reasoning"],
        serde_json::json!({"effort": "high"}),
        "{:?}",
        seen[1]
    );
}

/// An auth failure would fail the plain request the same way: no second try.
#[tokio::test]
async fn other_failures_are_not_retried() {
    let (base, seen) = server(|_| UNAUTHORIZED).await;
    run(ProviderKind::OpenaiResponses, base, Some("high"), false).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Every wire reports a refusal the same way to the fallback: the second
/// request goes out without the summary ask and keeps the effort settings.
#[tokio::test]
async fn every_wire_retries_a_refusal_without_the_summary_ask() {
    for kind in [ProviderKind::Anthropic, ProviderKind::Gemini] {
        let (base, seen) = server(|_| REFUSED).await;
        run(kind, base, Some("high"), false).await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "{kind:?} did not retry: {seen:?}");
        let (first, second) = (seen[0].to_string(), seen[1].to_string());
        assert!(
            first.contains("summarized") || first.contains("includeThoughts"),
            "{first}"
        );
        for key in ["summarized", "includeThoughts"] {
            assert!(!second.contains(key), "{kind:?} retry still asks: {second}");
        }
        assert!(
            second.contains("effort") || second.contains("thinkingBudget"),
            "{second}"
        );
    }
}
