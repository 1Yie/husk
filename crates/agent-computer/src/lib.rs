//! agent-computer — desktop control for computer-use tools.
//!
//! `detect_backend()` picks the best implemented backend (X11 today) and
//! otherwise returns a loud `none` that reports exactly why it cannot act.
//!
//! **This crate is deliberately not part of `agent-sandbox`.** The sandbox's
//! job is to hide the host from a process; a desktop backend's job is the
//! opposite — it must reach the live display server to be useful at all
//! (`--clearenv` + `--tmpfs /tmp` make the X socket unreachable by
//! construction). The defence line for desktop control is the permission
//! gate (`ToolClass::Process` → confirm / `dontAsk` deny), not filesystem
//! isolation: a process that can synthesize input for the whole session is
//! already outside what an fs sandbox can protect.
//!
//! Coordinates at this API surface are **screenshot-space** — see
//! [`traits::Capture`].

pub mod detect;
pub mod none;
pub mod traits;
pub mod x11;

pub use detect::detect_backend;
pub use none::NoneBackend;
pub use traits::{
    fit_dimensions, Capture, DesktopBackend, MouseButton, ScrollDir, ScreenshotMeta, MAX_EDGE,
};
