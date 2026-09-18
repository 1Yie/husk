//! Conversation — the scrollable message+tool stream plus the composer and
//! status bar that anchor the main column. Dispatches each `StreamItem` to
//! `message::view` or `tool_call::view`.

use iced::widget::{scrollable, text, Column};
use iced::{Element, Length};

use crate::ui::chat::{composer, loading, message, tool_call};
use crate::ui::layout::statusbar;
use crate::ui::message::Message;
use crate::ui::state::{App, StreamItem};
use crate::ui::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut col = Column::new().height(Length::Fill).width(Length::Fill);

    let mut msgs = Column::new().spacing(10).padding(16).width(Length::Fill);
    let mut is_active = false;
    // A session whose history is still being rebuilt off-thread shows a
    // loading surface instead of an empty stream — switching feels instant
    // because the heavy `view_from_history` work ran in `Task::perform`.
    if app.loading_session == Some(app.active_id) {
        msgs = msgs.push(
            text("Loading session…")
                .size(15)
                .color(theme::TEXT_MUTED),
        );
    } else if let Some(view) = app.active_view() {
        is_active = view.is_active;
        for (i, item) in view.stream.iter().enumerate() {
            match item {
                StreamItem::Message(m) => msgs = msgs.push(message::view(i, m)),
                StreamItem::Tool(s) => msgs = msgs.push(tool_call::view(i, s, view, app.tick_count)),
            }
        }
        if view.is_active
            && !matches!(
                view.stream.last(),
                Some(StreamItem::Message(m)) if m.streaming
            )
        {
            msgs = msgs.push(loading::view(view));
        }
    } else {
        msgs = msgs.push(
            text("Start a session — type below or pick one from the sidebar.")
                .size(15)
                .color(theme::TEXT_DIM),
        );
    }

    // iced Scrollable — instant 60px wheel steps but rock-solid: the
    // custom spring SmoothScroll kept jittering text under markdown
    // re-render, so stability wins over smoothness here.
    let stream = scrollable(msgs)
        .id(app.scroll.id())
        .on_scroll(Message::Scrolled)
        .direction(theme::scrollable_dir())
        .style(theme::scrollbar())
        .height(Length::Fill);
    col = col.push(stream);

    col = col.push(composer::view(app, is_active));
    col = col.push(statusbar::view(app));

    col.into()
}
