//! `App::view` — the Elm root. Thin shell: the whole widget tree is built
//! by `layout::root` (titlebar + sidebar + chat stream column); domain
//! renderers live under `chat/` and `layout/`.

use iced::Element;

use super::layout;
use super::message::Message;
use super::state::App;

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        layout::root::view(self)
    }
}
