//! Tool call — an inline capsule in the stream: state glyph + tool icon +
//! name + one-line target. `AwaitingConfirm` adds allow/deny buttons; the
//! expanded body shows the staged diff (write tools) or full output.

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Border, Element, Length};

use crate::ui::animation;
use crate::ui::chat::diff;
use crate::ui::icons::{icon, Icon};
use crate::ui::message::Message;
use crate::ui::state::{DiffLine, SessionView, StepRow, StepState};
use crate::ui::theme;

/// The one-line capsule row — glyph + tool icon + name + target, plus
/// allow/deny when awaiting confirmation. Extracted to a function so the
/// `AnimationBuilder` closure can rebuild it each animation frame (the
/// builder requires `Fn`, not a pre-built `Element`, since `Element`
/// isn't `Clone`).
fn capsule_row<'a>(i: usize, s: &'a StepRow, tick: u64) -> Element<'a, Message> {
    // State glyph → a Keyline icon colored by the run state. Running swaps
    // the static loader for the rotating braille spinner (真·转圈圈).
    let (ic, glyph_color) = match s.state {
        StepState::Running => (Icon::Loader, theme::STATUS_RUNNING),
        StepState::AwaitingConfirm => (Icon::Zap, theme::ACCENT),
        StepState::Success => (Icon::Check, theme::DIFF_ADD),
        StepState::Error => (Icon::X, theme::DIFF_DEL),
        StepState::Denied => (Icon::X, theme::TEXT_MUTED),
    };
    let state_glyph: Element<_> = if s.state == StepState::Running {
        text(crate::ui::animation::spinner_at(tick).to_string())
            .size(12)
            .color(theme::STATUS_RUNNING)
            .font(theme::MONO)
            .into()
    } else {
        icon(ic, 12.0, glyph_color)
    };
    let detail_flat: String = s.detail.lines().next().unwrap_or("").to_string();
    // ONE icon only — the status glyph (✓/running/denied). A second
    // tool-type icon read as a stray placeholder next to it, so the name
    // sits directly after the status icon now.
    let mut row_el = row![
        state_glyph,
        text(&s.name).size(12).color(theme::TEXT_WHITE).font(theme::MONO),
        text(detail_flat)
            .size(11)
            .color(theme::TEXT_MUTED)
            .font(theme::MONO),
    ]
    .spacing(7)
    .align_y(Alignment::Center)
    .height(Length::Fixed(20.0));

    if s.state == StepState::AwaitingConfirm {
        row_el = row_el.push(Space::new().width(Length::Fill));
        row_el = row_el.push(
            button(
                row![
                    icon(Icon::Check, 13.0, theme::BG_WORKSPACE),
                    text(" allow").size(12).color(theme::BG_WORKSPACE),
                ]
                .spacing(3)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Approve(i))
            .padding([2.0, 10.0])
            .style(|_t, _st| button::Style {
                background: Some(theme::DIFF_ADD.into()),
                border: Border::default().rounded(theme::RADIUS_SM),
                ..Default::default()
            }),
        );
        row_el = row_el.push(
            button(
                row![
                    icon(Icon::X, 13.0, theme::TEXT_SECONDARY),
                    text(" deny").size(12).color(theme::TEXT_SECONDARY),
                ]
                .spacing(3)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Deny(i))
            .padding([2.0, 10.0])
            .style(|_t, _st| button::Style {
                background: Some(theme::BG_CARD.into()),
                border: Border {
                    width: 1.0,
                    color: theme::BORDER_HAIRLINE,
                    ..Default::default()
                }
                .rounded(theme::RADIUS_SM),
                ..Default::default()
            }),
        );
    }
    row_el.into()
}

pub fn view<'a>(i: usize, s: &'a StepRow, view: &'a SessionView, tick: u64) -> Element<'a, Message> {
    // Expanded body — the staged diff if this step has one (kept on
    // the row so it survives approve/deny), else the full tool output.
    // The body only exists when there's something to show; its height is
    // animated by `animation::expand` so it springs open instead of popping.
    let diff_src: &[DiffLine] = if !s.diff_lines.is_empty() {
        &s.diff_lines
    } else if s.state == StepState::AwaitingConfirm {
        &view.pending_diff
    } else {
        &[]
    };
    let has_body = !diff_src.is_empty() || !s.output.is_empty();
    // Height follows content — no artificial floor, capped only at 400 so
    // a large dump scrolls. Short outputs/diffs render at natural height.
    let full_h = if !diff_src.is_empty() {
        (diff_src.len() as f32 * 16.0 + 8.0).min(400.0)
    } else {
        let lines = s.output.lines().count().max(1) as f32;
        (lines * 15.0 + 16.0).min(400.0)
    };

    let expanded_body: Option<Element<_>> = if has_body {
        let body = move |t: f32| {
            // Keep the widget tree shape stable across the spring — at t≈0
            // render the SAME container shape with height≈0 and transparent
            // chrome, not a bare Space, so `scrollable`'s child index never
            // sees the tree kind change mid-animation (which panics).
            if !diff_src.is_empty() {
                diff::view_scaled(diff_src, t)
            } else {
                // Full tool output — sized to its line count (≈15px/line),
                // min 60 so a one-line result still looks like a panel,
                // max 400 so a huge dump scrolls instead of eating the
                // stream. Height scales by the spring `t`.
                let show_chrome = t > 0.05;
                container(
                    scrollable(
                        text(&s.output)
                            .size(11)
                            .color(theme::TEXT_SECONDARY)
                            .font(theme::MONO),
                    )
                    .height(Length::Fixed(full_h * t.clamp(0.0, 1.0)))
                    .width(Length::Fill)
                    .direction(theme::scrollable_dir())
                    .style(theme::scrollbar()),
                )
                .width(Length::Fill)
                .padding([6.0, 8.0])
                .style(move |_t| container::Style {
                    background: if show_chrome { Some(theme::BG_WORKSPACE.into()) } else { None },
                    border: Border {
                        width: if show_chrome { 1.0 } else { 0.0 },
                        color: theme::BORDER_HAIRLINE,
                        ..Default::default()
                    }
                    .rounded(theme::RADIUS_SM),
                    ..Default::default()
                })
                .into()
            }
        };
        // `expand` always mounts the body (target 1 open / 0 closed) — the
        // spring drives the height so it animates open AND closed.
        Some(animation::expand(s.expanded, body).into())
    } else {
        None
    };

    // Border color springs hairline→accent when the capsule starts
    // awaiting confirmation — `iced_anim` animates `Color` natively, so
    // the pulse is a real spring, not a hard swap.
    let border_target = if s.state == StepState::AwaitingConfirm {
        theme::ACCENT
    } else {
        theme::BORDER_HAIRLINE
    };
    let capsule = animation::AnimationBuilder::new(border_target, move |bc| {
        container(capsule_row(i, s, tick))
            .width(Length::Fill)
            .padding([7.0, 12.0])
            .style(move |_t| container::Style {
                background: Some(theme::BG_CARD.into()),
                border: Border {
                    width: 1.0,
                    color: bc,
                    ..Default::default()
                }
                .rounded(theme::RADIUS_MD),
                ..Default::default()
            })
            .into()
    })
    .animation(animation::SPRING_LAYOUT);
    let capsule_btn = button(capsule)
        .on_press(Message::ToggleStep(i))
        .padding(0)
        .width(Length::Fill)
        .style(theme::ghost_button());
    let mut wrap = column![capsule_btn].spacing(0).width(Length::Fill);
    if let Some(d) = expanded_body {
        wrap = wrap.push(d);
    }
    wrap.into()
}
