//! Modal cards that replace the composer region while a turn is parked —
//! a single `Option<Modal>` stack: approvals, questions, pickers.
//!
//! While a modal is open it owns every key. Resolutions send UiCommands:
//!   approval: y → approve, n/Esc → deny, a → approve + session-whitelist
//!   question: digit picks that option's label, typed text + Enter sends it,
//!             Esc sends an empty answer.

use agent_ipc::events::AskOption;
use agent_kernel::session_store::SessionMeta;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use agent_ipc::UiCommand;

pub enum Modal {
    Approval {
        request_id: u64,
        tool_name: String,
        diff: String,
        fuzzy: bool,
    },
    Question {
        request_id: u64,
        question: String,
        options: Vec<AskOption>,
        selected: usize,
        free: String,
    },
    /// `/resume` — pick a persisted session to reopen. The pick resolves
    /// OUTSIDE the kernel (SessionActor::resume rebuilds the actor), so the
    /// outcome is `Resume(id)`, not a UiCommand.
    Sessions {
        items: Vec<SessionMeta>,
        selected: usize,
    },
    /// `/model` — pick provider then model. Stage 0 = provider list,
    /// stage 1 = that provider's models. `items` is the visible list.
    Models {
        stage: u8,
        provider: String,
        items: Vec<String>,
        selected: usize,
    },
    /// `/permission`, `/mode`, `/thinking` — single-pick menus.
    /// `kind` decides which UiCommand the Enter resolution emits.
    Pick {
        kind: PickKind,
        items: Vec<String>,
        selected: usize,
    },
}

/// Which setting a `Pick` modal drives — kept as an enum (not a string) so
/// the Enter arm can't misspell a mode label into a silent no-op command.
#[derive(Clone, Copy)]
pub enum PickKind {
    Permission,
    AgentMode,
    Thinking,
}

/// What the modal decided — `app` sends each command in order and commits
/// the transcript echo line. Vec because `a` sends whitelist + verdict.
pub enum ModalOutcome {
    Send(Vec<UiCommand>),
    /// The picker wants a session reopened — the app passes the id up to
    /// `main`, which rebuilds the actor (no UiCommand exists for it).
    Resume(i64),
    /// Esc/close with no resolution.
    Close,
    Handled,
}

impl Modal {
    /// Rows the modal needs in the composer region (capped by caller).
    pub fn height(&self, width: usize, max: usize) -> usize {
        let h = match self {
            Modal::Approval { diff, .. } => {
                // title + N diff preview lines + footer
                2 + diff.lines().take(6).count() + 1
            }
            Modal::Question { options, .. } => {
                // title + windowed options + indicators + input
                2 + options.len().min(LIST_MAX) + 2 + 1
            }
            Modal::Sessions { .. } | Modal::Models { .. } | Modal::Pick { .. } => {
                // `items` differs in type per variant — len only.
                let n = match self {
                    Modal::Sessions { items, .. } => items.len(),
                    Modal::Models { items, .. } => items.len(),
                    Modal::Pick { items, .. } => items.len(),
                    _ => 0,
                };
                // items (windowed) + indicators + title + footer
                2 + n.min(LIST_MAX) + 2
            }
        };
        let _ = width;
        h.min(max)
    }

