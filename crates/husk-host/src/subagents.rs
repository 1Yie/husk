//! Husk's named subagents, instruction files and saved child transcripts.

use std::path::{Path, PathBuf};

use agent_capabilities::{WorkspaceCtx, WorkspacePaths};
use agent_kernel::tools::delegate::{ChildSpec, SubagentHost};
use agent_kernel::tools::registry::ToolCtx;
use agent_llm::types::ChatMessage;

pub struct HuskSubagents;

impl SubagentHost for HuskSubagents {
    fn resolve(&self, ctx: &ToolCtx, name: &str) -> Option<ChildSpec> {
        let s = find_subagent(ctx.root().ok()?, name, &ctx.app_paths().ok()?)?;
        Some(ChildSpec {
            name: s.name,
            prompt: Some(s.prompt).filter(|p| !p.trim().is_empty()),
            tools: s.tools,
            readonly: s.readonly,
            model: s.model,
            thinking: s.thinking,
            inherit_instructions: s.context_files,
        })
    }

    fn instructions(&self, ctx: &ToolCtx, spec: &ChildSpec) -> String {
        let mut out = String::new();
        let Ok(root) = ctx.root() else {
            return out;
        };
        if spec.inherits_instructions() {
            if let Some(paths) = ctx.paths() {
                append_repo_instructions(&mut out, &paths.config_file("AGENTS.md"), "user");
            }
            append_repo_instructions(&mut out, &root.join("AGENTS.md"), "project");
        }
        out
    }

    fn persist(&self, ctx: &ToolCtx, child_id: u64, label: &str, history: &[ChatMessage]) {
        let Ok(state_dir) = ctx.workspace_state_dir() else {
            return;
        };
        let subs = state_dir.join("subagents");
        persist_transcript(&subs, child_id, label, history);
    }
}

/// One user-defined subagent — a `*.md` manifest under an agents dir.
/// The file's frontmatter carries `name`/`description`; the body IS the
/// child's system prompt (appended to the kernel's delegation contract).
#[derive(Debug, Clone)]
pub struct SubagentInfo {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub path: String,
    pub global: bool,
    /// `tools:` allowlist from the manifest. `None` = no restriction beyond the
    /// registry the spawn mode already picks.
    pub tools: Option<Vec<String>>,
    /// Manifest asks for the read-only child registry, same as `readonly: true`
    /// on the call. A call's own `readonly: true` still wins either way.
    pub readonly: bool,
    /// Manifest `model:` override, validated against the parent's provider at call time.
    pub model: Option<String>,
    /// Manifest `thinking:` override, validated against [`THINKING_LEVELS`].
    pub thinking: Option<String>,
    /// Manifest `context_files:` as written; resolved by [`ChildSpec`].
    pub context_files: Option<bool>,
}

