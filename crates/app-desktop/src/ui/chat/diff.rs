//! Diff — render a unified diff as colored +/- lines. Add on green
//! surface, delete on red surface, context dim. Height follows line count,
//! capped + scrollable so a huge diff doesn't eat the stream.

use iced::widget::{container, scrollable, text, Column};
use iced::{Border, Element, Length};

use crate::ui::message::Message;
use crate::ui::state::{DiffKind, DiffLine};
use crate::ui::theme;

/// Render a unified diff at `t` of its natural height — `t=1` is fully
/// open, `t=0` collapsed; the `animation::expand` spring passes `t` in so
/// the diff opens smoothly instead of popping.
pub fn view_scaled<'a>(lines: &'a [DiffLine], t: f32) -> Element<'a, Message> {
    let mut dcol = Column::new().spacing(0).width(Length::Fill);
    for d in lines.iter().take(80) {
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
    // Height matches CONTENT exactly — no min-height floor, so a 3-line
    // diff renders as a 3-line panel (the old `clamp(60,400)` floor left
    // dead space under short diffs). Only an upper cap keeps huge dumps
    // scrollable instead of eating the stream.
    let full = (lines.len() as f32 * 16.0 + 8.0).min(400.0);
    let h = (full * t.clamp(0.0, 1.0)).max(0.0);
    // Keep the widget TREE shape stable across the animation — returning a
    // different widget kind at t≈0 (e.g. a Space) makes `scrollable`'s
    // internal child index panic when AnimationBuilder rebuilds the tree
    // mid-spring. Instead we keep the same container(scrollable) shape and
    // just fade the border to transparent + collapse height.
    let border_alpha = if t > 0.05 { 1.0 } else { 0.0 };
    container(scrollable(dcol).height(Length::Fixed(h)).width(Length::Fill).direction(theme::scrollable_dir()).style(theme::scrollbar()))
        .width(Length::Fill)
        .style(move |_t| container::Style {
            border: Border {
                width: if border_alpha > 0.0 { 1.0 } else { 0.0 },
                color: theme::BORDER_HAIRLINE,
                ..Default::default()
            }
            .rounded(theme::RADIUS_SM),
            ..Default::default()
        })
        .into()
}
