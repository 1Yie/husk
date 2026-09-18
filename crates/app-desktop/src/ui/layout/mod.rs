//! Layout — the static window chrome around the chat stream: custom
//! titlebar, session sidebar, and the bottom status bar. `root` composes
//! them; `chat/` owns everything inside the stream column.

pub mod root;
pub mod sidebar;
pub mod statusbar;
pub mod titlebar;
