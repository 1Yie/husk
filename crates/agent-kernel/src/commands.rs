//! `commands.rs` — slash-command registry, phase 1 of the extension pipeline.
//!
//! Contract (plugin-system.md §Commands): input starting with `/` is
//! intercepted **before** the ReAct loop — zero tokens spent. Built-ins ship
//! first; workspace skills (`.agents/skills/<name>/SKILL.md`) resolve to a
//! `FeedToAgent` with the skill body inlined — the model follows the skill
//! text as instructions, matching Claude-Code's `/{skill}` convention.
//! MCP `prompts/*` and WASM `command_execute` register under `plugin_id:name`
//! when those runtimes land.
//!
//! Dispatch order: `CommandRegistry::try_run` returns `Some(result)` when a
//! `/x` matched a command — the session turns `ControlAction` into state
//! changes and `Reply`/`FeedToAgent` into messages without touching the LLM.
//!
//! `@path` and mid-text `$skill` mentions are expanded here too
//! (`expand_user_tokens`) — the composer leaves the literal `@src/main.rs`
//! / `$review` in the text; the kernel inlines the file's fenced content /
//! skill instructions before the prompt reaches history, so the model
//! sees real context instead of bare tokens.

use std::path::{Path, PathBuf};

use agent_ipc::UiEvent;
use agent_llm::types::ChatMessage;

const MAX_MENTION_BYTES: usize = 32 * 1024;

/// What a command produced — the session maps it to side effects.
#[derive(Debug)]
pub enum CommandResult {
    /// Local echo — append as a system message, no LLM call.
    Reply(String),
    /// Session control — reset/switch-model/undo…
    Control(ControlOp),
    /// Wrap the result into a normal prompt and continue the turn.
    FeedToAgent(String),
}

/// State-changing commands the session executes.
#[derive(Debug)]
pub enum ControlOp {
    /// `/clear` — wipe history back to the system prompt.
    ClearHistory,
    /// `/compact` — force a compaction pass.
    Compact,
    /// `/undo` — reverse the last turn's writes (HunkTracker).
    UndoLastTurn,
    /// `/model <provider>/<model>` — hot-swap for the next turn.
    SetModel { provider: String, model: String },
}

/// Context a command needs — session passes itself in pieces.
pub struct CommandCtx<'a> {
    pub history: &'a mut Vec<ChatMessage>,
    pub permission_mode: &'a str,
    pub workspace_root: &'a std::path::Path,
    /// Emit a UI event (status/system message) — the shared sink keeps the
    /// ctx `Send` so `handle` futures can `tokio::spawn`.
    pub ui_tx: &'a crate::channels::UiSink,
}

/// The registry — built-ins first; plugin commands register under
/// `plugin_id:name`.
pub struct CommandRegistry;

impl CommandRegistry {
    /// Intercept a `/x` or `$x` input. Returns `Some(result)` when it was a
    /// command, `None` when the text should fall through to the LLM
    /// (unknown `/x`/`$x` gets a hint but still feeds through as a prompt —
    /// the contract). `$` is a skill-only trigger: `$name` never matches
    /// built-ins, so `$clear` can't wipe history.
    pub async fn try_run(
        text: &str,
        ctx: &mut CommandCtx<'_>,
    ) -> Option<CommandResult> {
        let t = text.trim();
        let (name, args) = t.split_once(' ').map(|(n, a)| (n, a.trim()))
            .unwrap_or((t, ""));
        if let Some(cmd) = name.strip_prefix('/') {
            return Some(Self::dispatch(cmd, args, ctx));
        }
        // `$skill` — dollar trigger resolves ONLY against the skill dirs.
        if let Some(skill_name) = name.strip_prefix('$') {
            let skills = crate::skills::SkillManager::new(ctx.workspace_root);
            if let Ok(skill) = skills.load(skill_name, Some(args)) {
                return Some(CommandResult::FeedToAgent(crate::skills::prompt::for_user(
                    &skill,
                    &format!("${skill_name}"),
                )));
            }
            let line = format!("`${skill_name}` 不是可用技能 — 已作为普通消息发送");
            let _ = ctx.ui_tx.send(UiEvent::SystemMessage(line.clone()));
            ctx.history.push(ChatMessage::notice(line));
            return Some(CommandResult::FeedToAgent(
                expand_user_tokens(t, ctx.workspace_root),
            ));
        }
        // Plain prompt — still expand `@path`/`$skill` mentions so the
        // model sees file contents / skill instructions, not bare tokens.
        let expanded = expand_user_tokens(t, ctx.workspace_root);
        if expanded != t {
            return Some(CommandResult::FeedToAgent(expanded));
        }
        None
    }

