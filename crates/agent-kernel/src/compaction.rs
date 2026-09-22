//! `compaction` — two-pass history summarization near the context window.
//!
//! Contract (kernel-architecture.md §Compaction):
//!
//! - Trigger: estimated tokens ≈ 80% of `context_window`. Optional prefire
//!   starts Pass 1 at `PREFIRE_LEAD_PERCENT` (70%) so the latency is hidden
//!   behind the next turn.
//! - Two-pass: split history into prefix/suffix at the midpoint → summarize
//!   the prefix into `NOTE₁` (a dense context note) → condense `NOTE₁` +
//!   suffix so the total fits the window again.
//! - Sanitize pipeline runs *before* every sample (not just at compaction):
//!   flatten tool calls, gate image refs on model
//!   modality, `fit_conversation_to_budget`.
//! - Sticky suppression (`SUPPRESS_STICKY`, `SUPPRESS_UNTIL_SUCCESS`)
//!   prevents compaction retry storms — after a failed compaction we don't
//!   re-attempt on every turn.
//!
//! Token estimate: `chars / 4` (GPT-class models; good enough for the 80%
//! trigger — exact counting isn't worth a tokenizer dep).

use agent_llm::types::{ChatMessage, Role};

/// Fraction of `context_window` that triggers compaction.
pub const COMPACT_AT: f32 = 0.80;
/// Fraction at which a *prefire* Pass-1 summary may start (hidden latency).
pub const PREFIRE_LEAD_PERCENT: f32 = 0.70;

/// Rough token estimate — chars/4 covers GPT-class models; CJK text leans
/// toward 1 token/char so this stays conservative on the trigger side.
pub fn estimate_tokens(msgs: &[ChatMessage]) -> usize {
    msgs.iter()
        .map(|m| {
            let c = m.content.as_deref().map(str::len).unwrap_or(0);
            let tc = m
                .tool_calls
                .as_ref()
                .map(|v| v.iter().map(|t| t.name.len() + t.arguments.len()).sum::<usize>())
                .unwrap_or(0);
            (c + tc) / 4 + 4 + m.images.len() * 1100 // +4 msg overhead; ~1100/image
        })
        .sum()
}

/// Should compaction fire at a caller-chosen fraction? The settings UI
/// exposes 70/80/90% of the window.
pub fn should_compact_at(tokens: usize, window: usize, frac: f32) -> bool {
    tokens >= (window as f32 * frac) as usize
}

/// Should compaction fire? `tokens` is the current estimate, `window` the
/// model's context size.
pub fn should_compact(tokens: usize, window: usize) -> bool {
    should_compact_at(tokens, window, COMPACT_AT)
}

/// Should the *prefire* Pass-1 start early (hide the latency)?
pub fn should_prefire(tokens: usize, window: usize) -> bool {
    tokens >= (window as f32 * PREFIRE_LEAD_PERCENT) as usize
}

/// Sanitize + budget-fit the history in place — runs before every sample.
///
/// Steps (kernel-architecture.md):
/// 1. Flatten tool calls into the message text — **only** when the provider
///    doesn't accept structured `tool_calls` (`native_tool_calls == false`).
///    For a native provider, flattening would orphan every `Role::Tool`
///    result (its `tool_call_id` no longer has a matching `function_call`)
///    so the model loses all tool output (P1-b).
/// 2. Reasoning traces are deliberately left alone: they are display-only
///    (no adapter puts them on the wire), and the persisted row is what a
///    reopened session replays as its `思考过程` block.
/// 3. Image refs: kept verbatim when the active model declares `"image"`
///    input; stripped when it's text-only (e.g. a mid-session downgrade) —
///    the marker text stays as provenance.
/// 4. `fit_conversation_to_budget` — drop oldest non-system messages until
///    the estimate fits `budget_tokens`.
pub fn sanitize_for_sample(
    history: &mut Vec<ChatMessage>,
    budget_tokens: usize,
    native_tool_calls: bool,
    keep_images: bool,
) {
    // (1) flatten tool calls → appended as `[call: name(args)]` text, but
    // ONLY for text-protocol providers. A native provider needs the
    // structured `tool_calls` array intact for its `function_call` items.
    if !native_tool_calls {
        for m in history.iter_mut() {
            if let Some(calls) = m.tool_calls.take() {
                let flat: String = calls
                    .iter()
                    .map(|c| format!("\n[call: {}({})]", c.name, abbreviate(&c.arguments, 200)))
                    .collect();
                let mut c = m.content.take().unwrap_or_default();
                c.push_str(&flat);
                m.content = Some(c);
            }
        }
    }
    // (2) reasoning stays on the message. It is a *display* field: the wire
    // bodies are built from role/content/tool_calls only, and `openai_compat`
    // (the one adapter that serializes the whole struct) strips it. Dropping
    // it here would silently empty every replayed thinking block on the next
    // snapshot — sanitize runs against the real history, not a copy.

    // (3) images ride the `images` field now — real parts for a vision
    // model, dropped for a text-only one (the `<attached-image>` marker
    // in `content` keeps the path reference either way).
    if !keep_images {
        for m in history.iter_mut() {
            m.images.clear();
        }
    }

    // (4) fit to budget — drop oldest non-system messages.
    fit_conversation_to_budget(history, budget_tokens);
}

