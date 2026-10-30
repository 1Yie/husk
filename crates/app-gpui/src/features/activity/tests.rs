use super::{Detail, Pending, accept_reply, bounded, escape, transcript_text};
use crate::bridge::SessionKey;
use std::collections::HashMap;

#[test]
fn replies_cannot_cross_sessions_or_overwrite_newer_requests() {
    let key = SessionKey {
        root: "/workspace".into(),
        id: 1,
    };
    let other = SessionKey {
        root: "/workspace".into(),
        id: 2,
    };
    let pending = Pending {
        key: key.clone(),
        slot: "detail",
        detail: Some(Detail::Child(1)),
    };
    let latest = HashMap::from([("detail", 7)]);
    assert!(accept_reply(&pending, &Some(key.clone()), &key, 7, &latest));
    assert!(!accept_reply(
        &pending,
        &Some(other.clone()),
        &key,
        7,
        &latest
    ));
    assert!(!accept_reply(
        &pending,
        &Some(key.clone()),
        &other,
        7,
        &latest
    ));
    assert!(!accept_reply(
        &pending,
        &Some(key.clone()),
        &key,
        6,
        &latest
    ));
    assert!(!accept_reply(&pending, &None, &key, 7, &latest));
}

#[test]
fn text_is_escaped_bounded_and_utf8_safe() {
    assert_eq!(escape("<script>&"), "&lt;script&gt;&amp;");
    assert!(bounded("中文中文", 5).starts_with("中\n"));
    assert_eq!(bounded("hello", 5), "hello");
}

#[test]
fn transcript_includes_tool_calls_without_rendering_them_as_markup() {
    let messages = vec![serde_json::json!({
        "role":"assistant", "content":"actual report",
        "tool_calls":[{"function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]
    })];
    let text = transcript_text(&messages);
    assert!(text.contains("[assistant]"));
    assert!(text.contains("actual report"));
    assert!(text.contains("工具 read"));
}
