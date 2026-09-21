//! `ask_question` — structured user input mid-turn. The model offers a
//! question plus up to four labelled options; the UI renders a card
//! (option buttons + free-text field) in the same slot as approval
//! cards. The tool blocks until the user answers or the turn is
//! cancelled — same wait-discipline as `wait_for_decision`.
//!
//! `readonly: true` — asking a question mutates nothing and needs no
//! approval, so it auto-allows in every mode (plan included). In
//! headless contexts (delegated subagents) the channel carries no UI
//! sender and `exec` refuses fast instead of deadlocking.

use super::registry::{Args, ToolCtx, ToolError, ToolResult, ToolSpec};
use futures::FutureExt;

const MAX_QUESTION_BYTES: usize = 1024;
const MAX_OPTIONS: usize = 4;
const MAX_LABEL_BYTES: usize = 80;
const MAX_DESCRIPTION_BYTES: usize = 200;

/// `schema_for` reads the doc comments into the JSON Schema descriptions.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AskOptionArg {
    /// Short answer label rendered as a button (≤80 chars).
    label: String,
    /// Optional one-line explanation shown under the label.
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AskQuestionArgs {
    /// The question to ask — one decision or gap, stated plainly.
    question: String,
    /// Up to 4 offered answers rendered as buttons. Omit for a pure
    /// free-text question.
    #[serde(default)]
    options: Vec<AskOptionArg>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "ask_question",
        schema: super::registry::schema_for::<AskQuestionArgs>(
            "Ask the user a structured question when a decision or missing piece of information \
             is blocking progress and cannot be reasonably inferred. The UI shows the question \
             with your options as buttons plus a free-text field; the tool blocks until the user \
             answers or cancels the turn. Use sparingly — prefer reasonable defaults, and never \
             ask questions you could answer by inspecting the workspace.",
        ),
        readonly: true, // asking mutates nothing; auto-allowed like the goal contract
        exec: std::sync::Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: std::sync::Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: AskQuestionArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::Args(e.to_string()))?;

    let question = parsed.question.trim();
    if question.is_empty() || question.len() > MAX_QUESTION_BYTES {
        return Err(ToolError::Args(format!(
            "`question` must be 1..{MAX_QUESTION_BYTES} bytes"
        )));
    }
    if parsed.options.len() > MAX_OPTIONS {
        return Err(ToolError::Args(format!(
            "at most {MAX_OPTIONS} options — merge or drop the rest"
        )));
    }
    let mut options = Vec::with_capacity(parsed.options.len());
    for opt in &parsed.options {
        let label = opt.label.trim();
        if label.is_empty() || label.len() > MAX_LABEL_BYTES {
            return Err(ToolError::Args(format!(
                "option labels must be 1..{MAX_LABEL_BYTES} bytes"
            )));
        }
        let description = opt.description.as_ref().and_then(|d| {
            let t = d.trim();
            (!t.is_empty()).then(|| t.chars().take(MAX_DESCRIPTION_BYTES).collect::<String>())
        });
        options.push(agent_ipc::events::AskOption {
            label: label.to_string(),
            description,
        });
    }

    let answer = ctx
        .ask
        .ask(question.to_string(), options, ctx.cancel.clone())
        .await?;
    Ok(ToolResult::text(format!("用户回答：{answer}")))
}
