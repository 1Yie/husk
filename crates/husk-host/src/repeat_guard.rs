//! Stop a turn that is stuck repeating itself.
//!
//! Interactive sessions run without a tool-round cap on purpose: a real
//! refactor takes hundreds of round-trips, and the kernel's cap ends those
//! mid-edit. What a cap was actually catching is not "many calls" but "the
//! same call, over and over, with the same answer" — a model that re-runs a
//! failing command without changing anything, or re-reads a file it already
//! has. That is a loop, and it is cheap to recognise exactly: the same tool,
//! the same arguments, the same output.
//!
//! The streak is deliberately *consecutive only*. Any different call clears
//! it, because a different call may well have changed the answer — an edit
//! between two `cargo test` runs is progress, not a loop. So this hook never
//! guesses; it only speaks when nothing at all has moved.
//!
//! It is a host hook rather than a policy rule because only the hook seam sees
//! a call's arguments *and* its result; `TurnPolicy::before_dispatch` gets the
//! tool name alone.

use std::sync::Mutex;

use agent_kernel::hooks::AgentHook;
use agent_llm::types::ToolCall;
use async_trait::async_trait;
use xxhash_rust::xxh3::xxh3_64;

/// How many identical outcomes are tolerated before the next identical call
/// is refused. Four executions of one call is enough to establish that
/// nothing is moving; a fifth adds nothing but tokens.
const REPEAT_LIMIT: u32 = 3;

#[derive(Default)]
struct Inner {
    /// The call being repeated — a name plus the exact arguments the model
    /// sent, which is what makes "the same call" mean the same thing here as
    /// it does on the wire.
    key: Option<String>,
    /// Digest of the output that key produced.
    output: u64,
    /// How many consecutive identical `(key, output)` results came back.
    repeats: u32,
}

/// One per session, shared with the engine's hook chain through an `Arc`.
#[derive(Default)]
pub struct RepeatGuard(Mutex<Inner>);

impl RepeatGuard {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The streak the guard is watching — for tests and diagnostics.
    pub fn repeats(&self) -> u32 {
        self.lock().repeats
    }
}

fn call_key(call: &ToolCall) -> String {
    format!("{}\u{0}{}", call.name, call.arguments)
}

/// Enough of the arguments for the model to recognise the call it is stuck
/// on, without pasting a whole file body back into the context.
fn args_preview(call: &ToolCall) -> String {
    const CAP: usize = 200;
    let mut out: String = call.arguments.chars().take(CAP).collect();
    if call.arguments.chars().count() > CAP {
        out.push('…');
    }
    out
}

#[async_trait]
impl AgentHook for RepeatGuard {
    fn id(&self) -> &str {
        "husk:repeat-guard"
    }

    /// A new user message is a new intent: whatever was being retried before
    /// it may be exactly what the user wants tried again.
    async fn on_user_input(&self, _input: &str) -> anyhow::Result<agent_kernel::hooks::HookAction> {
        *self.lock() = Inner::default();
        Ok(agent_kernel::hooks::HookAction::Continue)
    }

    async fn before_tool_execute(&self, call: &mut ToolCall) -> anyhow::Result<()> {
        let key = call_key(call);
        let inner = self.lock();
        if inner.repeats >= REPEAT_LIMIT && inner.key.as_deref() == Some(key.as_str()) {
            return Err(anyhow::anyhow!(
                "已拒绝：`{} {}` 已经用同样的参数执行了 {} 次，每次返回完全相同的结果，再执行一次也不会有变化。\
                 换一个办法（改参数、先弄清失败原因，或用别的工具拿到同样的信息）；\
                 如果这条命令/读取就是卡住的原因，停在这里把阻塞告诉用户。",
                call.name,
                args_preview(call),
                inner.repeats + 1
            ));
        }
        Ok(())
    }

    async fn after_tool_execute(
        &self,
        call: &ToolCall,
        output: &mut String,
    ) -> anyhow::Result<()> {
        let key = call_key(call);
        let digest = xxh3_64(output.as_bytes());
        let mut inner = self.lock();
        if inner.key.as_deref() == Some(key.as_str()) && inner.output == digest {
            inner.repeats = inner.repeats.saturating_add(1);
        } else {
            // A different call, or a different answer to the same one: the
            // workspace moved, so the streak starts over.
            inner.key = Some(key);
            inner.output = digest;
            inner.repeats = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: args.into(),
        }
    }

    /// The same call with the same answer is refused once the streak is long
    /// enough — and the refusal happens *before* the call runs again.
    #[tokio::test]
    async fn a_loop_is_refused_after_repeated_identical_results() {
        let guard = RepeatGuard::default();
        let mut c = call("bash", r#"{"command":"cargo test"}"#);
        for _ in 0..=REPEAT_LIMIT {
            assert!(guard.before_tool_execute(&mut c).await.is_ok());
            let mut out = "error[E0308]: mismatched types".to_string();
            guard.after_tool_execute(&c, &mut out).await.unwrap();
        }
        let err = guard.before_tool_execute(&mut c).await.unwrap_err().to_string();
        assert!(err.contains("cargo test"), "{err}");
        assert!(err.contains("没有任何变化") || err.contains("完全相同"), "{err}");
    }

    /// A different answer means the workspace moved — the streak is gone, so
    /// edit-then-retest is never mistaken for a loop.
    #[tokio::test]
    async fn a_changed_result_clears_the_streak() {
        let guard = RepeatGuard::default();
        let mut c = call("bash", r#"{"command":"cargo test"}"#);
        for _ in 0..=REPEAT_LIMIT {
            guard.before_tool_execute(&mut c).await.unwrap();
            let mut out = "fail".to_string();
            guard.after_tool_execute(&c, &mut out).await.unwrap();
        }
        let mut out = "ok".to_string();
        guard.after_tool_execute(&c, &mut out).await.unwrap();
        assert_eq!(guard.repeats(), 0);
        assert!(guard.before_tool_execute(&mut c).await.is_ok());
    }

    /// A different call in between — the edit a retry would be waiting for —
    /// also clears it.
    #[tokio::test]
    async fn another_call_in_between_clears_the_streak() {
        let guard = RepeatGuard::default();
        let mut test = call("bash", r#"{"command":"cargo test"}"#);
        for _ in 0..=REPEAT_LIMIT {
            guard.before_tool_execute(&mut test).await.unwrap();
            let mut out = "fail".to_string();
            guard.after_tool_execute(&test, &mut out).await.unwrap();
        }
        let mut patch = call("fuzzy_patch", r#"{"path":"src/lib.rs"}"#);
        guard.before_tool_execute(&mut patch).await.unwrap();
        let mut written = "patched".to_string();
        guard.after_tool_execute(&patch, &mut written).await.unwrap();
        assert!(guard.before_tool_execute(&mut test).await.is_ok());
    }
}
