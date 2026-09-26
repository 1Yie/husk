//! Wiring assertion for `ProviderCompat` — the table was imported wholesale
//! from the upstream schema and `Option<bool>` fields make "never read" and
//! "defaults to unset" indistinguishable at runtime. This test is the forcing
//! function the review asked for:
//!
//!   a field must EITHER have a reader outside `config.rs` (adapter call or a
//!   helper that adapters call) OR sit in `KNOWN_DEAD` — declared upstream,
//!   not wired here yet. Adding a field without a consumer fails red; wiring
//!   a dead field means removing it from `KNOWN_DEAD`, which is how the debt
//!   list only ever shrinks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Every `pub` field on `ProviderCompat`, in declaration order. Adding a row
/// is deliberate: the new field must be wired or explicitly debt-listed.
const COMPAT_FIELDS: &[&str] = &[
    "supports_store",
    "supports_developer_role",
    "supports_reasoning_effort",
    "max_tokens_field",
    "supports_usage_in_streaming",
    "supports_finish_reason",
    "requires_tool_result_name",
    "requires_assistant_after_tool_result",
    "requires_thinking_as_text",
    "requires_reasoning_content_on_assistant_messages",
    "thinking_format",
    "chat_template_kwargs",
    "chat_template_args",
    "thinking_token_budget_field",
    "supports_thinking_token_budget",
    "cache_control_format",
    "send_session_affinity_headers",
    "session_affinity_format",
    "supports_strict_mode",
    "supports_openai_grammar_tools",
    "supports_long_cache_retention",
    "open_router_routing",
    "vercel_gateway_routing",
    "supports_eager_tool_input_streaming",
    "supports_cache_control_on_tools",
    "force_adaptive_thinking",
    "supports_mid_convo_effort",
    "allow_empty_signature",
    "supports_strict_tools",
    "allowed_fallback_models",
];

/// Fields whose read path is a `ProviderCompat` helper rather than the field
/// name itself — the helper being called IS the wiring.
const HELPER_READERS: &[(&str, &str)] = &[
    ("supports_developer_role", "developer_role"),
    ("max_tokens_field", "max_tokens_field_name"),
    ("thinking_format", "thinking_format_name"),
    (
        "thinking_token_budget_field",
        "thinking_token_budget_field_name",
    ),
];

/// Declared-but-unwired surface — accepted debt, not invisible debt. Each
/// entry means the field has zero readers anywhere in the workspace.
const KNOWN_DEAD: &[&str] = &[
    "supports_usage_in_streaming",
    "supports_finish_reason",
    "requires_thinking_as_text",
    "thinking_token_budget_field",
    "supports_thinking_token_budget",
    "cache_control_format",
    "send_session_affinity_headers",
    "session_affinity_format",
    "supports_strict_mode",
    "supports_openai_grammar_tools",
    "supports_long_cache_retention",
    "open_router_routing",
    "vercel_gateway_routing",
    "supports_eager_tool_input_streaming",
    "supports_cache_control_on_tools",
    "force_adaptive_thinking",
    "supports_mid_convo_effort",
    "allow_empty_signature",
    "supports_strict_tools",
    "allowed_fallback_models",
];

fn src_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("src dir readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            src_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|n| n != "config.rs")
        {
            out.push(path);
        }
    }
}

#[test]
fn every_compat_field_is_wired_or_listed_as_debt() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    // Consumers live in this crate's adapters/factory and in the kernel.
    src_files(&manifest.join("src"), &mut files);
    src_files(&manifest.join("../agent-kernel/src"), &mut files);
    let sources: Vec<String> = files
        .iter()
        .map(|p| std::fs::read_to_string(p).expect("source file readable"))
        .collect();

    let dead: HashSet<&str> = KNOWN_DEAD.iter().copied().collect();
    let fields: HashSet<&str> = COMPAT_FIELDS.iter().copied().collect();
    assert_eq!(
        fields.len(),
        COMPAT_FIELDS.len(),
        "duplicate entry in COMPAT_FIELDS"
    );
    for f in KNOWN_DEAD {
        assert!(fields.contains(f), "KNOWN_DEAD names a non-field: {f}");
    }

    let mut unlisted_dead = Vec::new();
    let mut wired_but_listed = Vec::new();
    for field in COMPAT_FIELDS {
        let direct = sources.iter().any(|s| s.contains(field));
        let via_helper = HELPER_READERS
            .iter()
            .find(|(f, _)| f == field)
            .is_some_and(|(_, h)| sources.iter().any(|s| s.contains(&format!("{h}("))));
        let wired = direct || via_helper;
        if wired && dead.contains(field) {
            wired_but_listed.push(*field);
        } else if !wired && !dead.contains(field) {
            unlisted_dead.push(*field);
        }
    }

    assert!(
        unlisted_dead.is_empty(),
        "ProviderCompat fields with no reader and no KNOWN_DEAD entry — wire \
         them or list them as debt: {unlisted_dead:?}"
    );
    assert!(
        wired_but_listed.is_empty(),
        "fields wired since listing — remove them from KNOWN_DEAD: {wired_but_listed:?}"
    );
}
