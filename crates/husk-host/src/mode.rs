//! Agent mode — what the session is allowed to aim at, orthogonal to the
//! permission gate (which governs *how* tool calls get approved).
//!
//! | Mode    | Tools                                    | Turn ends when            |
//! |---------|------------------------------------------|---------------------------|
//! | `build` | full registry                            | model stops calling tools |
//! | `plan`  | readonly tools only                      | same — writes impossible  |
//! | `goal`  | full registry + `goal_*` contract tools  | `goal_complete` declared  |
//! | `office`| full registry + `office_*` document tools | model stops calling tools |

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Build,
    Plan,
    Goal,
    Office,
}

impl AgentMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            agent_coding::MODE_PLAN => Self::Plan,
            agent_coding::MODE_GOAL => Self::Goal,
            "office" => Self::Office,
            _ => Self::Build,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => agent_coding::MODE_PLAN,
            Self::Goal => agent_coding::MODE_GOAL,
            Self::Office => "office",
        }
    }

    /// Complete contract rendered into each foreground request's live state —
    /// the model needs its current mode's instructions, not just a tool schema
    /// or the mode it had when the session was born.
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
            Self::Office => {
                "Mode: office — you are an office-document assistant. Produce REAL \
                 Word/Excel/PowerPoint files (.docx/.xlsx/.pptx) — this is a \
                 document-production task, not a software-engineering task. Before \
                 touching a document, load the `officecli` skill (`skill` → \
                 `officecli`) — it teaches the STRATEGY (L1 read → L2 DOM edit → \
                 L3 raw XML) and the design rules — then the format's ruleset via \
                 `office_skill` (word/pptx/excel). Every officecli verb is a typed \
                 `office_*` tool you call directly, never through `bash`: \
                 create→office_create, get→office_get, query→office_query, \
                 set→office_set (props map, plus find/replace), add→office_add, \
                 remove→office_remove, move→office_move, batch→office_batch \
                 (`commands` array — never assemble JSON in a bash heredoc), \
                 merge→office_merge, import→office_import, view→office_view, \
                 validate→office_validate, help→office_help (property names), \
                 load_skill→office_skill, raw→office_raw; anything else goes \
                 through `office_exec` with the verbatim argv. \
                 One slide/card row/table block = ONE `office_batch` carrying all \
                 its add/set commands — never drip-feed shapes one call at a time. \
                 Layout — the document at `<slug>/<name>.pptx` (or .docx/.xlsx) \
                 and every supporting file under `<slug>/assets/`, with a short \
                 task slug of your own (e.g. `q4-review`). Never write a \
                 deliverable or loose asset to the workspace root; other \
                 sessions' `<slug>/` folders are readable when the user \
                 references earlier work. \
                 Writes go straight to disk (no resident process in this \
                 sandbox), so create → edit → deliver needs no open/save/close; \
                 a batch is atomic — one failing item applies nothing. Verify \
                 ONCE: one `office_validate` pass plus one `office_view` issues \
                 pass, fix only what blocks the file opening or makes it \
                 unusable, then STOP and deliver. The format ruleset's Gate 3 \
                 screenshot audit runs ONLY when the user asks for visual \
                 polish — chasing every [O1]/[C1] nit burns the turn on a file \
                 the user could already be reviewing. \
                 Assets: `image_search` finds images (`wikimedia` for \
                 license-clean photos, `bing` for broad web/brand coverage), \
                 `web_download` saves the URL into `<slug>/assets/`, \
                 `office_add --type picture` embeds it, and `view_image` lets \
                 you SEE an image before using it. \
                 A request for a \"PPT\"/\"演示文稿\"/\"幻灯片\" means a real .pptx \
                 file built via the `office_*` tools — `dashi-ppt` generates an \
                 HTML web deck (not a .pptx), so load it ONLY when the user \
                 explicitly asks for an HTML/web presentation. `bash`/`apply_patch` \
                 stay available for supporting work (CSV/JSON prep, image assets, \
                 format conversion via `soffice`), but the deliverable is the \
                 document file itself. When finished, report the produced file \
                 path(s) — never claim a document exists without having written \
                 it to disk."
            }
        }
    }
}
