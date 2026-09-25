//! The desktop-control seam — one trait, one job per method.
//!
//! Coordinates in this API are **screenshot-space**: the model looks at the
//! PNG [`DesktopBackend::screenshot`] wrote, so every input method takes
//! `[x, y]` in *that image's* pixel grid and the backend maps it onto the real
//! display through the capture it last took. A backend that has not captured
//! yet falls back to identity (the caller's coordinates are display
//! coordinates) — which is what a caller that skipped the screenshot means.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Long-edge cap for the PNG handed to the model. 1568 px is the widest
/// common provider bound (Anthropic's stated recommendation); an unscaled 4K
/// capture would cost megabytes of base64 per frame.
pub const MAX_EDGE: u32 = 1568;

/// Which mouse button an action presses. The wire numbers are the backend's
/// business (X11: 1/2/3) — this enum stays protocol-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Scroll wheel direction. Backend maps it to X11 buttons 4/5/6/7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDir {
    Up,
    Down,
    Left,
    Right,
}

/// The mapping between the model's pixel grid and the display's — recorded by
/// the last capture, applied to every input coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    /// Native capture size — the space synthetic input speaks.
    pub display: [u32; 2],
    /// Downscaled size of the PNG the model was shown.
    pub image: [u32; 2],
}

impl Capture {
    /// Display pixels per image pixel, x axis.
    fn sx(&self) -> f64 {
        self.display[0].max(1) as f64 / self.image[0].max(1) as f64
    }

    /// Display pixels per image pixel, y axis.
    fn sy(&self) -> f64 {
        self.display[1].max(1) as f64 / self.image[1].max(1) as f64
    }

    /// Image coordinate → display coordinate, clamped into the display.
    pub fn map_to_display(&self, c: [i32; 2]) -> [i32; 2] {
        [
            (f64::round(c[0] as f64 * self.sx()) as i64).clamp(0, self.display[0].max(1) as i64 - 1)
                as i32,
            (f64::round(c[1] as f64 * self.sy()) as i64).clamp(0, self.display[1].max(1) as i64 - 1)
                as i32,
        ]
    }

    /// Display coordinate → image coordinate (for `cursor_position`, so the
    /// model reads back the same grid it sees), clamped into the image.
    pub fn map_to_image(&self, c: [i32; 2]) -> [i32; 2] {
        [
            (f64::round(c[0] as f64 / self.sx()) as i64).clamp(0, self.image[0].max(1) as i64 - 1)
                as i32,
            (f64::round(c[1] as f64 / self.sy()) as i64).clamp(0, self.image[1].max(1) as i64 - 1)
                as i32,
        ]
    }
}

/// What a capture produced: where the PNG landed plus both resolutions.
#[derive(Debug, Clone)]
pub struct ScreenshotMeta {
    /// The downscaled PNG written for the model.
    pub path: PathBuf,
    /// Native screen resolution — the space input coordinates are mapped into.
    pub display: [u32; 2],
    /// Resolution of `path` — the grid the model reads coordinates off.
    pub image: [u32; 2],
}

impl ScreenshotMeta {
    /// The scale this capture installed on its backend.
    pub fn capture(&self) -> Capture {
        Capture {
            display: self.display,
            image: self.image,
        }
    }
}