    /// Render into the composer-region width; returns styled lines
    /// (unwrapped — render.rs pads/clips to the region).
    pub fn lines(&self, width: usize) -> Vec<Line<'static>> {
        let warn = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(Color::DarkGray);
        let dimspan = |s: String| Span::styled(s, dim);
        match self {
            Modal::Approval {
                tool_name,
                diff,
                fuzzy,
                ..
            } => {
                let mut out = vec![Line::from(vec![
                    Span::styled("⚠ ", warn),
                    Span::styled(format!("审批: {tool_name}"), warn),
                    if *fuzzy {
                        Span::styled("  (fuzzy match)", dim)
                    } else {
                        Span::raw("")
                    },
                ])];
                for l in diff.lines().take(6) {
                    let st = if l.starts_with('+') {
                        Style::default().fg(Color::Green)
                    } else if l.starts_with('-') {
                        Style::default().fg(Color::Red)
                    } else {
                        dim
                    };
                    let clipped: String = l.chars().take(width.saturating_sub(2)).collect();
                    out.push(Line::from(Span::styled(format!("  {clipped}"), st)));
                }
                if diff.lines().count() > 6 {
                    out.push(Line::from(Span::styled(
                        format!("  … {} more", diff.lines().count() - 6),
                        dim,
                    )));
                }
                out.push(Line::from(vec![
                    Span::styled("[y]", warn),
                    Span::raw(" 批准  "),
                    Span::styled("[n]", warn),
                    Span::raw("/Esc 拒绝  "),
                    Span::styled("[a]", warn),
                    Span::raw(" 本轮放行此工具"),
                ]));
                out
            }
            Modal::Question {
                question,
                options,
                selected,
                free,
                ..
            } => {
                let mut out = vec![Line::from(vec![
                    Span::styled("? ", Style::default().fg(Color::Cyan)),
                    Span::raw(question.clone()),
                ])];
                let start = win_start(*selected, options.len());
                if start > 0 {
                    out.push(Line::from(dimspan(format!("  ↑ 还有 {start} 项"))));
                }
                for (i, o) in options.iter().enumerate().skip(start).take(LIST_MAX) {
                    let on = i == *selected;
                    let mut spans = vec![Span::styled(
                        format!("  {} ", i + 1),
                        if on {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            dim
                        },
                    )];
                    spans.push(Span::styled(
                        o.label.clone(),
                        if on {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    ));
                    if let Some(d) = &o.description {
                        spans.push(Span::styled(format!("  — {d}"), dim));
                    }
                    out.push(Line::from(spans));
                }
                let below = options.len().saturating_sub(start + LIST_MAX);
                if below > 0 {
                    out.push(Line::from(dimspan(format!("  ↓ 还有 {below} 项"))));
                }
                let mut inp = vec![Span::styled("  回答: ", dim), Span::raw(free.clone())];
                if free.is_empty() && options.is_empty() {
                    inp.push(Span::styled("（输入，Enter 发送）", dim));
                }
                out.push(Line::from(inp));
                out
            }
            Modal::Sessions { items, selected } => {
                let mut out = vec![Line::from(vec![
                    Span::styled("↺ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        "恢复会话",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])];
                if items.is_empty() {
                    out.push(Line::from(dimspan("  此工作区没有历史会话".to_string())));
                }
                let start = win_start(*selected, items.len());
                if start > 0 {
                    out.push(Line::from(dimspan(format!("  ↑ 还有 {start} 条"))));
                }
                for (i, m) in items.iter().enumerate().skip(start).take(LIST_MAX) {
                    let on = i == *selected;
                    let mark = if on { "▸ " } else { "  " };
                    let title = if m.title.trim().is_empty() {
                        format!("会话 #{}", m.id)
                    } else {
                        m.title.chars().take(42).collect()
                    };
                    let when = fmt_age(m.updated_at);
                    out.push(Line::from(vec![
                        Span::styled(
                            mark,
                            Style::default().fg(if on { Color::Cyan } else { Color::DarkGray }),
                        ),
                        Span::styled(
                            format!("{title:<44}"),
                            if on {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ),
                        dimspan(format!("  {when}")),
                    ]));
                }
                let below = items.len().saturating_sub(start + LIST_MAX);
                if below > 0 {
                    out.push(Line::from(dimspan(format!("  ↓ 还有 {below} 条"))));
                }
                out.push(Line::from(dimspan(
                    "  ↑/↓/PgUp 选择 · Enter 恢复 · 1-8 直选 · Esc 关闭".to_string(),
                )));
                out
            }
            Modal::Models {
                stage,
                provider,
                items,
                selected,
            } => {
                let title = if *stage == 0 {
                    "选择 provider".to_string()
                } else {
                    format!("{provider} — 选择模型")
                };
                let mut out = vec![Line::from(vec![
                    Span::styled("◆ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        title,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])];
                let start = win_start(*selected, items.len());
                if start > 0 {
                    out.push(Line::from(dimspan(format!("  ↑ 还有 {start} 项"))));
                }
                for (i, it) in items.iter().enumerate().skip(start).take(LIST_MAX) {
                    out.push(menu_row(i == *selected, it, &dimspan));
                }
                if items.len() > start + LIST_MAX {
                    out.push(Line::from(dimspan(format!(
                        "  ↓ 还有 {} 项",
                        items.len() - start - LIST_MAX
                    ))));
                }
                out.push(Line::from(dimspan(if *stage == 0 {
                    "  ↑/↓ 选择 · Enter 选 provider · Esc 关闭".to_string()
                } else {
                    "  ↑/↓ 选择 · Enter 应用 · ←/Esc 返回 · 输入过滤".to_string()
                })));
                out
            }
            Modal::Pick {
                kind,
                items,
                selected,
            } => {
                let title = match kind {
                    PickKind::Permission => "权限模式",
                    PickKind::AgentMode => "Agent 模式",
                    PickKind::Thinking => "思考强度",
                };
                let mut out = vec![Line::from(vec![
                    Span::styled("◆ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        title,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])];
                let start = win_start(*selected, items.len());
                for (i, it) in items.iter().enumerate().skip(start).take(LIST_MAX) {
                    out.push(menu_row(i == *selected, it, &dimspan));
                }
                if items.len() > LIST_MAX {
                    out.push(Line::from(dimspan(format!(
                        "  ({}/{})",
                        *selected + 1,
                        items.len()
                    ))));
                }
                out.push(Line::from(dimspan(
                    "  ↑/↓ 选择 · Enter 应用 · Esc 关闭".to_string(),
                )));
                out
            }
        }
    }

    /// One keypress → maybe a UiCommand. The caller commits the transcript
    /// echo line (approved/denied/answered) so the modal stays render-pure.
    pub fn handle_key(&mut self, key: KeyEvent) -> ModalOutcome {
        match self {
            Modal::Approval {
                request_id,
                tool_name,
                ..
            } => match (key.code, key.modifiers) {
                (KeyCode::Char('y'), _) => ModalOutcome::Send(vec![UiCommand::ToolDecision {
                    request_id: *request_id,
                    approved: true,
                }]),
                (KeyCode::Char('n'), _) | (KeyCode::Esc, _) => {
                    ModalOutcome::Send(vec![UiCommand::ToolDecision {
                        request_id: *request_id,
                        approved: false,
                    }])
                }
                // whitelist first, then the verdict — order matters for tools
                // that immediately re-dispatch
                (KeyCode::Char('a'), _) => ModalOutcome::Send(vec![
                    UiCommand::ApproveSessionTool {
                        tool_name: tool_name.clone(),
                    },
                    UiCommand::ToolDecision {
                        request_id: *request_id,
                        approved: true,
                    },
                ]),
                _ => ModalOutcome::Handled,
            },
            Modal::Question {
                request_id,
                options,
                selected,
                free,
                ..
            } => match key.code {
                KeyCode::Esc => ModalOutcome::Send(vec![UiCommand::AnswerQuestion {
                    request_id: *request_id,
                    answer: String::new(),
                }]),
                KeyCode::Enter => {
                    let answer = if free.is_empty() {
                        options
                            .get(*selected)
                            .map(|o| o.label.clone())
                            .unwrap_or_default()
                    } else {
                        std::mem::take(free)
                    };
                    ModalOutcome::Send(vec![UiCommand::AnswerQuestion {
                        request_id: *request_id,
                        answer,
                    }])
                }
                _ if list_nav(&key, selected, options.len(), false) => ModalOutcome::Handled,
                KeyCode::Char(c)
                    if c.is_ascii_digit()
                        && c != '0'
                        && (c as usize - '0' as usize) <= options.len()
                        && free.is_empty() =>
                {
                    let i = c as usize - '0' as usize - 1;
                    ModalOutcome::Send(vec![UiCommand::AnswerQuestion {
                        request_id: *request_id,
                        answer: options[i].label.clone(),
                    }])
                }
                KeyCode::Char(c) => {
                    free.push(c);
                    ModalOutcome::Handled
                }
                KeyCode::Backspace => {
                    free.pop();
                    ModalOutcome::Handled
                }
                _ => ModalOutcome::Handled,
            },
            Modal::Models {
                stage,
                provider,
                items,
                selected,
            } => {
                use agent_ipc::UiCommand;
                match key.code {
                    KeyCode::Esc => {
                        if *stage == 0 {
                            ModalOutcome::Close
                        } else {
                            *stage = 0;
                            ModalOutcome::Handled
                        }
                    }
                    KeyCode::Left if *stage == 1 => {
                        *stage = 0;
                        ModalOutcome::Handled
                    }
                    _ if list_nav(&key, selected, items.len(), true) => {
                        ModalOutcome::Handled
                    }
                    KeyCode::Enter => {
                        if *stage == 0 {
                            if let Some(p) = items.get(*selected) {
                                *provider = p.clone();
                                *stage = 1;
                                *selected = 0;
                                // stage-1 items are filled by app (it owns
                                // the model catalog) via `refill`.
                            }
                            ModalOutcome::Handled
                        } else {
                            items
                                .get(*selected)
                                .map(|m| {
                                    ModalOutcome::Send(vec![UiCommand::SetModel {
                                        provider: provider.clone(),
                                        model: m.clone(),
                                    }])
                                })
                                .unwrap_or(ModalOutcome::Close)
                        }
                    }
                    _ => ModalOutcome::Handled,
                }
            }
            Modal::Pick {
                kind,
                items,
                selected,
            } => match key.code {
                KeyCode::Esc => ModalOutcome::Close,
                _ if list_nav(&key, selected, items.len(), true) => ModalOutcome::Handled,
                KeyCode::Enter => items
                    .get(*selected)
                    .map(|v| {
                        let cmd = match kind {
                            PickKind::Permission => {
                                UiCommand::SetPermissionMode { mode: v.clone() }
                            }
                            PickKind::AgentMode => UiCommand::SetAgentMode { mode: v.clone() },
                            PickKind::Thinking => UiCommand::SetThinkingLevel { level: v.clone() },
                        };
                        ModalOutcome::Send(vec![cmd])
                    })
                    .unwrap_or(ModalOutcome::Close),
                _ => ModalOutcome::Handled,
            },
            Modal::Sessions { items, selected } => match key.code {
                KeyCode::Esc => ModalOutcome::Close,
                KeyCode::Enter => items
                    .get(*selected)
                    .map(|m| ModalOutcome::Resume(m.id))
                    .unwrap_or(ModalOutcome::Close),
                _ if list_nav(&key, selected, items.len(), true) => ModalOutcome::Handled,
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let i = c as usize - '0' as usize - 1;
                    if i < items.len().min(8) {
                        *selected = i;
                        ModalOutcome::Resume(items[i].id)
                    } else {
                        ModalOutcome::Handled
                    }
                }
                _ => ModalOutcome::Handled,
            },
        }
    }
}

/// Rows a pick-list may occupy inside the modal pane.
const LIST_MAX: usize = 9;

/// Shared navigation for every scroll-windowed list — one contract, no
/// per-modal hand-rolled clamps: ↑/k ↓/j step, PgUp/PgDn jump a window,
/// Home/End hit the edges. `alpha` gates the j/k aliases — lists that
/// accept typed input (Question's free-form answer) can't steal letters.
pub(crate) fn list_nav(key: &KeyEvent, selected: &mut usize, len: usize, alpha: bool) -> bool {
    if len == 0 {
        return false;
    }
    match key.code {
        KeyCode::Up => *selected = selected.saturating_sub(1),
        KeyCode::Down => *selected = (*selected + 1).min(len - 1),
        KeyCode::Char('k') if alpha => *selected = selected.saturating_sub(1),
        KeyCode::Char('j') if alpha => *selected = (*selected + 1).min(len - 1),
        KeyCode::PageUp => *selected = selected.saturating_sub(LIST_MAX),
        KeyCode::PageDown => *selected = (*selected + LIST_MAX).min(len - 1),
        KeyCode::Home => *selected = 0,
        KeyCode::End => *selected = len - 1,
        _ => return false,
    }
    true
}

/// First visible index for a scroll-windowed list — keeps `selected`
/// inside the window and pins to the tail at the last item.
pub(crate) fn win_start(selected: usize, len: usize) -> usize {
    if len <= LIST_MAX {
        0
    } else if selected + 1 >= len {
        len - LIST_MAX
    } else {
        selected.saturating_sub(LIST_MAX - 1)
    }
}

/// `updated_at` is unix seconds — render as a compact age ("3m" / "2h" /
/// "5d") instead of a timestamp nobody parses at a glance.
fn fmt_age(updated_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age = now.saturating_sub(updated_at);
    if age < 60 {
        "刚刚".into()
    } else if age < 3600 {
        format!("{}m前", age / 60)
    } else if age < 86400 {
        format!("{}h前", age / 3600)
    } else {
        format!("{}d前", age / 86400)
    }
}

/// One picker row: ▸ marker + bold when selected.

fn menu_row(on: bool, label: &str, _dimspan: &dyn Fn(String) -> Span<'static>) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            if on { "▸ " } else { "  " },
            Style::default().fg(if on { Color::Cyan } else { Color::DarkGray }),
        ),
        if on {
            Span::styled(
                label.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(label.to_string())
        },
    ])
}
