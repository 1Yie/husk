//! Message — a single chat row. `User` right-aligned, `System` mono/warn,
//! `Agent` a markdown body with a collapsible reasoning header, copy/select
//! affordances, and a streaming caret.

use iced::widget::{button, container, row, text, Column, Space};
use iced::{Alignment, Border, Element, Length};

use crate::ui::animation;
use crate::ui::icons::{icon, Icon};
use crate::ui::message::Message;
use crate::ui::state::{MessageRow, Role};
use crate::ui::theme;

/// Codex-flavored markdown styling — inline code on a card chip, monospace
/// code blocks, accent-colored links.
fn md_style() -> iced::widget::markdown::Style {
    iced::widget::markdown::Style {
        font: theme::SANS,
        inline_code_highlight: iced::widget::markdown::Highlight {
            background: theme::BG_CARD.into(),
            border: Border::default().rounded(theme::RADIUS_SM),
        },
        inline_code_padding: iced::padding::left(2).right(2),
        inline_code_color: theme::ACCENT,
        inline_code_font: theme::MONO,
        code_block_font: theme::MONO,
        link_color: theme::ACCENT,
    }
}

pub fn view(i: usize, m: &MessageRow) -> Element<'_, Message> {
    match m.role {
        // User → a right-aligned chat bubble (tinted, large radius, fills
        // only ~70% width like a modern messenger).
        Role::User => {
            let bubble: Element<Message> = container(
                text(&m.text)
                    .size(15)
                    .color(theme::TEXT_WHITE)
                    .font(theme::SANS),
            )
            .padding([9.0, 14.0])
            .style(theme::bubble(true))
            .into();
            container(bubble)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .padding([2.0, 0.0])
                .into()
        }
        // System → a quiet centered caption, sans not mono.
        Role::System => container(
            text(&m.text)
                .size(13)
                .color(theme::WARN)
                .font(theme::SANS),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into(),
        Role::Agent => agent_row(i, m),
    }
}

fn agent_row(i: usize, m: &MessageRow) -> Element<'_, Message> {
    // Agent → a left-aligned bubble card (panel fill, large radius) — the
    // modern messenger look instead of full-bleed TUI text.
    let mut c = Column::new().spacing(4).width(Length::Fill);
    if !m.reasoning.is_empty() {
        let marker = if m.reasoning_open { "▾" } else { "▸" };
        // Live phase → "Thinking…"; done → a quiet "Thought".
        let label = if m.thinking_done { "Thought" } else { "Thinking…" };
        let head = text(format!("{marker} {label}"))
            .size(13)
            .color(theme::TEXT_MUTED)
            .font(theme::SANS);
        c = c.push(
            button(head)
                .on_press(Message::ToggleReasoning(i))
                .padding(0)
                .style(theme::ghost_button()),
        );
        // Reasoning body springs open/closed via `animation::expand` — its
        // height scales with the spring `t` and the text fades in with it.
        let reasoning = &m.reasoning;
        c = c.push(
            animation::expand(m.reasoning_open, move |t| {
                // Keep the widget tree stable: same container(text) shape
                // across the spring (a Space at t≈0 swaps the widget kind
                // and crashes AnimationBuilder's tree diff). Height →0 and
                // text→transparent does the collapse without a kind change.
                let lines = reasoning.lines().count().max(1) as f32;
                // Content height, capped at 400 — no floor, so a 2-line
                // reasoning block doesn't get a tall empty panel.
                let full = (lines * 18.0 + 4.0).min(400.0);
                let c = animation::fade(theme::TEXT_DIM, t);
                container(
                    text(reasoning)
                        .size(13)
                        .color(c)
                        .font(theme::SANS),
                )
                .height(Length::Fixed(full * t.clamp(0.0, 1.0)))
                .width(Length::Fill)
                .into()
            })
        );
    }
    // Body: markdown view normally; a read-only text_editor when
    // the user toggles "select" so they can drag-select + Ctrl+C.
    if m.selectable {
        let ed = iced::widget::text_editor::TextEditor::new(&m.editor)
            .font(theme::MONO)
            .size(15)
            .on_action(move |a| Message::SelectAction(i, a))
            .style(|_t, _st| iced::widget::text_editor::Style {
                background: iced::Background::Color(theme::BG_CARD),
                border: iced::Border {
                    color: theme::BORDER_HAIRLINE,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                placeholder: theme::TEXT_MUTED,
                value: theme::TEXT_WHITE,
                selection: iced::Color::from_rgba8(0xe4, 0xe4, 0xe7, 0.3),
            });
        c = c.push(ed);
    } else {
        let md = iced::widget::markdown::view(
            &m.items,
            iced::widget::markdown::Settings::with_text_size(14, md_style()),
        )
        .map(Message::LinkClicked);
        c = c.push(md);
    }
    if m.streaming {
        // The caret breathes via its `iced_anim` pulse — alpha rides the
        // looping `Animated<f32>` advanced by `Message::Tick`.
        let c_color = animation::fade(theme::ACCENT, *m.caret.value());
        c = c.push(text("▮").size(15).color(c_color));
    } else {
        // Affordances: ⧉ copy (whole answer) + ⿻ select (toggle
        // into a selectable-text view for drag-select + Ctrl+C).
        let select_label = if m.selectable { "back" } else { "select" };
        c = c.push(
            row![
                button(
                    row![
                        icon(Icon::Copy, 13.0, theme::TEXT_MUTED),
                        text(" copy").size(11).color(theme::TEXT_MUTED),
                    ]
                    .spacing(3)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::CopyMessage(i))
                .padding([2, 0])
                .style(theme::ghost_button()),
                Space::new().width(Length::Fixed(12.0)),
                button(
                    row![
                        icon(Icon::SquarePen, 13.0, theme::TEXT_MUTED),
                        text(select_label).size(11).color(theme::TEXT_MUTED),
                    ]
                    .spacing(3)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::ToggleSelect(i))
                .padding([2, 0])
                .style(theme::ghost_button()),
            ]
            .spacing(0),
        );
    }
    // Wrap the whole agent message in a bubble card — the modern chat
    // surface. Fills only ~92% width so the bubble has a visible edge.
    container(c)
        .padding([10.0, 14.0])
        .width(Length::FillPortion(11))
        .style(theme::bubble(false))
        .into()
}
