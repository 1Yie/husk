//! Real HTTP requests through the public factory, not synthetic body builders.
//! A local server captures each complete request then rejects it deliberately.
use std::collections::HashMap;
use std::time::Duration;

use agent_llm::{
    ChatMessage, ModelConfig, ModelParams, ProviderConfig, ProviderFactory, ProviderKind,
};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug)]
struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

fn config(kind: ProviderKind, fast: Value) -> ProviderConfig {
    ProviderConfig {
        kind,
        models: vec![serde_json::from_value::<ModelConfig>(json!({
            "id":"logical", "reasoning":true,
            "samplingParams":{"top_p":0.7, "service_tier":"auto"},
            "fastMode":fast
        }))
        .unwrap()
        .into()],
        ..Default::default()
    }
}

async fn capture(
    cfg: &ProviderConfig,
    enabled: Option<bool>,
    model: &str,
    effort: Option<&str>,
) -> Request {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = cfg.clone();
    cfg.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        let split = loop {
            if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break split;
            }
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0, "missing HTTP headers");
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
            .into();
        let headers: HashMap<String, String> = head
            .lines()
            .skip(1)
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_ascii_lowercase(), value.trim().into())
            })
            .collect();
        let length: usize = headers["content-length"].parse().unwrap();
        while bytes.len() < split + 4 + length {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0, "missing HTTP body");
            bytes.extend_from_slice(&chunk[..n]);
        }
        let body = serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap();
        let error = r#"{"error":{"message":"captured","type":"invalid_request_error"}}"#;
        let reply = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            error.len(), error
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        Request {
            path,
            headers,
            body,
        }
    });
    let provider = match enabled {
        Some(enabled) => {
            ProviderFactory::build_with_fast_mode(&cfg, None, Some("fast-fixture"), enabled)
        }
        None => ProviderFactory::build_with_conversation(&cfg, None, Some("fast-fixture")),
    }
    .unwrap();
    let params = ProviderFactory::model_params(&cfg, model);
    tokio::time::timeout(Duration::from_secs(5), async {
        if let Ok(mut stream) = provider
            .chat_stream(
                model,
                &[ChatMessage::user("hello")],
                None,
                0.4,
                effort,
                &params,
            )
            .await
        {
            while stream.next().await.is_some() {}
        }
        server.await.unwrap()
    })
    .await
    .expect("request must reach fixture")
}

#[tokio::test]
async fn priority_reaches_both_openai_wires_after_sampling_without_changing_thinking() {
    for (kind, path) in [
        (ProviderKind::OpenaiCompat, "/v1/chat/completions"),
        (ProviderKind::OpenaiResponses, "/v1/responses"),
    ] {
        let cfg = config(kind, json!({"mode":"priority"}));
        let fast = capture(&cfg, Some(true), "logical", Some("high")).await;
        let base = capture(&cfg, Some(false), "logical", Some("high")).await;
        assert_eq!(fast.path, path);
        assert_eq!(fast.body["model"], "logical");
        assert_eq!(fast.body["service_tier"], "priority");
        assert_eq!(base.body["service_tier"], "auto");
        assert_eq!(fast.body["top_p"], base.body["top_p"]);
        assert_eq!(fast.body["reasoning"], base.body["reasoning"]);
        assert_eq!(fast.body["reasoning_effort"], base.body["reasoning_effort"]);
        if kind == ProviderKind::OpenaiResponses {
            assert_eq!(fast.body["reasoning"]["effort"], "high");
        } else {
            assert_eq!(fast.body["reasoning_effort"], "high");
        }
        assert!(fast.body.get("fast_mode").is_none());
        assert!(fast.body.get("fastMode").is_none());
    }
}

#[tokio::test]
async fn anthropic_speed_merges_case_insensitive_beta_flags_and_preserves_thinking() {
    let mut cfg = config(
        ProviderKind::Anthropic,
        json!({
            "mode":"speed", "headers":{"ANTHROPIC-BETA":"fast-mode-2026-02-01,custom-beta", "X-Fast":"1"}
        }),
    );
    cfg.headers
        .insert("Anthropic-Beta".into(), "base-beta".into());
    let model = cfg.models[0].detailed().unwrap().clone();
    cfg.models = vec![ModelConfig {
        headers: Some(HashMap::from([(
            "anthropic-beta".into(),
            "model-beta,base-beta".into(),
        )])),
        ..model
    }
    .into()];
    let fast = capture(&cfg, Some(true), "logical", Some("high")).await;
    let base = capture(&cfg, Some(false), "logical", Some("high")).await;
    assert_eq!(fast.path, "/v1/messages");
    assert_eq!(fast.body["speed"], "fast");
    assert!(base.body.get("speed").is_none());
    let flags: Vec<_> = fast.headers["anthropic-beta"].split(',').collect();
    for flag in [
        "base-beta",
        "model-beta",
        "custom-beta",
        "fast-mode-2026-02-01",
    ] {
        assert_eq!(flags.iter().filter(|f| **f == flag).count(), 1, "{flags:?}");
    }
    assert_eq!(fast.headers["x-fast"], "1");
    assert!(!base.headers.contains_key("x-fast"));
    assert_eq!(fast.body["thinking"], base.body["thinking"]);
    assert_eq!(fast.body["output_config"], base.body["output_config"]);
    assert_eq!(fast.body["thinking"]["type"], "adaptive");
}