    /// `/x` dispatch — built-ins first, then workspace-skill fallback.
    fn dispatch(name: &str, args: &str, ctx: &mut CommandCtx<'_>) -> CommandResult {
        match name {
            "clear" => CommandResult::Control(ControlOp::ClearHistory),
            "compact" => CommandResult::Control(ControlOp::Compact),
            "undo" => CommandResult::Control(ControlOp::UndoLastTurn),
            "skills" => {
                let skills = crate::skills::SkillManager::new(ctx.workspace_root);
                // One catalog, one rendering — the same listing the `skill`
                // tool's `list` action returns, and the same one the prompt
                // catalog is built from.
                if skills.list().is_empty() {
                    CommandResult::Reply(
                        "no skills found — add `.agents/skills/<name>/SKILL.md` to the workspace".into(),
                    )
                } else {
                    CommandResult::Reply(skills.catalog_listing())
                }
            }
            "model" => {
                // `/model provider/model` or bare `/model provider model`
                let parts: Vec<&str> = args.splitn(2, |c| c == '/' || c == ' ').collect();
                let (provider, model) = match parts.as_slice() {
                    [p, m] => ((*p).to_string(), (*m).to_string()),
                    [m] => ("".to_string(), (*m).to_string()),
                    _ => return CommandResult::Reply("usage: /model <provider>/<model>".into()),
                };
                CommandResult::Control(ControlOp::SetModel { provider, model })
            }
            "diff" => {
                // `/diff` — report the turn's changed files via hunk tracker.
                // The session owns the tracker; command returns a reply
                // asking the session to emit the file list (ControlOp could
                // carry it, but Reply keeps the boundary simple).
                CommandResult::Reply(
                    "[/diff] file-change report is emitted by the session".into(),
                )
            }
            _ => {
                // Skill fallback — `/{name}` resolves against the workspace +
                // user skill dirs; the skill body is inlined into the turn
                // so the model follows it as instructions.
                let skills = crate::skills::SkillManager::new(ctx.workspace_root);
                if let Ok(skill) = skills.load(name, Some(args)) {
                    CommandResult::FeedToAgent(crate::skills::prompt::for_user(
                        &skill,
                        &format!("/{name}"),
                    ))
                } else {
                    // Unknown /x → hint + fall through as a normal prompt.
                    let line = format!("`/{name}` 不是命令或技能 — 已作为普通消息发送");
                    let _ = ctx.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                    ctx.history.push(ChatMessage::notice(line));
                    CommandResult::FeedToAgent(expand_user_tokens(&t_fallback(name, args), ctx.workspace_root))
                }
            }
        }
    }
}

/// Reassemble the original `/x args` text for the unknown-command fallthrough.
fn t_fallback<'a>(name: &'a str, args: &'a str) -> String {
    if args.is_empty() { format!("/{name}") } else { format!("/{name} {args}") }
}

