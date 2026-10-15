//! Slash-command layer for `App`.
//!
//! Three tiers:
//!  - LOCAL: parsed here (model/permission/mode/thinking/resume/help/status)
//!    because they either need a picker UI or update TUI-side labels;
//!  - FAST-PATH: sent as UiCommand so `dispatch` routes them correctly;
//!  - KERNEL: anything else starting with `/` goes through as `Prompt`
//!    — `CommandRegistry::try_run` owns /clear /compact /remember /forget
//!    /memory /skills /diff, plugin commands, user commands, and skill
//!    names (`/review …` → FeedToAgent). Unknown `/x` falls through to
//!    the LLM — same behavior as the desktop.

use husk_host::command::UiCommand;

use crate::app::App;
use crate::cells::Cell;
use crate::modals::Modal;

/// Every `/x` the TUI knows how to complete — local commands (handled
/// here), fast-path commands (sent as UiCommand), and kernel commands the
/// registry owns. `(name, description)`.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    // local
    ("/help", "命令速查"),
    ("/status", "会话状态卡（同 /settings）"),
    ("/settings", "同 /status"),
    ("/model", "切换模型 — 参数 p/m 或弹两级选择器"),
    (
        "/permission",
        "权限模式 default/acceptEdits/auto/dontAsk/bypass",
    ),
    ("/mode", "Agent 模式 build/plan/goal"),
    (
        "/thinking",
        "思考强度 off/minimal/low/medium/high/xhigh/max",
    ),
    ("/resume", "选择历史会话恢复"),
    ("/new", "开一个全新会话"),
    ("/quit", "退出（同 /exit /q）"),
    // fast-path UiCommand
    ("/undo", "回滚上一轮"),
    ("/retry", "重试上一轮"),
    ("/cancel", "打断当前轮"),
    // kernel CommandRegistry
    ("/clear", "清空历史"),
    ("/compact", "压缩上下文"),
    ("/remember", "记一条偏好 /remember <note>"),
    ("/forget", "删除记忆 /forget <key>"),
    ("/memory", "列出所有记忆"),
    ("/skills", "列出技能"),
    ("/diff", "查看本轮 diff"),
];

impl App {
    /// `/` 前缀补全候选 — composer 第一行以 `/` 开头且未 Esc 关闭时返回
    /// 匹配项（按出现顺序，Tab/↑↓ 选，Enter 填充）。
    pub fn slash_candidates(&self) -> Vec<(&'static str, &'static str)> {
        if self.slash_dismissed || !self.composer.text().starts_with('/') {
            return Vec::new();
        }
        let t = self.composer.text();
        // only complete while the input is a single /word token
        if t.contains([' ', '\n']) || t.len() < 1 {
            return Vec::new();
        }
        SLASH_COMMANDS
            .iter()
            .filter(|(n, _)| n.starts_with(&*t))
            .map(|(n, d)| (*n, *d))
            .collect()
    }