#[tokio::test]
async fn explicit_wire_model_mapping_never_changes_logical_config_or_guesses_ids() {
    for kind in [ProviderKind::OpenaiCompat, ProviderKind::OpenaiResponses] {
        let cfg = config(kind, json!({"mode":"model","model":"explicit-highspeed"}));
        let fast = capture(&cfg, Some(true), "logical", Some("medium")).await;
        let off = capture(&cfg, Some(false), "logical", Some("medium")).await;
        let unsupported = capture(&cfg, Some(true), "unconfigured-turbo", None).await;
        assert_eq!(fast.body["model"], "explicit-highspeed");
        assert_eq!(off.body["model"], "logical");
        assert_eq!(unsupported.body["model"], "unconfigured-turbo");
        assert!(unsupported.body.get("speed").is_none());
        assert!(unsupported.body.get("service_tier").is_none());
        assert_eq!(cfg.models[0].id(), "logical");
    }
}

#[tokio::test]
async fn custom_params_and_headers_override_base_on_all_http_adapters() {
    for kind in [
        ProviderKind::OpenaiCompat,
        ProviderKind::OpenaiResponses,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
    ] {
        let mut cfg = config(
            kind,
            json!({
                "mode":"custom", "params":{"service_tier":"express", "gateway_fast":{"enabled":true}},
                "headers":{"x-routing":"fast"}
            }),
        );
        cfg.headers.insert("X-Routing".into(), "normal".into());
        let fast = capture(&cfg, Some(true), "logical", None).await;
        let off = capture(&cfg, Some(false), "logical", None).await;
        assert_eq!(fast.body["service_tier"], "express", "{kind:?}");
        assert_eq!(
            fast.body["gateway_fast"],
            json!({"enabled":true}),
            "{kind:?}"
        );
        assert!(off.body.get("gateway_fast").is_none(), "{kind:?}");
        assert_eq!(fast.headers["x-routing"], "fast");
        assert_eq!(off.headers["x-routing"], "normal");
    }
}

#[tokio::test]
async fn factory_default_off_and_unsupported_requests_match_base_exactly() {
    let cfg = config(
        ProviderKind::OpenaiResponses,
        json!({
            "mode":"priority", "params":{"top_p":0.1}, "headers":{"x-fast":"1"}
        }),
    );
    let default = capture(&cfg, None, "logical", Some("medium")).await;
    let off = capture(&cfg, Some(false), "logical", Some("medium")).await;
    assert_eq!(default.body, off.body);
    assert!(!off.headers.contains_key("x-fast"));
    assert_eq!(off.body["top_p"], json!(0.7));
    let unsupported_off = capture(&cfg, Some(false), "other", Some("medium")).await;
    let unsupported_fast = capture(&cfg, Some(true), "other", Some("medium")).await;
    assert_eq!(unsupported_off.body, unsupported_fast.body);
    assert!(!unsupported_fast.headers.contains_key("x-fast"));
}

#[tokio::test]
async fn effective_model_override_replaces_mode_and_applies_only_to_its_logical_id() {
    let mut cfg = config(
        ProviderKind::OpenaiResponses,
        json!({
            "mode":"model", "model":"old-turbo", "params":{"old_fast":true}, "headers":{"x-old-fast":"1"}
        }),
    );
    cfg.model_overrides = Some(HashMap::from([(
        "logical".into(),
        serde_json::from_value(json!({
            "fastMode":{"mode":"priority", "params":{"new_fast":true}}
        }))
        .unwrap(),
    )]));
    let fast = capture(&cfg, Some(true), "logical", Some("medium")).await;
    assert_eq!(fast.body["model"], "logical");
    assert_eq!(fast.body["service_tier"], "priority");
    assert_eq!(fast.body["new_fast"], true);
    assert!(fast.body.get("old_fast").is_none());
    assert!(!fast.headers.contains_key("x-old-fast"));
    assert_eq!(fast.body["reasoning"]["effort"], "medium");
}

#[test]
fn model_params_contract_never_carries_app_fast_mode() {
    let cfg = config(ProviderKind::OpenaiCompat, json!({"mode":"priority"}));
    let params: ModelParams = ProviderFactory::model_params(&cfg, "logical");
    assert_eq!(
        params.sampling_params,
        Some(json!({"top_p":0.7,"service_tier":"auto"}))
    );
}
