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
            "plan" => Self::Plan,
            "goal" => Self::Goal,
            "office" => Self::Office,
            _ => Self::Build,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::Goal => "goal",
            Self::Office => "office",
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
            Self::Office => {
                "Mode: office — you are an office-document assistant. Produce REAL \
                 Word/Excel/PowerPoint files (.docx/.xlsx/.pptx) — this is a \
                 document-production task, not a software-engineering task. Before \
                 touching a document, load the `officecli` skill (`skill` → \
                 `officecli`) — it teaches the STRATEGY (L1 read → L2 DOM edit → \
                 L3 raw XML) and the design rules; then load the format's \
                 ruleset via `office_skill` (word/pptx/excel). The skill shows \
                 `officecli <verb>` commands, but you NEVER run them through \
                 `bash` — every officecli verb maps to a typed `office_*` tool \
                 you call directly: create→office_create, open/close/save→\
                 office_open/office_close/office_save, get→office_get, \
                 query→office_query, set→office_set (props key→value map, plus \
                 find/replace), add→office_add (props map, after/before/index/\
                 from), remove→office_remove, move→office_move, batch→\
                 office_batch (pass the command objects as the `commands` array \
                 argument — never assemble JSON inside a `bash` heredoc), \
                 merge→office_merge, import→office_import, view→office_view, \
                 validate→office_validate, help→office_help (property names), \
                 load_skill→office_skill, raw/raw-set→office_raw; for anything \
                 left, `office_exec` takes a verbatim officecli argv. \
                 Efficiency: one slide/card row/table block = ONE `office_batch` \
                 call carrying all its add/set commands — never drip-feed \
                 shapes one call at a time. \
                 Output layout — every artifact gets its own folder: pick a \
                 short task slug (e.g. `q4-review`) and keep the whole \
                 deliverable inside `<slug>/` — the document at \
                 `<slug>/<name>.pptx` (or .docx/.xlsx), every supporting file \
                 under `<slug>/assets/` (downloaded images, generated \
                 diagrams/charts, CSV/JSON data, screenshots). Never write a \
                 deliverable or loose asset to the workspace root. \
                 Lifecycle: create → edit → deliver — every `office_*` write \
                 goes straight to disk (no resident process in this sandbox), \
                 so `office_open`/`office_save`/`office_close` are unnecessary. \
                 `office_batch` is atomic: if any item fails, the whole batch \
                 rolls back — nothing is applied. Verification \
                 budget: ONE `office_validate` pass + ONE `office_view` issues \
                 pass — fix only what blocks the file opening or renders it \
                 unusable (schema errors, missing content), then STOP and \
                 deliver. The format ruleset's Gate 3 screenshot audit runs \
                 ONLY when the user asks for visual polish — iterating on \
                 every [O1]/[C1] nit without being asked burns most of the \
                 turn on a file the user could already be reviewing. \
                 Assets: to enrich a document with pictures, `image_search` \
                 searches the internet across engines — `wikimedia` for \
                 license-clean photos, `bing` for broad web/brand/CN \
                 coverage (engine is optional; `auto` tries all) — then \
                 `web_download` saves the chosen URL into `<slug>/assets/` and \
                 `office_add` --type picture embeds it; `view_image` lets you \
                 SEE any workspace image (assets the user placed here, \
                 downloaded files) before using it. \
                 A request for a \"PPT\"/\"演示文稿\"/\"幻灯片\" means a real .pptx \
                 file built via the `office_*` tools — `dashi-ppt` generates an \
                 HTML web deck (not a .pptx), so load it ONLY when the user \
                 explicitly asks for an HTML/web presentation. `bash`/`apply_patch` \
                 remain \
                 available for supporting work (preparing CSV/JSON data, image \
                 assets, format conversion via `soffice`), but the deliverable is \
                 the document file itself. When finished, report the produced file \
                 path(s) — never claim a document exists without having written it \
                 to disk. The workspace root is shared by EVERY office session — \
                 other sessions' deliverables live in their own `<slug>/` \
                 folders; read them freely when the user references earlier work."
            }
        }
    }
}
