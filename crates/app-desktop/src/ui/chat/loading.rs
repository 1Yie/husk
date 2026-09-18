//! Loading dots — the three-dot "agent is thinking" indicator appended to
//! the stream while a turn is active. Each dot's alpha is driven by an
//! `iced_anim` looping pulse on the `SessionView` (phase-offset so they
//! travel as a wave), not a hand-rolled tick phase.

use iced::widget::{container, row, text};
use iced::Element;

use crate::ui::animation;
use crate::ui::message::Message;
use crate::ui::state::SessionView;
use crate::ui::theme;

pub fn view<'a>(view: &'a SessionView) -> Element<'a, Message> {
    let dots: Vec<Element<_>> = view
        .dot_pulses
        .iter()
        .map(|p| {
            // Pulse value 0→1→0 drives the dot's alpha — a wave because each
            // dot's phase is offset in `SessionView::default`.
            let c = animation::fade(theme::TEXT_SECONDARY, *p.value());
            text("●").size(15).color(c).into()
        })
        .collect();
    container(row(dots).spacing(4))
        .padding([4.0, 16.0])
        .into()
}
