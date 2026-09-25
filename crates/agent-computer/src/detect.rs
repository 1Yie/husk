//! Backend selection at session spawn — mirrors `agent_sandbox::detect_backend`.

use std::sync::Arc;

use crate::none::NoneBackend;
use crate::traits::DesktopBackend;

/// Pick the best implemented desktop backend for this host.
///
/// Linux → X11 when `$DISPLAY` + `xdotool` + a capture tool are all present,
/// otherwise `none` carrying the reason. Detection never guesses: a session
/// that cannot control the desktop says so instead of failing mid-click.
pub fn detect_backend() -> Arc<dyn DesktopBackend> {
    #[cfg(target_os = "linux")]
    fn pick() -> Result<Arc<dyn DesktopBackend>, String> {
        // Wayland is a later stage: wlroots has grim + ydotool, but ydotool
        // needs uinput access and GNOME/KDE expose neither — the honest
        // answer today is the X11 probe (XWayland still answers when
        // `$DISPLAY` is set) and otherwise a refusal that names the gap.
        crate::x11::X11Backend::probe()
            .map(|b| Arc::new(b) as Arc<dyn DesktopBackend>)
            .map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "linux"))]
    fn pick() -> Result<Arc<dyn DesktopBackend>, String> {
        // macOS (`screencapture` + CGEvent via a shim) and Windows (SendInput)
        // land with their own backends; until then, refuse loudly.
        Err(format!(
            "no desktop backend for {} yet (X11 only for now)",
            std::env::consts::OS
        ))
    }

    match pick() {
        Ok(b) => b,
        Err(reason) => Arc::new(NoneBackend::new(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Detection always yields a usable handle — a `none` backend is a valid
    /// answer (it refuses per call), never a panic or an `Option` for every
    /// caller to unwrap.
    #[test]
    fn detect_always_returns_a_backend() {
        let b = detect_backend();
        assert!(matches!(b.id(), "x11" | "none"));
    }
}
