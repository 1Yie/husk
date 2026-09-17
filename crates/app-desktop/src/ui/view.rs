//! `App::view` — derives the widget tree each frame. Elm architecture:
//! `view(&self) -> Element`. Sidebar 220px + stream + steps strip + input +
//! status bar. Renders the ACTIVE session's `SessionView` only.

use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Column, Space,
};
use iced::{Alignment, Border, Element, Length};

use super::message::Message;
use super::state::{App, DiffKind, Role, SessionRow, StepState, StreamItem};
use super::theme;

/// Codex-flavored markdown styling — inline code on a card chip, monospace
/// code blocks, accent-colored links.
fn md_style() -> iced::widget::markdown::Style {
    iced::widget::markdown::Style {
        font: theme::SANS,
        inline_code_highlight: iced::widget::markdown::Highlight {
            background: theme::BG_CARD.into(),
            border: Border::default().rounded(3.0),
        },
        inline_code_padding: iced::padding::left(2).right(2),
        inline_code_color: theme::ACCENT,
        inline_code_font: theme::MONO,
        code_block_font: theme::MONO,
        link_color: theme::ACCENT,
    }
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        row![
            self.sidebar(),
            self.stream_column(),
        ]
        .into()
    }

    // ------------------------------------------------------------------
    // Sidebar — session list, 220px.
    // ------------------------------------------------------------------
    fn sidebar(&self) -> Element<'_, Message> {
        // Fixed header — app name + new-session button stay put while the
        // session list scrolls below.
        let mut header = Column::new().spacing(0).padding(8);
        header = header.push(
            text("agent-rs")
                .size(16)
                .color(theme::TEXT_WHITE)
                .font(theme::SANS),
        );
        header = header.push(Space::new().height(Length::Fixed(10.0)));
        header = header.push(
            button(text("+ new session").size(11).color(theme::TEXT_SECONDARY))
                .on_press(Message::NewSession)
                .width(Length::Fill)
                .padding(8)
                .style(|_t, _st| button::Style {
                    background: Some(theme::BG_CARD.into()),
                    border: Border::default().rounded(4.0),
                    ..Default::default()
                }),
        );
        header = header.push(Space::new().height(Length::Fixed(8.0)));
        header = header.push(
            text("SESSIONS")
                .size(10)
                .color(theme::TEXT_MUTED),
        );
        header = header.push(Space::new().height(Length::Fixed(4.0)));

        // Scrollable session list — fills the space under the header.
        let mut list = Column::new().spacing(2).padding([0, 8]);
        for s in &self.sessions {
            list = list.push(self.session_row(s));
        }

        let body = column![header, scrollable(list).height(Length::Fill)]
            .height(Length::Fill);

        container(body)
            .width(Length::Fixed(220.0))
            .height(Length::Fill)
            .style(|_t| container::Style {
                background: Some(theme::BG_PANEL.into()),
                border: Border {
                    width: 1.0,
                    color: theme::BORDER_HAIRLINE,
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn session_row<'a>(&'a self, s: &'a SessionRow) -> Element<'a, Message> {
        let bg = if s.active { theme::BG_ACTIVE } else { theme::BG_PANEL };
        // A background session mid-turn shows a running marker.
        let preview = if s.running {
            format!("⟳ {}", s.preview)
        } else {
            s.preview.clone()
        };
        button(
            column![
                row![
                    text(&s.title).size(12).color(theme::TEXT_SECONDARY),
                    Space::new().width(Length::Fill),
                    if s.running {
                        text("●").size(9).color(theme::STATUS_RUNNING)
                    } else {
                        text("").size(9)
                    },
                ],
                text(preview).size(10).color(theme::TEXT_MUTED),
            ]
            .spacing(2),
        )
        .on_press(Message::SelectSession(s.id))
        .width(Length::Fill)
        .padding(8)
        .style(move |_t, _st| button::Style {
            background: Some(bg.into()),
            border: Border::default().rounded(4.0),
            text_color: theme::TEXT_SECONDARY,
            ..Default::default()
        })
        .into()
    }

    // ------------------------------------------------------------------
    // Stream column — messages + steps strip + input capsule + status bar.
    // ------------------------------------------------------------------
    fn stream_column(&self) -> Element<'_, Message> {
        let mut col = Column::new().height(Length::Fill).width(Length::Fill);

        let mut msgs = Column::new().spacing(10).padding(16).width(Length::Fill);
        let mut is_active = false;
        if let Some(view) = self.active_view() {
            is_active = view.is_active;
            for (i, item) in view.stream.iter().enumerate() {
                match item {
                    StreamItem::Message(m) => msgs = msgs.push(self.message_row(i, m)),
                    StreamItem::Tool(s) => msgs = msgs.push(self.tool_inline(i, s, view)),
                }
            }
            if view.is_active
                && !matches!(
                    view.stream.last(),
                    Some(StreamItem::Message(m)) if m.streaming
                )
            {
                msgs = msgs.push(self.loading_dots());
            }
        } else {
            msgs = msgs.push(
                text("Start a session — type below or pick one from the sidebar.")
                    .size(13)
                    .color(theme::TEXT_DIM),
            );
        }

        let stream = scrollable(msgs)
            .id(self.stream_scroll.clone())
            .on_scroll(Message::Scrolled)
            .height(Length::Fill);
        col = col.push(stream);

        col = col.push(self.input_capsule(is_active));
        col = col.push(self.status_bar());

        col.into()
    }

    fn message_row<'a>(
        &'a self,
        i: usize,
        m: &'a super::state::MessageRow,
    ) -> Element<'a, Message> {
        match m.role {
            Role::User => {
                container(
                    text(&m.text)
                        .size(14)
                        .color(theme::TEXT_WHITE)
                        .font(theme::SANS),
                )
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .padding([4.0, 0.0])
                .into()
            }
            Role::System => container(
                text(&m.text)
                    .size(12)
                    .color(theme::WARN)
                    .font(theme::MONO),
            )
            .width(Length::Fill)
            .into(),
            Role::Agent => {
                let mut c = Column::new().spacing(4).width(Length::Fill);
                if !m.reasoning.is_empty() {
                    let marker = if m.reasoning_open { "▾" } else { "▸" };
                    // Live phase → "Thinking…"; done → a quiet "Thought".
                    let label = if m.thinking_done { "Thought" } else { "Thinking…" };
                    let head = text(format!("{marker} {label}"))
                        .size(11)
                        .color(theme::TEXT_MUTED)
                        .font(theme::MONO);
                    c = c.push(
                        button(head)
                            .on_press(Message::ToggleReasoning(i))
                            .padding(0)
                            .style(|_t, _st| button::Style {
                                background: None,
                                ..Default::default()
                            }),
                    );
                    if m.reasoning_open {
                        c = c.push(
                            text(&m.reasoning)
                                .size(11)
                                .color(theme::TEXT_DIM)
                                .font(theme::MONO),
                        );
                    }
                }
                // Markdown-render the agent body (items kept in sync with
                // `text` on each append). Streaming appends a caret to the
                // raw text — the caret lands inside the last paragraph.
                let md = iced::widget::markdown::view(
                    &m.items,
                    iced::widget::markdown::Settings::with_text_size(14, md_style()),
                )
                .map(Message::LinkClicked);
                c = c.push(md);
                if m.streaming {
                    c = c.push(text("▮").size(14).color(theme::ACCENT));
                } else {
                    // Copy affordance — markdown isn't selectable in iced, so
                    // a quiet "⧉ copy" under the answer lets the user grab the
                    // full text (raw markdown source) to the clipboard.
                    c = c.push(
                        button(text("⧉ copy").size(10).color(theme::TEXT_MUTED))
                            .on_press(Message::CopyMessage(i))
                            .padding([2, 0])
                            .style(|_t, _st| button::Style {
                                background: None,
                                ..Default::default()
                            }),
                    );
                }
                c.into()
            }
        }
    }

    /// A tool call rendered INLINE in the stream — compact capsule showing
    /// `● name detail`, expanding to the pending diff, with allow/deny when
    /// it awaits confirmation. The chain stays in the message flow so the
    /// reader sees what the agent did, in order.
    fn tool_inline<'a>(
        &'a self,
        i: usize,
        s: &'a super::state::StepRow,
        view: &'a super::state::SessionView,
    ) -> Element<'a, Message> {
        let (glyph, glyph_color) = match s.state {
            StepState::Running => ("●", theme::STATUS_RUNNING),
            StepState::AwaitingConfirm => ("◌", theme::ACCENT),
            StepState::Success => ("✓", theme::DIFF_ADD),
            StepState::Error => ("✗", theme::DIFF_DEL),
            StepState::Denied => ("⊘", theme::TEXT_MUTED),
        };

        // The capsule is ONE line: glyph + name + a single-line target
        // summary (detail is already flat — args preview, or the first line
        // of output). Full output lives in the expandable body below.
        let detail_flat: String = s.detail.lines().next().unwrap_or("").to_string();
        let mut row_el = row![
            text(glyph).size(11).color(glyph_color).width(Length::Fixed(16.0)),
            text(&s.name).size(12).color(theme::TEXT_WHITE).font(theme::MONO),
            text(detail_flat)
                .size(11)
                .color(theme::TEXT_MUTED)
                .font(theme::MONO),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .height(Length::Fixed(22.0));

        if s.state == StepState::AwaitingConfirm {
            row_el = row_el.push(Space::new().width(Length::Fill));
            row_el = row_el.push(
                button(text("allow").size(10).color(theme::BG_WORKSPACE))
                    .on_press(Message::Approve(i))
                    .padding([2.0, 10.0])
                    .style(|_t, _st| button::Style {
                        background: Some(theme::DIFF_ADD.into()),
                        border: Border::default().rounded(4.0),
                        ..Default::default()
                    }),
            );
            row_el = row_el.push(
                button(text("deny").size(10).color(theme::TEXT_SECONDARY))
                    .on_press(Message::Deny(i))
                    .padding([2.0, 10.0])
                    .style(|_t, _st| button::Style {
                        background: Some(theme::BG_CARD.into()),
                        border: Border {
                            width: 1.0,
                            color: theme::BORDER_HAIRLINE,
                            ..Default::default()
                        }
                        .rounded(4.0),
                        ..Default::default()
                    }),
            );
        }

        // Expanded body — the pending diff if one is staged, else the full
        // tool output (so `smart_read`/`bash` results are readable).
        let expanded_diff: Option<Element<_>> = if s.expanded && !view.pending_diff.is_empty() && s.state == StepState::AwaitingConfirm {
            let mut dcol = Column::new().spacing(0).width(Length::Fill);
            for d in view.pending_diff.iter().take(50) {
                let (gutter, fg, bg) = match d.kind {
                    DiffKind::Add => ("+", theme::DIFF_ADD, theme::DIFF_ADD_SURFACE),
                    DiffKind::Delete => ("-", theme::DIFF_DEL, theme::DIFF_DEL_SURFACE),
                    DiffKind::Context => (" ", theme::TEXT_MUTED, theme::BG_CARD),
                };
                dcol = dcol.push(
                    container(
                        text(format!("{gutter} {}", d.content))
                            .size(11)
                            .color(fg)
                            .font(theme::MONO),
                    )
                    .width(Length::Fill)
                    .padding([1.0, 8.0])
                    .style(move |_t| container::Style {
                        background: Some(bg.into()),
                        ..Default::default()
                    }),
                );
            }
            Some(
                container(scrollable(dcol).height(Length::Fixed(200.0)).width(Length::Fill))
                    .width(Length::Fill)
                    .style(|_t| container::Style {
                        border: Border {
                            width: 1.0,
                            color: theme::BORDER_HAIRLINE,
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into(),
            )
        } else if s.expanded && !s.output.is_empty() {
            // Full tool output — the whole result, scrollable, not truncated.
            Some(
                container(
                    scrollable(
                        text(&s.output)
                            .size(11)
                            .color(theme::TEXT_SECONDARY)
                            .font(theme::MONO),
                    )
                    .height(Length::Fixed(180.0))
                    .width(Length::Fill), // scrollbar hugs the panel edge, not the text
                )
                .width(Length::Fill)
                .padding([6.0, 8.0])
                .style(|_t| container::Style {
                    background: Some(theme::BG_WORKSPACE.into()),
                    border: Border {
                        width: 1.0,
                        color: theme::BORDER_HAIRLINE,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
            )
        } else {
            None
        };

        let capsule = container(row_el)
            .width(Length::Fill)
            .padding([3.0, 8.0])
            .style(move |_t| container::Style {
                background: Some(theme::BG_CARD.into()),
                border: Border {
                    width: 1.0,
                    color: if s.state == StepState::AwaitingConfirm {
                        theme::ACCENT
                    } else {
                        theme::BORDER_HAIRLINE
                    },
                    ..Default::default()
                }
                .rounded(4.0),
                ..Default::default()
            });
        let capsule_btn = button(capsule)
            .on_press(Message::ToggleStep(i))
            .padding(0)
            .width(Length::Fill)
            .style(|_t, _st| button::Style {
                background: None,
                ..Default::default()
            });
        let mut wrap = Column::new().spacing(0).width(Length::Fill).push(capsule_btn);
        if let Some(d) = expanded_diff {
            wrap = wrap.push(d);
        }
        wrap.into()
    }

    fn input_capsule(&self, is_active: bool) -> Element<'_, Message> {
        let placeholder = if is_active {
            "Steer the agent…"
        } else {
            "Ask the agent…"
        };

        let input = text_input(placeholder, &self.input)
            .on_input(Message::InputChanged)
            .on_submit(if is_active { Message::Steer } else { Message::Submit })
            .padding(10)
            .size(14)
            .style(|_t, _st| text_input::Style {
                background: theme::BG_CARD.into(),
                border: Border {
                    width: 1.0,
                    color: theme::BORDER_HAIRLINE,
                    ..Default::default()
                }
                .rounded(8.0),
                icon: theme::TEXT_MUTED,
                placeholder: theme::TEXT_DIM,
                value: theme::TEXT_WHITE,
                selection: theme::BG_ACTIVE,
            });

        container(
            row![
                input,
                button(text("→").size(16).color(theme::BG_WORKSPACE))
                    .on_press(if is_active { Message::Steer } else { Message::Submit })
                    .padding([8.0, 16.0])
                    .style(|_t, _st| button::Style {
                        background: Some(theme::ACCENT.into()),
                        border: Border::default().rounded(8.0),
                        ..Default::default()
                    }),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(12)
        .width(Length::Fill)
        .into()
    }

    fn status_bar<'a>(&'a self) -> Element<'a, Message> {
        let tok = format!(
            "{} / {} tok",
            self.stats.tokens_used, self.stats.context_window
        );
        let model = format!("{} · {}", self.stats.active_provider, self.stats.active_model);
        let sandbox = if self.sandbox_unsafe { "UNSANDBOXED" } else { "sandbox" };
        let sandbox_color = if self.sandbox_unsafe { theme::WARN } else { theme::TEXT_MUTED };
        let state = self.stats.agent_state.clone();
        let mode = self.stats.permission_mode.clone();
        let files = format!("{} files", self.stats.files_changed);
        let sid = format!("session #{}", self.active_id);

        container(
            row![
                text(state).size(10).color(theme::STATUS_RUNNING).font(theme::MONO),
                text(mode).size(10).color(theme::TEXT_MUTED).font(theme::MONO),
                text(sandbox).size(10).color(sandbox_color).font(theme::MONO),
                text(sid).size(10).color(theme::TEXT_DIM).font(theme::MONO),
                Space::new().width(Length::Fill),
                text(files).size(10).color(theme::TEXT_MUTED).font(theme::MONO),
                text(tok).size(10).color(theme::TEXT_MUTED).font(theme::MONO),
                text(model).size(10).color(theme::TEXT_SECONDARY).font(theme::MONO),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([6.0, 12.0])
        .style(|_t| container::Style {
            background: Some(theme::BG_PANEL.into()),
            border: Border {
                width: 1.0,
                color: theme::BORDER_HAIRLINE,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn loading_dots(&self) -> Element<'_, Message> {
        let phase = (self.tick % 60) as f32 / 60.0;
        let dots: Vec<Element<_>> = (0..3)
            .map(|i| {
                let p = (phase + i as f32 * 0.33) % 1.0;
                let on = p < 0.5;
                text("●")
                    .size(10)
                    .color(if on { theme::TEXT_SECONDARY } else { theme::TEXT_DIM })
                    .into()
            })
            .collect();
        container(row(dots).spacing(4))
            .padding([4.0, 16.0])
            .into()
    }
}