/// Built-in subagents — always available to `delegate { agent: "<name>" }`
/// with no manifest file needed. A `*.md` manifest of the same name in an
/// agent dir SHADOWS the builtin (checked first in [`find_subagent`]).
/// (name, one-line description, system prompt)
pub const BUILTIN_SUBAGENTS: &[(&str, &str, &str)] = &[
    (
        "review",
        "代码审查：按严重度分级报告真实缺陷，只读不改",
        r#"你是一名严格的代码审查员（code reviewer）。目标是找出真实存在的问题——bug、数据风险、安全漏洞、逻辑缺陷——而不是罗列风格偏好。

工作准则：
- 先弄清改动的意图与上下文（读相关调用方/被调方），再逐文件核对正确性
- 检查重点：边界条件、空值/错误路径、并发与生命周期、注入与越权、资源泄漏、兼容性破坏
- 按严重度分级：
  · 阻断 —— 会出错的逻辑、数据丢失、安全漏洞，必须修
  · 重要 —— 错误处理缺失、明显坏味道导致的隐患，应该修
  · 建议 —— 可读性、一致性、小改进，可选
- 每条问题给出：文件:行号、问题是什么、为什么是问题、建议怎么改
- 不修改任何文件，只输出审查报告；拿不准的标"需人工确认"，不要当问题上报
- 没有实质问题就直说，不要硬凑条目

结束时输出：
## 审查结论
阻断 N / 重要 N / 建议 N，然后按级别列出问题清单。"#,
    ),
    (
        "test",
        "测试工程师：针对边界与错误路径设计并执行测试",
        r#"你是一名测试工程师（test engineer）。目标是用测试暴露缺陷，而不是证明代码能跑。

工作准则：
- 先读实现，标出边界值、空值、错误路径、并发/时序、状态转换——测试瞄准这些点
- 覆盖优先级：异常输入 > 错误处理 > 边界值 > 回归场景 > happy path
- 遵循项目现有测试框架、目录约定与 fixture/helper 复用方式；测试名说清意图
- 每个用例一个断言主题；不依赖执行顺序、不碰真实外部服务（用 mock/fake）
- 能跑就跑：失败信息是成果的一部分；跑不了就说明环境与原因
- 顺手记录覆盖缺口：哪些路径仍没有测试保护

结束时输出：
## 测试报告
新增 N 个用例 / 通过 / 失败明细 / 仍未覆盖的风险点。"#,
    ),
    (
        "ui-design",
        "UI/UX 设计：交互优先的界面方案与可实现代码",
        r#"你是一名 UI/UX 设计师。目标是把界面做"对"：先想清楚交互与信息层级，再落视觉与实现。

工作准则：
- 先定用户任务与信息优先级，再定布局结构——不为视觉效果牺牲可用性
- 遵循项目现有设计体系：组件、间距阶、字号阶、语义色令牌；缺令牌时用中性色阶补齐
- 可达性红线：对比度、键盘可达、可见焦点态、触摸目标 ≥ 40px
- 每个组件都要交代状态：空态、加载、错误、悬停、禁用、选中
- 输出落到实现层：结构说明 + 关键决策理由 + 可直接使用的代码/CSS（不写伪代码）
- 文案用界面语言（动词开头、具体、无官腔）

结束时输出：
## 设计方案
信息结构 → 关键决策 → 实现代码/标注 → 响应式与边界行为。"#,
    ),
];

/// Agent manifest dirs — workspace first (a project agent shadows a global one
/// of the same name), then user-level. `<workspace_dir>/agents/` is the host's
/// own workspace state dir (next to its `attachments/`); the others are read-only aliases
/// for trees other tools already write (`.agents/` = agentskills convention,
/// `.claude/` = Claude Code).
fn workspace_agent_dirs(root: &Path, paths: &agent_paths::AppPaths) -> Vec<PathBuf> {
    vec![
        paths.workspace_dir(root).join("agents"),
        root.join(".agents/agents"),
        root.join(".claude/agents"),
    ]
}

/// Where a project-scoped agent manifest belongs.
pub fn workspace_agent_dir(root: &Path, paths: &agent_paths::AppPaths) -> PathBuf {
    paths.workspace_dir(root).join("agents")
}

/// Where a user-scoped agent manifest belongs.
pub fn global_agent_dir(paths: &agent_paths::AppPaths) -> PathBuf {
    paths.config_file("agents")
}

/// User-level agent dirs, highest priority first.
fn global_agent_dirs(paths: &agent_paths::AppPaths) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![global_agent_dir(paths)];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".agents/agents"));
        dirs.push(home.join(".claude/agents"));
    }
    dirs
}

