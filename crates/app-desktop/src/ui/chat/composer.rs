//! Composer — the input capsule at the bottom of the stream. Placeholder
//! flips between "Ask…" and "Steer…" by whether a turn is active; the
//! action button is a red ✕ cancel while running, a send arrow otherwise.

use iced::widget::{button, container, row, text_input};
use iced::{Alignment, Element, Length};

use crate::ui::icons::{icon, Icon};
use crate::ui::message::Message;
use crate::ui::state::App;
use crate::ui::theme;

pub fn view(app: &App, is_active: bool) -> Element<'_, Message> {
    let placeholder = if is_active {
        "Steer the agent…"
    } else {
        "Ask the agent…"
    };

    let input = text_input(placeholder, &app.input)
        .on_input(Message::InputChanged)
        .on_submit(if is_active { Message::Steer } else { Message::Submit })
        .padding(10)
        .size(15)
        .style(theme::composer_input());

    // While a turn is running the action button becomes a stop (✕) —
    // pressing it sets the session's cancel flag and aborts the turn
    // mid-stream; otherwise it's the normal send/steer submit.
    // Pill action buttons — fully rounded, the modern send/stop chip.
    let action = if is_active {
        button(icon(Icon::X, 18.0, theme::BG_WORKSPACE))
            .on_press(Message::Cancel)
            .padding([9.0, 18.0])
            .style(theme::pill(theme::ERROR, theme::BG_WORKSPACE))
    } else {
        button(icon(Icon::Send, 18.0, theme::BG_WORKSPACE))
            .on_press(Message::Submit)
            .padding([9.0, 18.0])
            .style(theme::pill(theme::ACCENT, theme::BG_WORKSPACE))
    };

    container(row![input, action].spacing(10).align_y(Alignment::Center))
        .padding([14.0, 16.0])
        .width(Length::Fill)
        .into()
}