    pub(crate) fn slash(&mut self, t: &str) -> Option<Vec<UiCommand>> {
        let (name, args) = match t.split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim()),
            None => (t, ""),
        };
        match name {
            "/quit" | "/exit" | "/q" => {
                self.quit = true;
                Some(Vec::new())
            }
            "/undo" => Some(vec![UiCommand::UndoLastTurn]),
            "/retry" => Some(vec![UiCommand::Retry]),
            "/cancel" => Some(vec![UiCommand::Cancel]),
            "/help" => {
                self.commit(self.help_card());
                Some(Vec::new())
            }
            "/status" | "/settings" => {
                self.commit(self.status_card());
                Some(Vec::new())
            }
            "/model" => {
                if args.is_empty() {
                    // two-stage picker: provider → model (catalog from cfg)
                    if self.model_catalog.is_empty() {
                        self.commit(Cell::Note(format!(
                            "当前 {}/{}（配置无可用模型目录）",
                            self.provider_name, self.model
                        )));
                        return Some(Vec::new());
                    }
                    let providers: Vec<String> =
                        self.model_catalog.iter().map(|(p, _)| p.clone()).collect();
                    // pre-select the live provider — reopening shows where you are
                    let selected = providers
                        .iter()
                        .position(|p| *p == self.provider_name)
                        .unwrap_or(0);
                    self.modal = Some(Modal::Models {
                        stage: 0,
                        provider: self.provider_name.clone(),
                        items: providers,
                        selected,
                    });
                    Some(Vec::new())
                } else {
                    // `/model p/m` or `/model m` — bare m keeps the provider
                    let (provider, model) = match args.split_once('/') {
                        Some((p, m)) => (p.trim().to_string(), m.trim().to_string()),
                        None => (self.provider_name.clone(), args.to_string()),
                    };
                    if provider.is_empty() || model.is_empty() {
                        self.commit(Cell::Err("用法: /model [provider/]<model>".into()));
                        return Some(Vec::new());
                    }
                    self.provider_name = provider.clone();
                    self.model = model.clone();
                    Some(vec![UiCommand::SetModel { provider, model }])
                }
            }
            "/permission" => Some(self.pick_or_set(
                args,
                crate::modals::PickKind::Permission,
                &[
                    "default",
                    "acceptEdits",
                    "auto",
                    "dontAsk",
                    "bypassPermissions",
                ],
                &self.permission_mode.clone(),
                |v| UiCommand::SetPermissionMode { mode: v },
            )),
            "/mode" => Some(self.pick_or_set(
                args,
                crate::modals::PickKind::AgentMode,
                &["build", "plan", "goal"],
                &self.agent_mode.clone(),
                |v| UiCommand::SetAgentMode { mode: v },
            )),
            "/thinking" => Some(self.pick_or_set(
                args,
                crate::modals::PickKind::Thinking,
                &agent_llm::THINKING_LEVELS,
                &self.thinking_level.clone().unwrap_or_else(|| "默认".into()),
                |v| UiCommand::SetThinkingLevel { level: v },
            )),
            "/resume" => {
                let items = agent_store::SessionStore::open_existing(&self.workspace)
                    .map(|s| s.list())
                    .unwrap_or_default();
                // pre-select the live session so reopening shows where you are
                let selected = items
                    .iter()
                    .position(|m| m.id == self.session_id)
                    .unwrap_or(0);
                self.modal = Some(Modal::Sessions { items, selected });
                Some(Vec::new())
            }
            "/new" => {
                self.new_request = true;
                Some(Vec::new())
            }
            // Everything else starting with `/` belongs to the kernel's
            // CommandRegistry (/clear /compact /remember /forget /memory
            // /skills /diff /review…, plugin commands, user commands).
            s if s.starts_with('/') => Some(vec![UiCommand::Prompt {
                text: t.to_string(),
            }]),
            _ => None,
        }
    }

    /// `/permission` family: with an arg → validate + command; without →
    /// open the pick modal (current value pre-selected).
    pub(crate) fn pick_or_set<F>(
        &mut self,
        args: &str,
        kind: crate::modals::PickKind,
        options: &[&str],
        current: &str,
        set: F,
    ) -> Vec<UiCommand>
    where
        F: FnOnce(String) -> UiCommand,
    {
        if args.is_empty() {
            let selected = options.iter().position(|o| *o == current).unwrap_or(0);
            self.modal = Some(crate::modals::Modal::Pick {
                kind,
                items: options.iter().map(|s| s.to_string()).collect(),
                selected,
            });
            return Vec::new();
        }
        if options.contains(&args) {
            vec![set(args.to_string())]
        } else {
            self.commit(Cell::Err(format!(
                "无效 `{args}` — 可选: {}",
                options.join(" ")
            )));
            Vec::new()
        }
    }

    /// `/help` — committed card, not a modal: it belongs to the transcript.
    fn help_card(&self) -> Cell {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let key = Style::default().fg(Color::Cyan);
        let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(Color::DarkGray));
        let row =
            |k: &str, d: &str| Line::from(vec![Span::styled(format!("  {k:<13}"), key), dim(d)]);
        Cell::Lines(vec![
            Line::from(Span::styled(
                " 命令",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            row(
                "/model",
                "切换 provider/模型（Enter 两段选择 · /model p/m 直接换）",
            ),
            row(
                "/permission",
                "权限模式 default/acceptEdits/auto/dontAsk/bypassPermissions",
            ),
            row("/mode", "Agent 模式 build/plan/goal"),
            row(
                "/thinking",
                "思考强度 off/minimal/low/medium/high/xhigh/max",
            ),
            row("/resume", "恢复历史会话"),
            row("/status", "当前会话设置"),
            row("/clear", "清空历史（kernel）"),
            row("/compact", "手动压缩上下文（kernel）"),
            row("/memory", "记忆列表 · /remember 写入 · /forget 删除"),
            row("/skills", "已装技能目录（kernel）"),
            row("/undo", "回滚上一回合的文件改动"),
            row("/retry", "重发上一条 prompt"),
            row("/cancel", "打断当前回合（Esc/Ctrl+C 同效）"),
            row("/quit", "退出"),
            Line::from(dim(
                "  未列出的 /x 会送进模型（技能名、插件命令、用户命令）",
            )),
        ])
    }

    /// `/status` — the session's live settings, committed as a card.
    fn status_card(&self) -> Cell {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let key = Style::default().fg(Color::DarkGray);
        let row = |k: &str, v: String| {
            Line::from(vec![Span::styled(format!("  {k:<12}"), key), Span::raw(v)])
        };
        let mut out = vec![Line::from(Span::styled(
            " 状态",
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        out.push(row(
            "provider",
            format!("{}/{}", self.provider_name, self.model),
        ));
        out.push(row("permission", self.permission_mode.clone()));
        out.push(row("mode", self.agent_mode.clone()));
        out.push(row(
            "thinking",
            self.thinking_level.clone().unwrap_or_else(|| "默认".into()),
        ));
        out.push(row("sandbox", self.sandbox_id.clone()));
        out.push(row("workspace", self.workspace.display().to_string()));
        if let Some((_, used, window)) = self.usage {
            out.push(row("context", format!("{used}/{window} tok")));
        }
        Cell::Lines(out)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn slash_prefix_filters() {
        let cmds: Vec<(&str, &str)> = super::SLASH_COMMANDS
            .iter()
            .filter(|(n, _)| n.starts_with("/mo"))
            .map(|(n, d)| (*n, *d))
            .collect();
        assert_eq!(cmds.len(), 2, "expected /model + /mode, got {cmds:?}");
    }
}