/// Inline `@path` mentions — each `@rel/path` token is replaced by a fenced
/// content block so the model reads the file, not just its name. Missing /
/// binary / oversized files degrade to a bracketed note rather than
/// silently dropping the mention.
fn expand_mentions(text: &str, root: &Path) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        // `@` must start a token — preceded by whitespace or string start —
        // or it's an email/mention literal we leave alone.
        let boundary = at == 0 || rest.as_bytes()[at - 1].is_ascii_whitespace();
        if !boundary {
            out.push_str(&rest[..at + 1]);
            rest = &rest[at + 1..];
            continue;
        }
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        // Path token = up to the next whitespace; reject empties and pure `@`.
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let token = &after[..end];
        if token.is_empty() || token.starts_with('@') {
            out.push('@');
            rest = after;
            continue;
        }
        // Resolve inside the workspace — refuse escapes (`@../x`, absolute).
        // Normalize `..`/`.` lexically first so a non-existent escape target
        // is still caught (canonicalize fails on missing files).
        let rel = Path::new(token);
        let mut norm = PathBuf::new();
        let mut escapes = rel.is_absolute();
        for comp in rel.components() {
            use std::path::Component::*;
            match comp {
                ParentDir => { if !norm.pop() { escapes = true; } }
                CurDir => {}
                Normal(c) => norm.push(c),
                RootDir | Prefix(_) => escapes = true,
            }
        }
        let joined = root.join(&norm);
        let canon = joined.canonicalize().unwrap_or_else(|_| joined.clone());
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if escapes || !canon.starts_with(&root_canon) {
            out.push_str(&format!("`@{token}` (outside workspace — not inlined)"));
            rest = &after[end..];
            continue;
        }
        match std::fs::read(&canon) {
            Ok(bytes) if !bytes.contains(&0) => {
                let capped = if bytes.len() > MAX_MENTION_BYTES {
                    &bytes[..MAX_MENTION_BYTES]
                } else {
                    &bytes[..]
                };
                let body = String::from_utf8_lossy(capped);
                let trunc = if bytes.len() > MAX_MENTION_BYTES {
                    format!("\n… ({} bytes truncated)", bytes.len() - MAX_MENTION_BYTES)
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "\n\n`<workspace-file path=\"{token}\">`\n```\n{body}{trunc}\n```\n"
                ));
            }
            Ok(_) => {
                out.push_str(&format!("`@{token}` (binary file — not inlined)"));
            }
            Err(_) => {
                out.push_str(&format!("`@{token}` (not found — left as literal)"));
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Inline `$skill` mentions mid-text — like [`expand_mentions`] but for
/// skills: a whitespace/start-bounded `$name` token that resolves via
/// [`find_skill`] is replaced by the skill's instructions (same wrapper
/// the dispatch path emits, minus args). Unresolvable tokens stay
/// literal — `$HOME`, `$5`, `$(cmd)` are shell syntax, not intent.
fn expand_skill_refs(text: &str, root: &Path) -> String {
    if !text.contains('$') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        // Same boundary rule as `@` — start-of-string or after whitespace.
        let boundary = at == 0 || rest.as_bytes()[at - 1].is_ascii_whitespace();
        if !boundary {
            out.push_str(&rest[..at + 1]);
            rest = &rest[at + 1..];
            continue;
        }
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let token = &after[..end];
        // `$$` drops the first char and re-scans (same convention as
        // `@@`); empties stay literal.
        if token.is_empty() || token.starts_with('$') {
            out.push('$');
            rest = after;
            continue;
        }
        // A skill name can't start with a digit/paren — `$5`, `$(x)`
        // skip the filesystem lookup entirely.
        let looks_like_name = token
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
        let resolved = looks_like_name
            .then(|| crate::skills::SkillManager::new(root).load(token, None).ok())
            .flatten();
        if let Some(skill) = resolved {
            out.push_str(&format!(
                "\n\n`<skill name=\"{token}\">`\n{}\n",
                crate::skills::prompt::for_user(&skill, &format!("${token}"))
            ));
            rest = &after[end..];
        } else {
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Expand every inline mention token in user text — `$skill` refs first
/// (only the user's own text is scanned, no injected content yet), then
/// `@path` mentions over the result (so `@` tokens inside an injected
/// skill body still resolve).
fn expand_user_tokens(text: &str, root: &Path) -> String {
    expand_mentions(&expand_skill_refs(text, root), root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a `.agents/skills/<name>/SKILL.md` into a tempdir workspace.
    fn skill_ws() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agents/skills/review")).unwrap();
        std::fs::write(
            root.join(".agents/skills/review/SKILL.md"),
            "---\nname: review\ndescription: review code carefully\n---\n\nLook for bugs.\n",
        )
        .unwrap();
        // A no-front-matter skill — name falls back to the directory.
        std::fs::create_dir_all(root.join(".agents/skills/deploy")).unwrap();
        std::fs::write(root.join(".agents/skills/deploy/SKILL.md"), "ship it\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        (dir, root)
    }

    /// The scanner is metadata-only now — the body arrives from the loader,
    /// frontmatter stripped. This asserts the split holds end to end.
    #[test]
    fn metadata_carries_the_summary_and_the_loader_carries_the_body() {
        let (_d, root) = skill_ws();
        let skills = crate::skills::scanner::scan_skills(&root);
        assert_eq!(skills.len(), 2);

        let manager = crate::skills::SkillManager::new(&root);
        let review = manager.load("review", None).unwrap();
        assert_eq!(review.description, "review code carefully");
        assert!(review.body.contains("Look for bugs."));
        assert!(!review.body.contains("name:"), "front-matter must not reach the body");

        // A no-front-matter skill still loads (name from the directory).
        let deploy = manager.load("deploy", None).unwrap();
        assert!(deploy.body.contains("ship it"));

        // …and the metadata path never read either body.
        let meta = skills.iter().find(|s| s.name == "deploy").unwrap();
        assert_eq!(meta.description, "");
        assert_eq!(meta.path.file_name().unwrap(), "SKILL.md");
    }

    #[tokio::test]
    async fn slash_skill_feeds_agent_with_body() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        let res = CommandRegistry::try_run("/review", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => {
                assert!(p.contains("`/review` skill"));
                assert!(p.contains("Look for bugs."));
            }
            _ => panic!("expected FeedToAgent, got {res:?}"),
        }
    }

    #[tokio::test]
    async fn slash_skill_passes_args() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        let res = CommandRegistry::try_run("/review src/", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => assert!(p.contains("Skill arguments: src/")),
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[tokio::test]
    async fn unknown_slash_falls_back_to_prompt() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        let res = CommandRegistry::try_run("/nonexistent", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => assert_eq!(p, "/nonexistent"),
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[tokio::test]
    async fn at_mention_inlines_file_content() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        let res = CommandRegistry::try_run("check @main.rs please", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => {
                assert!(p.contains("fn main() {}"), "inlined body missing: {p}");
                assert!(p.contains("path=\"main.rs\""));
            }
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[test]
    fn mid_text_skill_ref_inlines_body() {
        let (_d, root) = skill_ws();
        let out = expand_skill_refs("用 $review 检查这段代码", &root);
        assert!(out.contains("<skill name=\"review\">"), "{out}");
        assert!(out.contains("Look for bugs."), "{out}");
        assert!(out.contains("invoked the `$review` skill"), "{out}");
        // The token is consumed — no stray `$review` remains.
        assert!(!out.contains(" $review "), "{out}");
    }

    #[test]
    fn skill_refs_stay_literal_when_shell_or_unknown() {
        let (_d, root) = skill_ws();
        let t = "echo $HOME 和 $5 和 $(cmd) 和 $unknown";
        assert_eq!(expand_skill_refs(t, &root), t);
    }

    #[test]
    fn user_tokens_expand_dollar_then_at() {
        let (_d, root) = skill_ws();
        let out = expand_user_tokens("用 $review 检查 @main.rs", &root);
        assert!(out.contains("<skill name=\"review\">"), "{out}");
        assert!(out.contains("path=\"main.rs\""), "{out}");
    }

    #[tokio::test]
    async fn at_mention_escapes_are_blocked() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        let res = CommandRegistry::try_run("read @../outside.txt", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => {
                assert!(p.contains("outside workspace"), "escape not blocked: {p}");
            }
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[tokio::test]
    async fn email_like_at_is_left_alone() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        // `a@b` — the `@` isn't at a token boundary, so no expansion and the
        // text falls through unchanged (None → normal prompt path).
        assert!(CommandRegistry::try_run("mail a@b.com", &mut ctx).await.is_none());
    }

    #[test]
    fn scan_finds_claude_and_pi_skill_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".claude/skills/review")).unwrap();
        std::fs::write(
            root.join(".claude/skills/review/SKILL.md"),
            "---\nname: claude-review\n---\nbody\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(".pi/skills/harness")).unwrap();
        std::fs::write(root.join(".pi/skills/harness/SKILL.md"), "pi body\n").unwrap();

        let skills = crate::skills::scanner::scan_skills(&root);
        assert!(skills.iter().any(|s| s.name == "claude-review"));
        // No front-matter → falls back to the directory name.
        assert!(skills.iter().any(|s| s.name == "harness"));
        assert!(!skills.iter().any(|s| s.global));
    }

    #[tokio::test]
    async fn dollar_trigger_dispatches_skill_only() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = crate::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            ui_tx: &tx,
        };
        // `$review` resolves the workspace skill…
        let res = CommandRegistry::try_run("$review src/", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => {
                assert!(p.contains("`$review` skill"));
                assert!(p.contains("Look for bugs."));
                assert!(p.contains("Skill arguments: src/"));
            }
            _ => panic!("expected FeedToAgent, got {res:?}"),
        }
        // …but never a built-in — `$clear` is NOT `/clear`.
        let res = CommandRegistry::try_run("$clear", &mut ctx).await.unwrap();
        match res {
            CommandResult::FeedToAgent(p) => assert_eq!(p, "$clear"),
            _ => panic!("$clear must not dispatch the /clear built-in"),
        }
    }
}
