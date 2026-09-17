//! `App::view` — derives the widget tree each frame. Elm architecture:
//! `view(&self) -> Element`. Sidebar 220px + stream + steps strip + input +
//! status bar. Renders the ACTIVE session's `SessionView` only.

use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Column, Space,
};
use iced::{Alignment, Border, Element, Length};

use super::message::Message;
use super::state::{App, DiffKind, Role, SessionRow, StepState};
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
            list = list.push(self.session_row(s));
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
        let view = self.active_view();
        let mut col = Column::new().height(Length::Fill).width(Length::Fill);

        let mut msgs = Column::new().spacing(12).padding(16).width(Length::Fill);
        for (i, m) in view.messages.iter().enumerate() {
            msgs = msgs.push(self.message_row(i, m));
        }
        if view.is_active
            && !view.messages.last().map(|m| m.streaming).unwrap_or(false)
        {
            msgs = msgs.push(self.loading_dots());
        }

        let stream = scrollable(msgs)
            .id(self.stream_scroll.clone())
            .on_scroll(Message::Scrolled)
            .height(Length::Fill);
        col = col.push(stream);

        // Steps strip — only in-flight steps (running / awaiting confirm).
        if view
            .active_steps
            .iter()
            .any(|s| matches!(s.state, StepState::Running | StepState::AwaitingConfirm))
        {
            col = col.push(self.steps_strip(view));
        }

        col = col.push(self.input_capsule(view.is_active));
        col = col.push(self.status_bar(view));

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

    fn steps_strip<'a>(&'a self, view: &'a super::state::SessionView) -> Element<'a, Message> {
        let mut strip = Column::new().spacing(4).padding(8).width(Length::Fill);
        for (i, s) in view.active_steps.iter().enumerate() {
            if !matches!(s.state, StepState::Running | StepState::AwaitingConfirm) {
                continue;
            }
            strip = strip.push(self.step_capsule(view, i, s));
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

    fn step_capsule<'a>(
        &'a self,
        view: &'a super::state::SessionView,
        i: usize,
        s: &'a super::state::StepRow,
    ) -> Element<'a, Message> {
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
        let expanded_diff: Option<Element<_>> = if s.expanded && !view.pending_diff.is_empty() {
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

    fn status_bar<'a>(&'a self, _view: &'a super::state::SessionView) -> Element<'a, Message> {
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
