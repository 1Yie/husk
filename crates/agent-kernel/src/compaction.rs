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
//!   flatten tool calls, strip reasoning blocks, replace images with
//!   placeholders, `fit_conversation_to_budget`.
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
            (c + tc) / 4 + 4 // +4 per-message role/formatting overhead
        })
        .sum()
}

/// Should compaction fire? `tokens` is the current estimate, `window` the
/// model's context size.
pub fn should_compact(tokens: usize, window: usize) -> bool {
    tokens >= (window as f32 * COMPACT_AT) as usize
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
/// 2. Strip reasoning blocks — `reasoning` deltas are transient, never sent.
/// 3. Replace image-bearing messages with a text placeholder (vision lands
///    in Stage 11; until then an image is a context bomb).
/// 4. `fit_conversation_to_budget` — drop oldest non-system messages until
///    the estimate fits `budget_tokens`.
pub fn sanitize_for_sample(
    history: &mut Vec<ChatMessage>,
    budget_tokens: usize,
    native_tool_calls: bool,
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
    // (2) reasoning never persisted into ChatMessage — nothing to strip.
    // (Reasoning deltas are a separate stream; they never enter history.)

    // (3) image placeholder — `content` may carry a data URI from a future
    // vision path; replace anything that smells like one.
    for m in history.iter_mut() {
        if let Some(c) = &m.content {
            if c.contains("data:image/") {
                m.content = Some("[image attached — vision not in this build]".into());
            }
        }
    }

    // (4) fit to budget — drop oldest non-system messages.
    fit_conversation_to_budget(history, budget_tokens);
}

/// Drop oldest non-system messages until `estimate_tokens(history)` fits
/// `budget`. The system prompt (history[0]) is never dropped.
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
            Some(i) if i < history.len() => { history.remove(i); }
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
    let prefix_tokens = estimate_tokens(&history[..cut]);
    Some(CompactionPlan { prefix_end: cut, prefix_tokens })
}

/// Apply a completed compaction: replace history[0..plan.prefix_end) with a
/// single synthesized `NOTE` message. The `note_text` is Pass-2's output
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
    };
    history.splice(0..plan.prefix_end, std::iter::once(note));
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
        ChatMessage { role, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: None }
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
            },
        ];
        // text-protocol provider → tool_calls flattened into message text.
        sanitize_for_sample(&mut h, 100_000, /*native_tool_calls*/ false);
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
        }];
        sanitize_for_sample(&mut h2, 100_000, /*native_tool_calls*/ true);
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
        sanitize_for_sample(&mut h, 200, true);
        assert!(h.iter().all(|m| m.role != Role::Tool));
        assert!(h[0].role == Role::System); // system never dropped
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
        assert_eq!(h.len(), orig_len - p.prefix_end + 1);
        assert!(h[0].content.as_deref().unwrap().contains("[context note"));
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
