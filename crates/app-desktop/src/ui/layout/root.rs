//! Root layout — titlebar on top, then `[sidebar | stream_column]` below.

use iced::widget::{column, row};
use iced::Element;

use crate::ui::chat::conversation;
use crate::ui::layout::{sidebar, titlebar};
use crate::ui::message::Message;
use crate::ui::state::App;

pub fn view(app: &App) -> Element<'_, Message> {
    column![
        titlebar::view(app),
        row![
            sidebar::view(app),
            conversation::view(app),
        ],
    ]
    .into()
}
