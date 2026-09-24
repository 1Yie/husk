//! Agent mode — what the session is allowed to aim at, orthogonal to the
//! permission gate (which governs *how* tool calls get approved).
//!
//! | Mode    | Tools                                    | Turn ends when            |
//! |---------|------------------------------------------|---------------------------|
//! | `build` | full registry                            | model stops calling tools |
//! | `plan`  | readonly tools only                      | same — writes impossible  |
//! | `goal`  | full registry + `goal_*` contract tools  | `goal_complete` declared  |

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Build,
    Plan,
    Goal,
}

impl AgentMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "plan" => Self::Plan,
            "goal" => Self::Goal,
            _ => Self::Build,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::Goal => "goal",
        }
    }

    /// One-paragraph instruction substituted into the system prompt —
    /// the model needs to know its mode's contract, not just its tool set.
    pub fn prompt_block(&self) -> &'static str {
        match self {
            Self::Build => "Mode: build — full tool set. Read, edit, run, verify.",
            Self::Plan => {
                "Mode: plan — READ-ONLY. You may inspect files, search, and reason, \
                 but you cannot edit files or run mutating commands. Do not attempt \
                 writes — the registry does not carry them. Your deliverable is a \
                 STRUCTURED plan, not prose: when the analysis is complete, call \
                 `submit_plan` ONCE with `summary`, ordered `steps` (each with \
                 `title`, `detail`, and the `files` it touches), `verification`, \
                 and `risks`. The user reviews that card and decides whether to \
                 execute it in build mode — a plain text answer is only for \
                 questions, never for a planning deliverable."
            }
            Self::Goal => {
                "Mode: goal — pursue the user's stated goal autonomously. The turn \
                 STARTS by confirming the goal: restate what you understood plus \
                 the success criteria you'll verify, and confirm with the user via \
                 `ask_question` before doing any work — only proceed autonomously \
                 after confirmation. The turn \
                 does NOT end when you stop calling tools: it ends only when you \
                 call `goal_complete` (with a summary of what was achieved) or \
                 `goal_blocked` (with the concrete blocker). If you produce a \
                 final-looking reply without declaring either, the engine pushes \
                 you back to work. Verify before declaring complete — a declared \
                 goal ends the run."
            }
        }
    }
}
