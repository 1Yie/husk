//! `App::view` — derives the widget tree each frame. Elm architecture:
//! `view(&self) -> Element`. Sidebar 220px + stream + steps strip + input.

use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Column, Space,
};
use iced::{Alignment, Border, Element, Length};

use super::message::Message;
use super::state::{App, Role, StepState};
use super::theme;

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
        let mut list = Column::new().spacing(2).padding(8);

        list = list.push(
            text("agent-rs")
                .size(16)
                .color(theme::TEXT_WHITE)
                .font(theme::SANS),
        );
        list = list.push(Space::new().height(Length::Fixed(12.0)));
        list = list.push(
            text("SESSIONS")
                .size(10)
                .color(theme::TEXT_MUTED),
        );
        list = list.push(Space::new().height(Length::Fixed(8.0)));

        for s in &self.sessions {
            let bg = if s.active { theme::BG_ACTIVE } else { theme::BG_PANEL };
            list = list.push(
                button(
                    column![
                        text(&s.title).size(12).color(theme::TEXT_SECONDARY),
                        text(&s.preview).size(10).color(theme::TEXT_MUTED),
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
                }),
            );
        }

        list = list.push(Space::new().height(Length::Fixed(12.0)));
        list = list.push(
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

        container(list)
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

    // ------------------------------------------------------------------
    // Stream column — messages + steps strip + input capsule.
    // ------------------------------------------------------------------
    fn stream_column(&self) -> Element<'_, Message> {
        let mut col = Column::new().height(Length::Fill).width(Length::Fill);

        // Message stream — scrollable, pinned to bottom unless user scrolled.
        let mut msgs = Column::new().spacing(12).padding(16).width(Length::Fill);
        for (i, m) in self.messages.iter().enumerate() {
            msgs = msgs.push(self.message_row(i, m));
        }
        // Loading dots while the agent is mid-turn and no streaming row yet.
        if self.is_active && !self.messages.last().map(|m| m.streaming).unwrap_or(false) {
            msgs = msgs.push(self.loading_dots());
        }

        let stream = scrollable(msgs)
            .id(self.stream_scroll.clone())
            .on_scroll(Message::Scrolled)
            .height(Length::Fill);
        col = col.push(stream);

        // Steps strip — capped height, internal scroll, pinned to bottom.
        if !self.active_steps.is_empty() {
            col = col.push(self.steps_strip());
        }

        // Input capsule.
        col = col.push(self.input_capsule());

        // Status bar — tokens / model / mode / sandbox.
        col = col.push(self.status_bar());

        col.into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
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

        container(
            row![
                text(state).size(10).color(theme::STATUS_RUNNING).font(theme::MONO),
                text(mode).size(10).color(theme::TEXT_MUTED).font(theme::MONO),
                text(sandbox).size(10).color(sandbox_color).font(theme::MONO),
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

    fn message_row<'a>(&'a self, i: usize, m: &'a super::state::MessageRow) -> Element<'a, Message> {
        match m.role {
            Role::User => {
                // Right-aligned plain text — no bubble.
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
                // Collapsed reasoning fold.
                if !m.reasoning.is_empty() {
                    let marker = if m.reasoning_open { "▾" } else { "▸" };
                    let head = text(format!("{marker} Thinking…"))
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
                // Body text + streaming caret.
                let body = if m.streaming {
                    format!("{}▮", m.text)
                } else {
                    m.text.clone()
                };
                c = c.push(
                    text(body)
                        .size(14)
                        .color(theme::TEXT_SECONDARY)
                        .font(theme::SANS),
                );
                c.into()
            }
        }
    }

    fn steps_strip(&self) -> Element<'_, Message> {
        let mut strip = Column::new().spacing(4).padding(8).width(Length::Fill);
        for (i, s) in self.active_steps.iter().enumerate() {
            strip = strip.push(self.step_capsule(i, s));
        }
        container(scrollable(strip).height(Length::Shrink))
            .width(Length::Fill)
            .max_height(116.0)
            .style(|_t| container::Style {
                background: Some(theme::BG_WORKSPACE.into()),
                border: Border {
                    width: 1.0,
                    color: theme::BORDER_HAIRLINE,
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn step_capsule<'a>(&'a self, i: usize, s: &'a super::state::StepRow) -> Element<'a, Message> {
        let (glyph, glyph_color) = match s.state {
            StepState::Running => ("●", theme::STATUS_RUNNING),
            StepState::AwaitingConfirm => ("◌", theme::ACCENT),
            StepState::Success => ("✓", theme::DIFF_ADD),
            StepState::Error => ("✗", theme::DIFF_DEL),
            StepState::Denied => ("⊘", theme::TEXT_MUTED),
        };

        let mut row_el = row![
            text(glyph).size(12).color(glyph_color).width(Length::Fixed(18.0)),
            text(&s.name).size(12).color(theme::TEXT_WHITE).font(theme::MONO),
            text(&s.detail).size(11).color(theme::TEXT_MUTED).font(theme::MONO),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        // Inline approve/deny on an awaiting capsule.
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

        // Expanded: show the pending diff inline under the capsule.
        let expanded_diff: Option<Element<_>> = if s.expanded && !self.pending_diff.is_empty() {
            let mut dcol = Column::new().spacing(0).width(Length::Fill);
            for d in self.pending_diff.iter().take(50) {
                let (gutter, fg, bg) = match d.kind {
                    super::state::DiffKind::Add => ("+", theme::DIFF_ADD, theme::DIFF_ADD_SURFACE),
                    super::state::DiffKind::Delete => ("-", theme::DIFF_DEL, theme::DIFF_DEL_SURFACE),
                    super::state::DiffKind::Context => (" ", theme::TEXT_MUTED, theme::BG_CARD),
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
                container(scrollable(dcol).height(Length::Fixed(200.0)))
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
        } else {
            None
        };

        let capsule = container(row_el)
            .width(Length::Fill)
            .padding([4.0, 8.0])
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
        // Wrap in a button so the capsule is tappable → expand/collapse.
        let capsule_btn = button(capsule)
            .on_press(Message::ToggleStep(i))
            .padding(0)
            .width(Length::Fill)
            .style(|_t, _st| button::Style {
                background: None,
                ..Default::default()
            });
        // Capsule + optional expanded diff body stacked vertically.
        let mut wrap = Column::new().spacing(0).width(Length::Fill).push(capsule_btn);
        if let Some(d) = expanded_diff {
            wrap = wrap.push(d);
        }
        wrap.into()
    }

    fn input_capsule(&self) -> Element<'_, Message> {
        let placeholder = if self.is_active {
            "Steer the agent…"
        } else {
            "Ask the agent…"
        };

        let input = text_input(placeholder, &self.input)
            .on_input(Message::InputChanged)
            .on_submit(if self.is_active { Message::Steer } else { Message::Submit })
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
                    .on_press(if self.is_active { Message::Steer } else { Message::Submit })
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

    fn loading_dots(&self) -> Element<'_, Message> {
        // Three staggered dots pulsing via the tick counter.
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
