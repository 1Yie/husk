//! Native PTY terminal surface.

mod deck;
mod panel;
pub mod preferences;
pub mod screen;

pub(crate) use deck::bind_keys;
pub use deck::{TerminalDeck, TerminalDeckEvent};
pub use panel::TerminalPanel;
pub use preferences::{TerminalPreferences, apply_preferences};

#[cfg(test)]
mod tests;
