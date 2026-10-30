//! `commands.rs` — slash-command registry.
//!
//! Input starting with `/` is intercepted before the ReAct loop, so a command
//! costs no tokens. Skills resolve to a `FeedToAgent` with the body inlined;
//! `try_run` returns `Some(result)` on a match and the session turns it into
//! state changes or messages without touching the LLM.
//!
//! `expand_user_tokens` also handles `@path` and mid-text `$skill` mentions: the
//! composer leaves the literal token in the text and the kernel inlines the
//! file's content or the skill's instructions before the prompt reaches history.

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
    /// A plugin's `tool:<name>` slash command — the session resolves the
    /// args and dispatches through the plugin router (async, so it can't
    /// live in `dispatch` itself). `tool` is the wire name (`id__tool`),
    /// `name` the command the user typed (for error echo — the wire name
    /// is meaningless to them).
    PluginTool {
        tool: String,
        name: String,
        args: String,
    },
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
    /// Persistent memory handle — `/remember`, `/forget`, `/memory` operate
    /// on the persona k/v table. `None` when the session has no store.
    pub memory: Option<&'a agent_context::memory::MemoryStore>,
    /// Enabled plugins' declared slash commands — a snapshot the session
    /// refreshes per dispatch (plugins can reload mid-session).
    pub plugin_commands: &'a [agent_plugin::PluginCommand],
    /// Emit a UI event (status/system message) — the shared sink keeps the
    /// ctx `Send` so `handle` futures can `tokio::spawn`.
    pub ui_tx: &'a agent_kernel::channels::UiSink,
    /// The session's `.blueprint/` switch — `/blueprint` answers "off" when
    /// it is false.
    pub blueprint_enabled: bool,
    /// The session's linked workspaces. `@path` reaches them too (by absolute
    /// path only — a relative token always names the workspace).
    pub linked_roots: &'a [agent_capabilities::LinkedRoot],
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
    pub async fn try_run(text: &str, ctx: &mut CommandCtx<'_>) -> Option<CommandResult> {
        let t = text.trim();
        let (name, args) = t
            .split_once(' ')
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((t, ""));
        if let Some(cmd) = name.strip_prefix('/') {
            return Some(Self::dispatch(cmd, args, ctx));
        }
        // `$skill` — dollar trigger resolves ONLY against the skill dirs.
        if let Some(skill_name) = name.strip_prefix('$') {
            let skills = agent_skills::SkillManager::new(ctx.workspace_root);
            if let Ok(skill) = skills.load(skill_name, Some(args)) {
                return Some(CommandResult::FeedToAgent(agent_skills::prompt::for_user(
                    &skill,
                    &format!("${skill_name}"),
                )));
            }
            let line = format!("`${skill_name}` 不是可用技能 — 已作为普通消息发送");
            let _ = ctx.ui_tx.send(UiEvent::SystemMessage(line.clone()));
            ctx.history.push(ChatMessage::notice(line));
            return Some(CommandResult::FeedToAgent(expand_user_tokens(t, ctx.workspace_root, ctx.linked_roots)));
        }
        // Plain prompt — still expand `@path`/`$skill` mentions so the
        // model sees file contents / skill instructions, not bare tokens.
        let expanded = expand_user_tokens(t, ctx.workspace_root, ctx.linked_roots);
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
            // Memory surface — the user-facing half of the persona table the
            // `remember` tool and the turn distiller also write.
            "remember" => {
                let Some(m) = ctx.memory else {
                    return CommandResult::Reply("memory store unavailable".into());
                };
                let text = args.trim();
                if text.is_empty() {
                    return CommandResult::Reply(
                        "usage: /remember <durable note or preference>".into(),
                    );
                }
                // Same gate as the `remember` tool: a pasted credential would be
                // injected into every later prompt.
                if agent_context::memory::distill::looks_secret(text) {
                    return CommandResult::Reply(
                        "refused: this looks like a credential — memory keeps preferences \
                         and project facts, never secrets."
                            .into(),
                    );
                }
                // Keys are user/agent-visible labels — `/remember` takes the
                // lowest free `note<N>` so `/forget <key>` can address it.
                let keys: std::collections::BTreeSet<String> = m
                    .all_persona()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| p.key)
                    .collect();
                let mut n = 1;
                while keys.contains(&format!("note{n}")) {
                    n += 1;
                }
                let key = format!("note{n}");
                match m.set_persona(&key, text) {
                    Ok(_) => CommandResult::Reply(format!(
                        "remembered `{key}` — `/forget {key}` removes it"
                    )),
                    Err(e) => CommandResult::Reply(format!("memory write failed: {e}")),
                }
            }
            "forget" => {
                let Some(m) = ctx.memory else {
                    return CommandResult::Reply("memory store unavailable".into());
                };
                let key = args.trim();
                if key.is_empty() {
                    return CommandResult::Reply(
                        "usage: /forget <key> — `/memory` lists keys".into(),
                    );
                }
                match m.remove_persona(key) {
                    Ok(true) => CommandResult::Reply(format!("forgot `{key}`")),
                    Ok(false) => CommandResult::Reply(format!("no memory named `{key}`")),
                    Err(e) => CommandResult::Reply(format!("memory write failed: {e}")),
                }
            }
            "memory" => {
                let Some(m) = ctx.memory else {
                    return CommandResult::Reply("memory store unavailable".into());
                };
                let persona = m.all_persona().unwrap_or_default();
                let mut out = String::from("── persona ──\n");
                if persona.is_empty() {
                    out.push_str(
                        "(empty — `/remember <note>` or the agent's `remember` tool adds one)\n",
                    );
                } else {
                    for p in &persona {
                        out.push_str(&format!("`{}`: {}\n", p.key, p.value));
                    }
                }
                CommandResult::Reply(out)
            }
            "skills" => {
                let skills = agent_skills::SkillManager::new(ctx.workspace_root);
                // One catalog, one rendering — the same listing the `skill`
                // tool's `list` action returns, and the same one the prompt
                // catalog is built from.
                if skills.list().is_empty() {
                    CommandResult::Reply(
                        "no skills found — add `.agents/skills/<name>/SKILL.md` to the workspace"
                            .into(),
                    )
                } else {
                    CommandResult::Reply(skills.catalog_listing())
                }
            }
            // Architecture blueprint — index/check/graph answer locally;
            // `init` hands the model a bootstrap prompt.
            "blueprint" => match agent_coding::blueprint::command(
                ctx.workspace_root,
                ctx.blueprint_enabled,
                args,
            ) {
                Ok(reply) => CommandResult::Reply(reply),
                Err(prompt) => CommandResult::FeedToAgent(prompt),
            },
            "model" => {
                // `/model provider/model` or bare `/model provider model`
                let parts: Vec<&str> = args.splitn(2, ['/', ' ']).collect();
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
                CommandResult::Reply("[/diff] file-change report is emitted by the session".into())
            }
            _ => {
                // Plugin-declared commands — a manifest's `capabilities.commands`
                // resolves before the skill fallback: an explicit declaration
                // wins over a heuristic skill load. Both spellings match —
                // `/id:name` (collision-safe) and bare `/name` (first enabled
                // plugin's wins when several declare the same name).
                if let Some(pc) = ctx
                    .plugin_commands
                    .iter()
                    .find(|c| c.name == name || c.qualified() == name)
                {
                    if let Some(tool) = pc.action.strip_prefix("tool:") {
                        return CommandResult::PluginTool {
                            tool: agent_plugin::wire_tool_name(&pc.plugin_id, tool),
                            name: name.to_string(),
                            args: args.to_string(),
                        };
                    }
                    if let Some(prompt) = pc.action.strip_prefix("prompt:") {
                        let body = if args.is_empty() {
                            prompt.to_string()
                        } else {
                            format!("{prompt}\n\n{args}")
                        };
                        return CommandResult::FeedToAgent(body);
                    }
                    return CommandResult::Reply(format!(
                        "`/{name}` declared an unrecognized action `{}` — expected `tool:` or `prompt:`",
                        pc.action
                    ));
                }
                // Skill fallback — `/{name}` resolves against the workspace +
                // user skill dirs; the skill body is inlined into the turn
                // so the model follows it as instructions.
                let skills = agent_skills::SkillManager::new(ctx.workspace_root);
                if let Ok(skill) = skills.load(name, Some(args)) {
                    CommandResult::FeedToAgent(agent_skills::prompt::for_user(
                        &skill,
                        &format!("/{name}"),
                    ))
                } else {
                    // Unknown /x → hint + fall through as a normal prompt.
                    let line = format!("`/{name}` 不是命令或技能 — 已作为普通消息发送");
                    let _ = ctx.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                    ctx.history.push(ChatMessage::notice(line));
                    CommandResult::FeedToAgent(expand_user_tokens(
                        &t_fallback(name, args),
                        ctx.workspace_root,
                        ctx.linked_roots,
                    ))
                }
            }
        }
    }
}

