//! What a Responses request body says about the thinking that led to a tool
//! call, captured off the wire.
//!
//! DeepSeek's thinking mode validates the turn it is being asked to continue:
//! a step that calls a tool, or that answers one, must open with a reasoning
//! item carrying non-empty `reasoning_text`, or the whole request is refused
//! with 400 "The `reasoning_text` in the thinking mode must be passed back to
//! the API". rig drops a reasoning item that has no provider id, so the
//! provider mints one for the wire and the outbound patch removes it again.
//! The repair is opt-in through `requires_reasoning_content_on_assistant_messages`
//! — the model name plays no part.
//!
//! The rule is only observable in the request body, so these tests serve one
//! and read it.

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use agent_llm::types::ChatMessage;
use agent_llm::{ModelParams, ProviderCompat, ProviderConfig, ProviderFactory, ProviderKind};

/// A throwaway HTTP server: every request body it receives is recorded, and
/// each connection is answered with a 400 so the provider stops without
/// waiting for a real completion.
async fn capture_server() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let sink = sink.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let Some(split) = head.windows(4).position(|w| w == b"\r\n\r\n") else {
                    return;
                };
                let head_text = String::from_utf8_lossy(&head[..split]).to_lowercase();
                let declared = head_text
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = head[split + 4..].to_vec();
                while body.len() < declared {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&buf[..n]),
                    }
                }
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) {
                    sink.lock().unwrap().push(value);
                }
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            });
        }
    });
    (format!("http://{addr}/v1"), seen)
}

fn config(base_url: String, replay: Option<bool>) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::OpenaiResponses,
        base_url,
        api_key: "test-key".into(),
        compat: replay.map(|on| ProviderCompat {
            requires_reasoning_content_on_assistant_messages: Some(on),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The reporter's history: the model thought, called a tool, thought again,
/// said something, then called a second tool. Both steps must reach the wire
/// with their thinking in front.
async fn request_body(
    model: &str,
    replay: Option<bool>,
    effort: Option<&str>,
    a_real_tool_history: bool,
) -> serde_json::Value {
    let (base, seen) = capture_server().await;
    let provider = ProviderFactory::build(&config(base, replay)).unwrap();

    let mut first = ChatMessage::assistant("");
    first.reasoning = Some("check the suite".into());
    first.tool_calls = Some(vec![agent_llm::types::ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }]);
    let mut second = ChatMessage::assistant("");
    second.reasoning = Some("read the failure".into());
    second.tool_calls = Some(vec![agent_llm::types::ToolCall {
        id: "c2".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }]);

    let messages = if a_real_tool_history {
        vec![
            ChatMessage::user("run the tests"),
            first,
            ChatMessage::tool_result("c1", "ok"),
            second,
            ChatMessage::tool_result("c2", "ok"),
        ]
    } else {
        vec![ChatMessage::user("hi"), ChatMessage::assistant("hello")]
    };

    let mut stream = provider
        .chat_stream(model, &messages, None, 1.0, effort, &ModelParams::EMPTY)
        .await
        .expect("stream");
    while stream.next().await.is_some() {}
    let body = seen.lock().unwrap().last().cloned();
    body.expect("the server saw no request")
}

fn item_types(body: &serde_json::Value) -> Vec<String> {
    body["input"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let kind = item["type"].as_str().unwrap_or_default();
                    match kind {
                        "message" => format!("message:{}", item["role"].as_str().unwrap_or("")),
                        other => other.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A backend that declares reasoning replay gets each tool-using step's
/// thinking back, in front of the step, and no fabricated reasoning id
/// survives onto the wire.
#[tokio::test]
async fn a_declared_backend_gets_reasoning_in_front_of_every_tool_step() {
    let body = request_body("any-thinking-model", Some(true), Some("max"), true).await;
    let kinds = item_types(&body);
    assert_eq!(
        kinds,
        vec![
            "message:user",
            "reasoning",
            "function_call",
            "function_call_output",
            "reasoning",
            "function_call",
            "function_call_output",
        ],
        "{body}"
    );
    // The thinking itself travelled, and the sentinel id did not.
    for item in body["input"].as_array().unwrap() {
        if item["type"] == "reasoning" {
            assert!(item.get("id").is_none(), "fabricated id leaked: {item}");
            let text = item["content"][0]["text"].as_str().unwrap_or_default();
            assert!(!text.is_empty(), "empty reasoning_text: {item}");
        }
    }
    assert!(body["input"][1]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("check the suite"));
}

/// Nothing declared means nothing synthesized — even for a model whose name
/// says DeepSeek. The user's config is the only switch.
#[tokio::test]
async fn an_undeclared_backend_gets_no_synthesized_reasoning() {
    for replay in [None, Some(false)] {
        let body = request_body("deepseek-v4.1-flash", replay, Some("max"), true).await;
        assert!(
            item_types(&body).iter().all(|kind| kind != "reasoning"),
            "{body}"
        );
    }
}

/// With thinking off the rule does not apply, so a declared backend is left
/// alone too.
#[tokio::test]
async fn thinking_off_skips_the_replay() {
    for effort in [None, Some("none")] {
        let body = request_body("any-thinking-model", Some(true), effort, true).await;
        assert!(
            item_types(&body).iter().all(|kind| kind != "reasoning"),
            "{body}"
        );
    }
}

/// A plain exchange has no tool step to protect and is not rewritten.
#[tokio::test]
async fn a_conversation_without_tools_is_left_alone() {
    let body = request_body("any-thinking-model", Some(true), Some("max"), false).await;
    assert_eq!(
        item_types(&body),
        vec!["message:user", "message:assistant"],
        "{body}"
    );
}