/// Downscale `(w, h)` so its long edge is ≤ `max_edge`, preserving aspect.
/// Never upscales; a zero dimension passes through untouched.
pub fn fit_dimensions(w: u32, h: u32, max_edge: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= max_edge || long == 0 {
        return (w, h);
    }
    let scale = max_edge as f64 / long as f64;
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// One desktop backend. Everything a computer-use loop needs, nothing more —
/// `screenshot` feeds the model's eyes, the rest are its hands.
#[async_trait::async_trait]
pub trait DesktopBackend: Send + Sync {
    /// Stable id for telemetry/UI (`"x11"`, `"none"`).
    fn id(&self) -> &'static str;

    /// Capture the screen, write a PNG (long edge ≤ [`MAX_EDGE`]) to `dest`,
    /// and remember the scale for the input methods below.
    async fn screenshot(&self, dest: &Path) -> Result<ScreenshotMeta>;

    /// Press `button` at `at`.
    async fn click(&self, button: MouseButton, at: [i32; 2]) -> Result<()>;

    /// Two left presses at `at`.
    async fn double_click(&self, at: [i32; 2]) -> Result<()>;

    /// Move the pointer to `at` without pressing.
    async fn move_to(&self, at: [i32; 2]) -> Result<()>;

    /// Press left at `from`, move to `to`, release.
    async fn drag(&self, from: [i32; 2], to: [i32; 2]) -> Result<()>;

    /// Type `text` into the focused window.
    async fn type_text(&self, text: &str) -> Result<()>;

    /// Press a key or chord (`"Return"`, `"ctrl+s"`); `hold_ms` keeps it down.
    async fn key(&self, combo: &str, hold_ms: Option<u64>) -> Result<()>;

    /// Scroll `amount` steps in `dir`, optionally after moving to `at`.
    async fn scroll(&self, dir: ScrollDir, amount: i32, at: Option<[i32; 2]>) -> Result<()>;

    /// Pointer position in screenshot coordinates.
    async fn cursor_position(&self) -> Result<[i32; 2]>;

    /// Launch a GUI application by name (`"kwrite"`, `"code"`, `"firefox"`).
    /// The process is detached from the agent — it outlives the tool call.
    /// Runs outside the sandbox on the real display session: launching is
    /// the "I want a GUI on screen" escape hatch the `bash` tool cannot be
    /// (its sandbox would hide `$DISPLAY`/`/tmp/.X11-unix` by construction).
    async fn launch_app(&self, command: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_dimensions_never_upscales() {
        assert_eq!(fit_dimensions(800, 600, MAX_EDGE), (800, 600));
        assert_eq!(fit_dimensions(MAX_EDGE, MAX_EDGE, MAX_EDGE), (MAX_EDGE, MAX_EDGE));
        assert_eq!(fit_dimensions(0, 0, MAX_EDGE), (0, 0));
    }

    #[test]
    fn fit_dimensions_caps_the_long_edge_keeping_aspect() {
        // 4K landscape → 1568×882 (16:9), the size the model is billed for.
        assert_eq!(fit_dimensions(3840, 2160, MAX_EDGE), (1568, 882));
        // Portrait caps by height instead.
        assert_eq!(fit_dimensions(1080, 1920, MAX_EDGE), (882, 1568));
        // Odd aspect ratios still land inside the bound.
        let (w, h) = fit_dimensions(3000, 1000, MAX_EDGE);
        assert_eq!(w, 1568);
        assert!(h <= MAX_EDGE && h > 0);
    }

    #[test]
    fn capture_maps_image_coords_onto_the_display() {
        // 3840×2160 display shown as 1568×882: center of the image is the
        // center of the display, corners clamp into range.
        let cap = Capture {
            display: [3840, 2160],
            image: [1568, 882],
        };
        assert_eq!(cap.map_to_display([784, 441]), [1920, 1080]);
        assert_eq!(cap.map_to_display([0, 0]), [0, 0]);
        // The last image pixel maps to just inside the last display pixel
        // (1567/1568 × 3840 = 3837.55 → 3838) — never past the edge.
        assert_eq!(cap.map_to_display([1567, 881]), [3838, 2158]);
        // Out-of-range model output clamps instead of erroring the call.
        assert_eq!(cap.map_to_display([99_999, -5]), [3839, 0]);
    }

    #[test]
    fn capture_round_trips_and_survives_unscaled_capture() {
        let cap = Capture {
            display: [1920, 1080],
            image: [1920, 1080],
        };
        assert_eq!(cap.map_to_display([960, 540]), [960, 540]);
        assert_eq!(cap.map_to_image([960, 540]), [960, 540]);

        let scaled = Capture {
            display: [3840, 2160],
            image: [1568, 882],
        };
        assert_eq!(scaled.map_to_image(scaled.map_to_display([784, 441])), [784, 441]);
    }

    #[test]
    fn degenerate_display_does_not_panic() {
        let cap = Capture {
            display: [0, 0],
            image: [0, 0],
        };
        assert_eq!(cap.map_to_display([10, 10]), [0, 0]);
        assert_eq!(cap.map_to_image([10, 10]), [0, 0]);
    }
}