/// Reassemble the original `/x args` text for the unknown-command fallthrough.
fn t_fallback<'a>(name: &'a str, args: &'a str) -> String {
    if args.is_empty() {
        format!("/{name}")
    } else {
        format!("/{name} {args}")
    }
}

/// Inline `@path` mentions — each `@rel/path` token is replaced by a fenced
/// content block so the model reads the file, not just its name. Missing /
/// binary / oversized files degrade to a bracketed note rather than
/// silently dropping the mention.
fn expand_mentions(text: &str, root: &Path, linked: &[agent_capabilities::LinkedRoot]) -> String {
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
        // Resolve inside the workspace, or inside a linked workspace for an
        // absolute token — that is the only way to name a file the workspace
        // root cannot reach. A relative token always means the workspace.
        // `..`/`.` are normalized lexically first so a non-existent escape
        // target is still caught (canonicalize fails on missing files).
        let rel = Path::new(token);
        let mut norm = PathBuf::new();
        let mut escapes = rel.is_absolute();
        for comp in rel.components() {
            use std::path::Component::*;
            match comp {
                ParentDir => {
                    if !norm.pop() {
                        escapes = true;
                    }
                }
                CurDir => {}
                Normal(c) => norm.push(c),
                RootDir | Prefix(_) => escapes = true,
            }
        }
        let canon = if rel.is_absolute() {
            rel.canonicalize().unwrap_or_else(|_| rel.to_path_buf())
        } else {
            let joined = root.join(&norm);
            joined.canonicalize().unwrap_or_else(|_| joined.clone())
        };
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let in_workspace = !escapes && canon.starts_with(&root_canon);
        let in_linked = linked.iter().any(|r| canon.starts_with(&r.path));
        if !in_workspace && !in_linked {
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
            .then(|| agent_skills::SkillManager::new(root).load(token, None).ok())
            .flatten();
        if let Some(skill) = resolved {
            out.push_str(&format!(
                "\n\n`<skill name=\"{token}\">`\n{}\n",
                agent_skills::prompt::for_user(&skill, &format!("${token}"))
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
fn expand_user_tokens(
    text: &str,
    root: &Path,
    linked: &[agent_capabilities::LinkedRoot],
) -> String {
    expand_mentions(&expand_skill_refs(text, root), root, linked)
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
        let skills = agent_skills::scanner::scan_skills(&root);
        assert_eq!(skills.len(), 2);

        let manager = agent_skills::SkillManager::new(&root);
        let review = manager.load("review", None).unwrap();
        assert_eq!(review.description, "review code carefully");
        assert!(review.body.contains("Look for bugs."));
        assert!(
            !review.body.contains("name:"),
            "front-matter must not reach the body"
        );

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
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
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
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        let res = CommandRegistry::try_run("/review src/", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::FeedToAgent(p) => assert!(p.contains("Skill arguments: src/")),
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[tokio::test]
    async fn unknown_slash_falls_back_to_prompt() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        let res = CommandRegistry::try_run("/nonexistent", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::FeedToAgent(p) => assert_eq!(p, "/nonexistent"),
            _ => panic!("expected FeedToAgent"),
        }
    }

    #[tokio::test]
    async fn at_mention_inlines_file_content() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        let res = CommandRegistry::try_run("check @main.rs please", &mut ctx)
            .await
            .unwrap();
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
        let out = expand_user_tokens("用 $review 检查 @main.rs", &root, &[]);
        assert!(out.contains("<skill name=\"review\">"), "{out}");
        assert!(out.contains("path=\"main.rs\""), "{out}");
    }

    /// An absolute token may name a file in a linked workspace; a relative one
    /// never does, and an absolute token outside every root stays literal.
    #[test]
    fn at_mentions_reach_linked_workspaces_by_absolute_path() {
        let (_d, root) = skill_ws();
        let elsewhere = tempfile::tempdir().unwrap();
        let linked = elsewhere.path().canonicalize().unwrap();
        std::fs::write(linked.join("kernel.rs"), "fn linked() {}\n").unwrap();
        let roots = [agent_capabilities::LinkedRoot {
            path: linked.clone(),
            access: agent_capabilities::RootAccess::Read,
        }];
        let abs = linked.join("kernel.rs");
        let out = expand_user_tokens(&format!("look at @{}", abs.display()), &root, &roots);
        assert!(out.contains("fn linked()"), "{out}");

        let out = expand_user_tokens(&format!("look at @{}", abs.display()), &root, &[]);
        assert!(out.contains("outside workspace — not inlined"), "{out}");

        let out = expand_user_tokens("look at @kernel.rs", &root, &roots);
        assert!(out.contains("not found — left as literal"), "{out}");
    }

    #[tokio::test]
    async fn at_mention_escapes_are_blocked() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        let res = CommandRegistry::try_run("read @../outside.txt", &mut ctx)
            .await
            .unwrap();
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
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        // `a@b` — the `@` isn't at a token boundary, so no expansion and the
        // text falls through unchanged (None → normal prompt path).
        assert!(CommandRegistry::try_run("mail a@b.com", &mut ctx)
            .await
            .is_none());
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

        let skills = agent_skills::scanner::scan_skills(&root);
        assert!(skills.iter().any(|s| s.name == "claude-review"));
        // No front-matter → falls back to the directory name.
        assert!(skills.iter().any(|s| s.name == "harness"));
        assert!(!skills.iter().any(|s| s.global));
    }

    #[tokio::test]
    async fn dollar_trigger_dispatches_skill_only() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        // `$review` resolves the workspace skill…
        let res = CommandRegistry::try_run("$review src/", &mut ctx)
            .await
            .unwrap();
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

    /// `/remember` writes a persona note under an auto `note<N>` key,
    /// `/memory` renders it, `/forget` removes it — the user-facing surface
    /// of the same table the `remember` tool and distiller write.
    #[tokio::test]
    async fn remember_refuses_a_credential_note() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            agent_context::memory::MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap();
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: Some(&store),
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };

        let res = CommandRegistry::try_run(
            "/remember sk-proj-abcdefghijklmnopqrstuvwxyz0123456789",
            &mut ctx,
        )
        .await
        .unwrap();
        match res {
            CommandResult::Reply(r) => assert!(r.contains("credential"), "{r}"),
            _ => panic!("expected Reply"),
        }
        assert!(
            store.all_persona().unwrap().is_empty(),
            "credential was stored"
        );
    }

    #[tokio::test]
    async fn remember_memory_forget_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            agent_context::memory::MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap();
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: Some(&store),
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };

        let res = CommandRegistry::try_run("/remember 用中文回答", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::Reply(r) => assert!(r.contains("note1"), "{r}"),
            _ => panic!("expected Reply"),
        }

        // A second note takes the next free index.
        CommandRegistry::try_run("/remember 少写注释", &mut ctx)
            .await
            .unwrap();
        let keys: Vec<String> = store
            .all_persona()
            .unwrap()
            .into_iter()
            .map(|p| p.key)
            .collect();
        assert_eq!(keys, vec!["note1".to_string(), "note2".to_string()]);

        let res = CommandRegistry::try_run("/memory", &mut ctx).await.unwrap();
        match res {
            CommandResult::Reply(r) => {
                assert!(r.contains("`note1`: 用中文回答"), "{r}");
                assert!(r.contains("`note2`: 少写注释"), "{r}");
            }
            _ => panic!("expected Reply"),
        }

        let res = CommandRegistry::try_run("/forget note1", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::Reply(r) => assert!(r.contains("forgot"), "{r}"),
            _ => panic!("expected Reply"),
        }
        assert_eq!(store.persona("note1").unwrap(), None);

        // Deleting note1 frees its index — the next /remember reuses note1.
        CommandRegistry::try_run("/remember 再来一条", &mut ctx)
            .await
            .unwrap();
        assert_eq!(store.persona("note1").unwrap().as_deref(), Some("再来一条"));
    }

    /// Empty args and missing keys degrade to usage hints, never panics.
    #[tokio::test]
    async fn remember_forget_usage_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            agent_context::memory::MemoryStore::open(&dir.path().join("m.db"), dir.path()).unwrap();
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: Some(&store),
            plugin_commands: &[],
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        let res = CommandRegistry::try_run("/remember", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::Reply(r) => assert!(r.contains("usage"), "{r}"),
            _ => panic!("expected Reply"),
        }
        let res = CommandRegistry::try_run("/forget nosuch", &mut ctx)
            .await
            .unwrap();
        match res {
            CommandResult::Reply(r) => assert!(r.contains("no memory"), "{r}"),
            _ => panic!("expected Reply"),
        }
    }

    fn plugin_cmds() -> Vec<agent_plugin::PluginCommand> {
        vec![
            agent_plugin::PluginCommand {
                plugin_id: "wasm-textutils".into(),
                name: "slugify".into(),
                description: "slug".into(),
                action: "tool:slugify".into(),
            },
            agent_plugin::PluginCommand {
                plugin_id: "docs".into(),
                name: "guide".into(),
                description: "guide".into(),
                action: "prompt:Read the docs.".into(),
            },
        ]
    }

    #[tokio::test]
    async fn slash_plugin_tool_command_returns_plugin_tool() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let cmds = plugin_cmds();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &cmds,
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        match CommandRegistry::try_run("/slugify hello world", &mut ctx)
            .await
            .unwrap()
        {
            CommandResult::PluginTool { tool, args, .. } => {
                // The wire name is `id__tool` — `dispatch_tool_call`
                // resolves it, a model call spelled the same way does too.
                assert_eq!(tool, "wasm-textutils__slugify");
                assert_eq!(args, "hello world");
            }
            _ => panic!("expected PluginTool"),
        }
    }

    #[tokio::test]
    async fn slash_plugin_qualified_and_prompt_forms() {
        let (_d, root) = skill_ws();
        let (tx, _rx) = agent_kernel::channels::UiSink::channel();
        let mut hist = Vec::new();
        let cmds = plugin_cmds();
        let mut ctx = CommandCtx {
            history: &mut hist,
            permission_mode: "default",
            workspace_root: &root,
            linked_roots: &[],
            memory: None,
            plugin_commands: &cmds,
            ui_tx: &tx,
            blueprint_enabled: true,
        };
        // `/id:name` — the collision-safe spelling.
        match CommandRegistry::try_run("/wasm-textutils:slugify hi", &mut ctx)
            .await
            .unwrap()
        {
            CommandResult::PluginTool { tool, .. } => {
                assert_eq!(tool, "wasm-textutils__slugify")
            }
            _ => panic!("expected PluginTool"),
        }
        // `prompt:` action — body + typed args feed the agent.
        match CommandRegistry::try_run("/guide section-2", &mut ctx)
            .await
            .unwrap()
        {
            CommandResult::FeedToAgent(p) => {
                assert!(p.contains("Read the docs."));
                assert!(p.contains("section-2"));
            }
            _ => panic!("expected FeedToAgent"),
        }
    }
}