/// Drop oldest non-system messages until `estimate_tokens(history)` fits
/// `budget`. The system prompt (history[0]) is never dropped.
///
/// Tool-pair integrity (Responses API): dropping a `Role::Tool` row would
/// orphan its `function_call` — strict backends (deepseek) reject the
/// request with `No tool output found for tool call …`. So removing a tool
/// row also strips the matching `tool_calls` entry on its assistant row;
/// an assistant row left with no calls AND no content is removed too (a
/// blank turn replays as a meaningless message).
fn fit_conversation_to_budget(history: &mut Vec<ChatMessage>, budget: usize) {
    while history.len() > 1 && estimate_tokens(history) > budget {
        // Find the first non-system index and remove it — prefer dropping
        // tool results (they're the bulkiest) then assistant turns.
        let drop_idx = history
            .iter()
            .enumerate()
            .skip(1)
            .position(|(_, m)| m.role == Role::Tool)
            .map(|p| p + 1)
            .or_else(|| {
                history.iter().enumerate().skip(1)
                    .position(|(_, m)| m.role != Role::System).map(|p| p + 1)
            });
        match drop_idx {
            Some(i) if i < history.len() => {
                let removed = history.remove(i);
                if removed.role == Role::Tool {
                    if let Some(tid) = &removed.tool_call_id {
                        // Orphan guard: the matching assistant row's call
                        // is pulled with its output so the pair stays
                        // atomic — an unpaired function_call is a 400.
                        for m in history.iter_mut() {
                            if m.role != Role::Assistant {
                                continue;
                            }
                            if let Some(calls) = &mut m.tool_calls {
                                let before = calls.len();
                                calls.retain(|c| c.id != *tid);
                                if calls.len() != before {
                                    // Assistant row now carries neither
                                    // calls nor text → drop the blank turn.
                                    if calls.is_empty()
                                        && m.content.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true)
                                    {
                                        m.tool_calls = None;
                                        m.content = None;
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                // Sweep any assistant row that ended up fully empty
                // (no calls, no content) — it replays as a blank message.
                history.retain(|m| {
                    m.role != Role::Assistant
                        || m.content.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
                        || m.tool_calls.as_ref().map(|c| !c.is_empty()).unwrap_or(false)
                });
            }
            _ => break, // only system left — can't shrink further
        }
    }
}

/// The compaction plan — prefix/suffix split for the two-pass summarize.
#[derive(Debug)]
pub struct CompactionPlan {
    /// history[0..cut) → summarized into `NOTE₁`.
    pub prefix_end: usize,
    /// Estimated tokens in the prefix (the part being compressed away).
    pub prefix_tokens: usize,
}

/// Decide where to split. The suffix keeps the last N messages verbatim so
/// the model doesn't lose the immediate task context — ~25% of the window.
pub fn plan(history: &[ChatMessage], window: usize) -> Option<CompactionPlan> {
    if history.len() < 4 {
        return None; // nothing worth compacting
    }
    // Keep the system prompt + the most recent ~25% of the window's worth
    // of messages as the suffix. Walk back from the end accumulating tokens.
    let suffix_budget = (window as f32 * 0.25) as usize;
    let mut acc = 0usize;
    let mut cut = history.len();
    for (i, m) in history.iter().enumerate().rev() {
        let t = m.content.as_deref().map(|s| s.len() / 4 + 4).unwrap_or(4);
        if acc + t > suffix_budget && i > 1 {
            cut = i + 1;
            break;
        }
        acc += t;
        cut = i;
    }
    if cut <= 1 {
        return None; // suffix already covers everything
    }
    // Don't split a tool block: if the suffix would begin with `Role::Tool`
    // rows, their assistant `tool_calls` row sits in the prefix — compacting
    // it away orphans the outputs, and strict backends (deepseek) reject the
    // replayed request. Fold the dangling outputs into the compacted prefix.
    while cut < history.len() && history[cut].role == Role::Tool {
        cut += 1;
    }
    if cut >= history.len() {
        return None; // the tail is one tool block — nothing to summarize past
    }
    let prefix_tokens = estimate_tokens(&history[..cut]);
    Some(CompactionPlan { prefix_end: cut, prefix_tokens })
}

/// Apply a completed compaction: replace history[1..plan.prefix_end) with a
/// single synthesized `NOTE` message — `history[0]` (the kernel system
/// prompt) is never compacted away. The `note_text` is Pass-2's output
/// (the condensed summary the provider produced).
pub fn apply(history: &mut Vec<ChatMessage>, plan: &CompactionPlan, note_text: String) {
    let note = ChatMessage {
        role: Role::System,
        content: Some(format!(
            "[context note — earlier turns compacted]\n{note_text}"
        )),
        tool_calls: None,
        tool_call_id: None,
        is_error: None,
        notice: None,
        ts: None,
        images: Vec::new(),
        reasoning: None,
    };
    // Splice from index 1 — `history[0]` is the rendered kernel system
    // prompt; compacting it away would strip the model's tool protocol and
    // mode contract for the rest of the session.
    history.splice(1..plan.prefix_end, std::iter::once(note));
}

/// Sticky suppression state — after a failed compaction we don't re-fire on
/// the very next turn (storm guard). `UntilSuccess` latches until a
/// compaction actually succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Suppress {
    #[default]
    None,
    /// Skip one more trigger check.
    Sticky,
    /// Skip until a compaction completes successfully.
    UntilSuccess,
}

#[derive(Debug, Default)]
pub struct CompactionSuppressor {
    state: Suppress,
}

impl CompactionSuppressor {
    /// Check + consume. `fire` when the trigger is allowed to run.
    pub fn check(&mut self) -> bool {
        match self.state {
            Suppress::None => true,
            Suppress::Sticky => {
                self.state = Suppress::None;
                false
            }
            Suppress::UntilSuccess => false,
        }
    }

    /// Compaction failed — latch suppression so the next turn doesn't retry.
    pub fn on_failure(&mut self) {
        self.state = Suppress::UntilSuccess;
    }

    /// Compaction succeeded — clear all suppression.
    pub fn on_success(&mut self) {
        self.state = Suppress::None;
    }
}

/// Truncate `s` to `n` chars at a char boundary (for flattened tool args).
fn abbreviate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        let mut end = n;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: Role, text: &str) -> ChatMessage {
        ChatMessage { role, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: None, images: Vec::new(), reasoning: None }
    }

    #[test]
    fn estimate_is_chars_over_four() {
        let h = vec![msg(Role::User, &"a".repeat(400))];
        // 400 chars / 4 + 4 overhead = 104
        assert_eq!(estimate_tokens(&h), 104);
    }

    #[test]
    fn trigger_at_80_percent() {
        assert!(should_compact(80_000, 100_000));
        assert!(!should_compact(79_000, 100_000));
    }

    #[test]
    fn prefire_at_70_percent() {
        assert!(should_prefire(70_000, 100_000));
        assert!(!should_prefire(69_000, 100_000));
    }

    #[test]
    fn sanitize_flattens_tool_calls() {
        let mut h = vec![
            msg(Role::System, "sys"),
            ChatMessage {
                role: Role::Assistant,
                content: Some("calling".into()),
                tool_calls: Some(vec![agent_llm::types::ToolCall {
                    id: "1".into(), name: "list_dir".into(),
                    arguments: "{\"path\":\".\"}".into(),
                }]),
                tool_call_id: None,
                is_error: None,
                        notice: None,
                ts: None,
            images: Vec::new(),
            
            reasoning: None,},
        ];
        // text-protocol provider → tool_calls flattened into message text.
        sanitize_for_sample(&mut h, 100_000, /*native_tool_calls*/ false, true);
        assert!(h[1].tool_calls.is_none());
        assert!(h[1].content.as_deref().unwrap().contains("[call: list_dir"));

        // native provider → tool_calls stay structured (NOT flattened),
        // else the matching Role::Tool result would orphan (P1-b).
        let mut h2 = vec![msg(Role::System, "sys"), ChatMessage {
            role: Role::Assistant,
            content: Some("ok".into()),
            tool_calls: Some(vec![agent_llm::types::ToolCall {
                id: "1".into(), name: "list_dir".into(),
                arguments: "{\"path\":\".\"}".into(),
            }]),
            tool_call_id: None,
            is_error: None,
                        notice: None,
            ts: None,
            images: Vec::new(),
        
        reasoning: None,}];
        sanitize_for_sample(&mut h2, 100_000, /*native_tool_calls*/ true, true);
        assert!(h2[1].tool_calls.is_some(), "native tool_calls must not flatten");
        assert!(!h2[1].content.as_deref().unwrap().contains("[call:"));
    }

    #[test]
    fn fit_drops_tool_results_first() {
        let mut h = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "q"),
            msg(Role::Tool, &"x".repeat(4_000)), // bulky tool result
            msg(Role::Assistant, "a"),
        ];
        // Budget so only ~system + last two survive.
        sanitize_for_sample(&mut h, 200, true, true);
        assert!(h.iter().all(|m| m.role != Role::Tool));
        assert!(h[0].role == Role::System); // system never dropped
    }

    #[test]
    fn fit_removes_orphaned_tool_call_with_its_output() {
        // A tool result is dropped for budget → the matching `tool_calls`
        // entry on the assistant row must go too, else the next request
        // replays an unpaired `function_call` and deepseek 400s with
        // "No tool output found for tool call …".
        let mut h = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "q"),
            ChatMessage {
                role: Role::Assistant,
                content: Some("calling".into()),
                tool_calls: Some(vec![
                    agent_llm::types::ToolCall {
                        id: "call_a".into(), name: "bash".into(), arguments: "{}".into(),
                    },
                    agent_llm::types::ToolCall {
                        id: "call_b".into(), name: "bash".into(), arguments: "{}".into(),
                    },
                ]),
                tool_call_id: None, is_error: None, notice: None, ts: None,
                images: Vec::new(),
            
            reasoning: None,},
            {
                let mut m = msg(Role::Tool, &"output_a ".repeat(500));
                m.tool_call_id = Some("call_a".into());
                m
            },
            {
                let mut m = msg(Role::Tool, "out_b");
                m.tool_call_id = Some("call_b".into());
                m
            },
            msg(Role::Assistant, "done"),
        ];
        // Budget fits everything except the bulky call_a output — one
        // drop removes it and the orphaned call_a entry, nothing else.
        let total = estimate_tokens(&h);
        let call_a_out = 500 * "output_a ".len() / 4 + 4;
        sanitize_for_sample(&mut h, total - call_a_out + 50, true, true);
        // The bulky call_a output is gone; its call must be gone too.
        let assistant = h.iter().find(|m| m.role == Role::Assistant && m.tool_calls.is_some()).unwrap();
        let ids: Vec<&str> = assistant.tool_calls.as_ref().unwrap().iter().map(|c| c.id.as_str()).collect();
        assert!(!ids.contains(&"call_a"), "orphaned call_a must be removed");
        assert!(ids.contains(&"call_b"), "call_b survives with its output");
        // call_b's output row is still present.
        assert!(h.iter().any(|m| m.role == Role::Tool && m.tool_call_id.as_deref() == Some("call_b")));
    }

    #[test]
    fn plan_splits_prefix_suffix() {
        let mut h = vec![msg(Role::System, "sys")];
        for _i in 0..20 {
            h.push(msg(Role::User, &"u".repeat(200).repeat(1).to_string()));
            h.push(msg(Role::Assistant, &"a".repeat(200)));
        }
        let p = plan(&h, 1_000).unwrap(); // 25% suffix = ~250 tokens
        assert!(p.prefix_end > 1 && p.prefix_end < h.len());
        // suffix stays under its ~250-token budget
        assert!(estimate_tokens(&h[p.prefix_end..]) <= 300);
    }

    #[test]
    fn apply_replaces_prefix_with_note() {
        // Enough history that `plan` actually splits — 30 pairs of 200-char
        // messages against a 400-token window (suffix keeps ~100 tokens).
        let mut h = vec![msg(Role::System, "sys")];
        for _ in 0..30 {
            h.push(msg(Role::User, &"u".repeat(200)));
            h.push(msg(Role::Assistant, &"a".repeat(200)));
        }
        let p = plan(&h, 400).unwrap();
        let orig_len = h.len();
        apply(&mut h, &p, "summary".into());
        // Splice starts at 1 — the system prompt survives compaction.
        assert_eq!(h.len(), orig_len - p.prefix_end + 2);
        assert_eq!(h[0].content.as_deref().unwrap(), "sys");
        assert!(h[1].content.as_deref().unwrap().contains("[context note"));
    }

    #[test]
    fn suppressor_blocks_retry_storm() {
        let mut s = CompactionSuppressor::default();
        s.on_failure();
        assert!(!s.check()); // UntilSuccess latches
        assert!(!s.check());
        s.on_success();
        assert!(s.check());
    }
}