/// Scan workspace + user-level agent manifests. Frontmatter is optional —
/// a bare `name.md` file with no `---` block still registers (name from
/// filename, no description).
pub fn discover_subagents(root: &Path, paths: &agent_paths::AppPaths) -> Vec<SubagentInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let ws: Vec<PathBuf> = workspace_agent_dirs(root, paths);
    let gl: Vec<PathBuf> = global_agent_dirs(paths);
    for (dirs, global) in [(ws, false), (gl, true)] {
        for base in dirs {
            let Ok(rd) = std::fs::read_dir(&base) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() || p.extension().is_none_or(|x| x != "md") {
                    continue;
                }
                let content = std::fs::read_to_string(&p).unwrap_or_default();
                let (fm_name, fm_desc, body) = split_agent(&content);
                let fm = agent_frontmatter(&content);
                let name = fm_name
                    .or_else(|| p.file_stem().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                if name.is_empty() || !seen.insert(name.clone()) {
                    continue;
                }
                out.push(SubagentInfo {
                    name,
                    description: fm_desc.unwrap_or_default(),
                    prompt: body,
                    path: p.to_string_lossy().into_owned(),
                    global,
                    tools: fm.tools,
                    readonly: fm.readonly,
                    model: fm.model,
                    thinking: fm.thinking,
                    context_files: fm.context_files,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One settings row per agent: a discovered manifest where it shadows a
/// built-in name, the built-in itself everywhere else. The same rule
/// [`find_subagent`] spawns by, so the list can never show two rows for one
/// name — nor a row that `delegate` would resolve to someone else.
pub fn agent_rows(root: &Path, paths: &agent_paths::AppPaths) -> Vec<SubagentInfo> {
    let discovered = discover_subagents(root, paths);
    let mut taken = vec![false; discovered.len()];
    let mut rows: Vec<SubagentInfo> = Vec::new();
    for (name, description, prompt) in BUILTIN_SUBAGENTS {
        match discovered
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(name))
        {
            Some(i) => {
                taken[i] = true;
                rows.push(discovered[i].clone());
            }
            None => rows.push(SubagentInfo {
                name: (*name).to_string(),
                description: (*description).to_string(),
                prompt: (*prompt).to_string(),
                path: String::new(),
                global: true,
                tools: None,
                readonly: BUILTIN_READONLY.contains(name),
                model: None,
                thinking: None,
                context_files: None,
            }),
        }
    }
    for (i, sub) in discovered.into_iter().enumerate() {
        if !taken[i] {
            rows.push(sub);
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The prompt's agent catalog: discovered manifests first (they shadow same-named
/// builtins), then the builtins nothing shadows. Without it the model can only
/// pick names it happens to remember from the tool description, which makes a
/// user-authored manifest effectively unreachable.
pub fn catalog(root: &Path, paths: &agent_paths::AppPaths) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for sub in discover_subagents(root, paths) {
        seen.insert(sub.name.to_lowercase());
        let description = sub.description.trim();
        lines.push(format!(
            "- `{}` — {} ({})",
            sub.name,
            if description.is_empty() {
                "(no description)"
            } else {
                description
            },
            if sub.global { "user" } else { "project" }
        ));
    }
    for (name, description, _) in BUILTIN_SUBAGENTS {
        if seen.contains(&name.to_lowercase()) {
            continue;
        }
        lines.push(format!("- `{name}` — {description} (built-in)"));
    }
    if lines.is_empty() {
        "(none installed)".to_string()
    } else {
        lines.join("\n")
    }
}

/// Resolve one agent name → manifest. File manifests win (a same-named
/// `*.md` overrides the builtin); builtins are the fallback so
/// `delegate { agent: "review" }` works out of the box.
fn find_subagent(root: &Path, name: &str, paths: &agent_paths::AppPaths) -> Option<SubagentInfo> {
    let want = name.to_lowercase();
    discover_subagents(root, paths)
        .into_iter()
        .find(|s| s.name.to_lowercase() == want)
        .or_else(|| {
            BUILTIN_SUBAGENTS
                .iter()
                .find(|(n, _, _)| n.eq_ignore_ascii_case(&want))
                .map(|(n, d, p)| SubagentInfo {
                    name: (*n).to_string(),
                    description: (*d).to_string(),
                    prompt: (*p).to_string(),
                    path: String::new(),
                    global: true,
                    tools: None,
                    readonly: BUILTIN_READONLY.contains(n),
                    model: None,
                    thinking: None,
                    context_files: None,
                })
        })
}

/// One built-in spec by name — how the settings editor opens a profile that
/// has no manifest file. Saving from there writes an override manifest, which
/// then shadows this entry like any other same-named file.
pub fn builtin_subagent(name: &str) -> Option<(String, String, String)> {
    BUILTIN_SUBAGENTS
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, d, p)| ((*n).to_string(), (*d).to_string(), (*p).to_string()))
}

/// Builtins that must never write. Their prompts say 不修改任何文件, but a
/// prompt is not a guard, so the registry they run on is read-only.
const BUILTIN_READONLY: &[&str] = &["review"];

/// Write one child's full history to `<workspace_dir>/subagents/` as JSONL, one message
/// per line. Best effort: a write failure is logged and never fails the delegate
/// call, because the report the parent needs does not depend on this file.
fn persist_transcript(dir: &Path, child_id: u64, label: &str, history: &[ChatMessage]) {
    // Zero-padded epoch milliseconds: file names sort in the order children ran,
    // without an extra date dependency for one timestamp.
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stamp = format!("{millis:015}");
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = dir.join(format!("{stamp}-{child_id}-{safe}.jsonl"));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut out = String::new();
        for msg in history {
            out.push_str(&serde_json::to_string(msg).map_err(std::io::Error::other)?);
            out.push('\n');
        }
        std::fs::write(&path, out)
    };
    if let Err(e) = write() {
        tracing::warn!(path = %path.display(), error = %e, "subagent transcript not saved");
    }
}

/// Append one instruction file as a labelled section, matching the parent
/// session's format. A missing or empty file adds nothing.
fn append_repo_instructions(out: &mut String, path: &Path, label: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push_str(&format!(
        "\n\n## User instructions — {label} ({})\n\n{text}",
        path.display()
    ));
}

/// The fields a manifest's frontmatter can constrain. Parsed separately from
/// `split_agent` so that function's tuple shape stays put for its callers.
#[derive(Debug, Default, PartialEq)]
pub struct AgentFrontmatter {
    pub tools: Option<Vec<String>>,
    pub readonly: bool,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub context_files: Option<bool>,
}

/// Read the constraining frontmatter keys. Unknown keys are ignored (other
/// tools' manifests share the convention); `tools` is a comma list or a
/// `[a, b]` flow list.
pub fn agent_frontmatter(content: &str) -> AgentFrontmatter {
    let mut out = AgentFrontmatter::default();
    let trimmed = content.trim_start();
    let Some(fm) = trimmed.strip_prefix("---") else {
        return out;
    };
    let Some(end) = fm.find("\n---") else {
        return out;
    };
    for line in fm[..end].lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("tools:") {
            let v = v.trim().trim_start_matches('[').trim_end_matches(']');
            let list: Vec<String> = v
                .split(',')
                .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|t| !t.is_empty())
                .collect();
            out.tools = Some(list);
        } else if let Some(v) = line.strip_prefix("readonly:") {
            out.readonly = v.trim().eq_ignore_ascii_case("true");
        } else if let Some(v) = line.strip_prefix("model:") {
            out.model = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = line.strip_prefix("context_files:") {
            out.context_files = Some(v.trim().eq_ignore_ascii_case("true"));
        } else if let Some(v) = line.strip_prefix("thinking:") {
            out.thinking = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    out
}

/// Render a manifest for the settings editor. Frontmatter keys the editor
/// does not own (`tools:`, `readonly:`, `thinking:`, …) are carried over from
/// `existing`, so saving from the dialog never drops a hand-written limit.
/// `model` empty or `None` = no `model:` line (the child follows the
/// subagent default, then the parent).
pub fn render_manifest(
    existing: Option<&str>,
    name: &str,
    description: &str,
    model: Option<&str>,
    prompt: &str,
) -> String {
    let mut out = format!("---\nname: {name}\ndescription: {description}\n");
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        out.push_str(&format!("model: {m}\n"));
    }
    let kept = existing
        .and_then(|c| c.trim_start().strip_prefix("---"))
        .and_then(|fm| fm.find("\n---").map(|end| &fm[..end]))
        .unwrap_or("");
    for line in kept.lines() {
        let key = line.trim_start();
        if key.is_empty()
            || ["name:", "description:", "model:"]
                .iter()
                .any(|k| key.starts_with(k))
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!("---\n\n{prompt}\n"));
    out
}

/// `name`/`description` frontmatter split — same `---` convention skills use.
/// Split a `*.md` subagent manifest into `(name, description, prompt)`.
/// Public so the settings page can hand the body back to its edit dialog.
pub fn split_agent(content: &str) -> (Option<String>, Option<String>, String) {
    let trimmed = content.trim_start();
    let Some(fm) = trimmed.strip_prefix("---") else {
        return (None, None, content.to_string());
    };
    let Some(end) = fm.find("\n---") else {
        return (None, None, content.to_string());
    };
    let mut name = None;
    let mut desc = None;
    for line in fm[..end].lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    (name, desc, fm[end + 4..].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_reads_tools_and_readonly() {
        let flow =
            agent_frontmatter("---\nname: x\ntools: [read, grep]\nreadonly: true\n---\nbody");
        assert_eq!(
            flow.tools,
            Some(vec!["read".to_string(), "grep".to_string()])
        );
        assert!(flow.readonly);

        let bare = agent_frontmatter("---\nname: x\n---\nbody");
        assert_eq!(bare, AgentFrontmatter::default());

        let no_fm = agent_frontmatter("just a body");
        assert_eq!(no_fm, AgentFrontmatter::default());
    }

    #[test]
    fn render_manifest_sets_model_and_keeps_foreign_keys() {
        let old =
            "---\nname: x\ndescription: d\nmodel: big\ntools: [read]\nreadonly: true\n---\nold";
        let out = render_manifest(Some(old), "x", "new", Some("cheap/small"), "body");
        let fm = agent_frontmatter(&out);
        assert_eq!(fm.model.as_deref(), Some("cheap/small"));
        assert_eq!(fm.tools, Some(vec!["read".to_string()]));
        assert!(fm.readonly);
        assert_eq!(
            split_agent(&out),
            (Some("x".into()), Some("new".into()), "body".into())
        );

        let cleared = render_manifest(Some(old), "x", "d", None, "body");
        assert_eq!(agent_frontmatter(&cleared).model, None);
    }

    /// An override manifest replaces its built-in row instead of sitting
    /// beside it — one name, one row, and it is the one `delegate` resolves.
    #[test]
    fn override_manifest_replaces_the_builtin_row() {
        let dir = tempfile::tempdir().unwrap();
        let paths = agent_paths::AppPaths::new(dir.path().join("husk"), dir.path(), ".husk");
        let before = agent_rows(dir.path(), &paths);
        assert_eq!(
            before.iter().filter(|r| r.name == "review").count(),
            1,
            "{before:?}"
        );

        let ws = workspace_agent_dir(dir.path(), &paths);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("review.md"),
            "---\nname: review\ndescription: ours\nmodel: cheap/small\n---\n\nHUNT\n",
        )
        .unwrap();

        let after = agent_rows(dir.path(), &paths);
        let hits: Vec<&SubagentInfo> = after.iter().filter(|r| r.name == "review").collect();
        assert_eq!(hits.len(), 1, "{after:?}");
        assert!(!hits[0].path.is_empty());
        assert_eq!(hits[0].model.as_deref(), Some("cheap/small"));
        // The built-ins nothing shadows are still listed.
        assert!(after.iter().any(|r| r.name == "test" && r.path.is_empty()));
    }

    #[test]
    fn builtin_review_is_read_only() {
        let paths = agent_paths::AppPaths::for_tests("builtin-review");
        let review = find_subagent(std::path::Path::new("/nonexistent-root"), "review", &paths)
            .expect("review builtin");
        assert!(review.readonly);
        assert!(review.tools.is_none());
    }

    /// The catalog lists builtins and project manifests that shadow them.
    #[test]
    fn catalog_lists_builtins_and_shadowing_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let paths = agent_paths::AppPaths::new(dir.path().join("husk"), dir.path(), ".husk");
        let bare = catalog(dir.path(), &paths);
        assert!(bare.contains("`review`"), "{bare}");
        assert!(bare.contains("(built-in)"), "{bare}");

        let ws = workspace_agent_dir(dir.path(), &paths);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("review.md"),
            "---\nname: review\ndescription: our own reviewer\n---\n\nHUNT\n",
        )
        .unwrap();
        let with_manifest = catalog(dir.path(), &paths);
        assert!(
            with_manifest.contains("our own reviewer"),
            "{with_manifest}"
        );
        assert!(with_manifest.contains("(project)"), "{with_manifest}");
        assert_eq!(
            with_manifest.matches("`review`").count(),
            1,
            "{with_manifest}"
        );
    }

    /// Manifests use host-owned dirs; project ones win without writing to HOME.
    #[test]
    fn project_agents_shadow_and_dirs_are_host_owned() {
        let dir = tempfile::tempdir().unwrap();
        let paths = agent_paths::AppPaths::new(dir.path().join("husk"), dir.path(), ".husk");
        let ws = workspace_agent_dir(dir.path(), &paths);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("shadow.md"),
            "---\nname: shadow\ndescription: project\n---\n\nPROJECT\n",
        )
        .unwrap();
        let found = discover_subagents(dir.path(), &paths);
        let hits: Vec<&SubagentInfo> = found.iter().filter(|s| s.name == "shadow").collect();
        assert_eq!(hits.len(), 1, "{found:?}");
        assert!(!hits[0].global);
        assert!(ws.ends_with(".husk/agents"), "{ws:?}");
        let global = global_agent_dir(&paths);
        assert!(global.ends_with("husk/agents"), "{global:?}");
    }
}
