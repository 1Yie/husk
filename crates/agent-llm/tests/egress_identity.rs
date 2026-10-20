//! What a request says about itself, captured off the wire.
//!
//! Standalone gateways (OpenCode Go and friends) route, meter and abuse-filter
//! by client: they ask a client to name itself with its own `User-Agent` and to
//! keep one stable session id per conversation. Both facts are only observable
//! in the request head, so this test serves one and reads it.

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use agent_llm::types::ChatMessage;
use agent_llm::{ModelParams, ProviderCompat, ProviderConfig, ProviderFactory, ProviderKind};

/// A throwaway HTTP server: every request head it sees is recorded, and each
/// connection is answered with a 400 so the provider stops without waiting for
/// a real completion.
async fn capture_server() -> (String, Arc<Mutex<Vec<String>>>) {
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
                let mut buf = vec![0u8; 8192];
                let mut head = Vec::new();
                // Only the head matters; the body is ignored and cut off by the
                // 400 below.
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).to_lowercase());
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

/// Run one request to completion (its failure is expected) and return the head
/// the server saw.
async fn request_head(
    provider: Arc<dyn agent_llm::LlmProvider>,
    seen: Arc<Mutex<Vec<String>>>,
) -> String {
    let mut stream = provider
        .chat_stream(
            "m",
            &[ChatMessage::user("hi")],
            None,
            1.0,
            None,
            &ModelParams::EMPTY,
        )
        .await
        .expect("stream");
    while stream.next().await.is_some() {}
    let head = seen.lock().unwrap().last().cloned();
    head.expect("the server saw no request")
}

fn config(base_url: String, affinity: Option<bool>) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::OpenaiCompat,
        base_url,
        api_key: "test-key".into(),
        compat: affinity.map(|send| ProviderCompat {
            send_session_affinity_headers: Some(send),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The client names itself, not its HTTP library, and every request carries
/// the conversation's routing key under the names a gateway looks for.
#[tokio::test]
async fn a_request_names_the_client_and_its_conversation() {
    let (base, seen) = capture_server().await;
    let cfg = config(base, None);
    agent_llm::factory::set_client_user_agent("husk/9.9.9");
    let provider =
        ProviderFactory::build_with_conversation(&cfg, None, Some("7:/workspace")).unwrap();
    let head = request_head(provider, seen).await;

    // The binary's declared identity, not this crate's version.
    assert!(head.contains("user-agent: husk/9.9.9"), "{head}");
    // The default set: namespaced names only, no request-scoped ones.
    for name in ["x-session-affinity", "x-session-id", "x-opencode-session"] {
        let expected = format!("{name}: husk-");
        assert!(head.contains(&expected), "missing {expected} in {head}");
    }
}

/// `userAgent` in config wins over the client default: a deployment that
/// validates clients by name sees the name it asked for.
#[tokio::test]
async fn a_configured_user_agent_overrides_the_default() {
    let (base, seen) = capture_server().await;
    let mut cfg = config(base, None);
    cfg.user_agent = Some("my-coding-agent/1.0".into());
    let provider = ProviderFactory::build(&cfg).unwrap();
    let head = request_head(provider, seen).await;
    assert!(head.contains("user-agent: my-coding-agent/1.0"), "{head}");
}
/// The session headers go out unless the deployment says otherwise — the only
/// way to lose them is the explicit switch, and even then the client still
/// names itself.
#[tokio::test]
async fn an_explicit_off_is_the_only_thing_that_silences_the_session_headers() {
    let (base, seen) = capture_server().await;
    let cfg = config(base, Some(false));
    let provider =
        ProviderFactory::build_with_conversation(&cfg, None, Some("7:/workspace")).unwrap();
    let head = request_head(provider, seen).await;
    for name in ["x-session-affinity", "x-session-id", "x-opencode-session"] {
        assert!(!head.contains(&format!("{name}:")), "{name} leaked: {head}");
    }
    assert!(head.contains("user-agent: husk/"), "{head}");
}

/// A gateway whose dialect nobody has heard of: the provider names the header
/// itself, and the conversation key rides it — the built-in trio steps aside.
#[tokio::test]
async fn a_custom_session_header_from_config_reaches_the_wire() {
    let (base, seen) = capture_server().await;
    let mut cfg = config(base, None);
    cfg.session_affinity_headers = vec!["x-acme-session".into()];
    let provider =
        ProviderFactory::build_with_conversation(&cfg, None, Some("7:/workspace")).unwrap();
    let head = request_head(provider, seen).await;
    assert!(head.contains("x-acme-session: husk-"), "{head}");
    assert!(!head.contains("x-opencode-session:"), "{head}");
}

/// One conversation, one routing key: rebuilding the provider — a model switch,
/// a fallback hop — must not hand the gateway a different session id, or the
/// prompt cache it already warmed is unreachable. Two conversations differ.
#[tokio::test]
async fn the_same_conversation_keeps_its_routing_key_across_rebuilds() {
    let key = |head: &str| {
        head.lines()
            .find_map(|l| l.strip_prefix("x-session-affinity: "))
            .map(str::to_string)
    };

    let (base, seen) = capture_server().await;
    let cfg = config(base, None);
    let first = ProviderFactory::build_with_conversation(&cfg, None, Some("7:/workspace"));
    let head = request_head(first.unwrap(), seen.clone()).await;
    let first_key = key(&head).expect("a routing key");

    // A rebuild of the same conversation — new provider instance, same key.
    let second = ProviderFactory::build_with_conversation(&cfg, None, Some("7:/workspace"));
    let head = request_head(second.unwrap(), seen.clone()).await;
    assert_eq!(key(&head).as_deref(), Some(first_key.as_str()));

    // A different conversation gets its own.
    let other = ProviderFactory::build_with_conversation(&cfg, None, Some("8:/workspace"));
    let head = request_head(other.unwrap(), seen).await;
    assert_ne!(key(&head).as_deref(), Some(first_key.as_str()));
}
