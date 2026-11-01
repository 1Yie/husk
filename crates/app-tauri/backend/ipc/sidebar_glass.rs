//! Window-local native backdrop for the React application shell.

#[cfg(target_os = "macos")]
mod macos;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Surface {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub background: [f64; 3],
    pub dark: bool,
}

impl Surface {
    fn validate(&self) -> Result<(), String> {
        if ![
            self.x,
            self.y,
            self.width,
            self.height,
            self.viewport_width,
            self.viewport_height,
        ]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 100_000.0)
            || self.width < 0.0
            || self.height < 0.0
            || self.viewport_width <= 0.0
            || self.viewport_height <= 0.0
            || !self
                .background
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return Err("invalid sidebar glass surface".into());
        }
        Ok(())
    }

    /// CSS viewport coordinates become WebView-local AppKit points, never pixels.
    fn clipped(&self, width: f64, height: f64, flipped: bool) -> [f64; 4] {
        let sx = width / self.viewport_width;
        let sy = height / self.viewport_height;
        let x = (self.x * sx).clamp(0.0, width);
        let top = (self.y * sy).clamp(0.0, height);
        let right = ((self.x + self.width) * sx).clamp(x, width);
        let bottom = ((self.y + self.height) * sy).clamp(top, height);
        [
            x,
            if flipped { top } else { height - bottom },
            right - x,
            bottom - top,
        ]
    }
}

#[tauri::command]
pub async fn sidebar_glass_update(
    window: tauri::WebviewWindow,
    revision: f64,
    surface: Option<Surface>,
) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("sidebar glass is restricted to the main window".into());
    }
    if !revision.is_finite() || !(0.0..=1e16).contains(&revision) {
        return Err("invalid sidebar glass revision".into());
    }
    if let Some(surface) = &surface {
        surface.validate()?;
    }
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = tokio::sync::oneshot::channel();
        window
            .with_webview(move |webview| {
                // SAFETY: Tauri lends the WKWebView on the main thread for this callback.
                let result = unsafe { macos::update(webview.inner().cast(), revision, surface) };
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        receive.await.map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> Surface {
        Surface {
            x: 0.0,
            y: 20.0,
            width: 240.0,
            height: 580.0,
            viewport_width: 1000.0,
            viewport_height: 600.0,
            background: [1.0; 3],
            dark: false,
        }
    }

    #[test]
    fn geometry_uses_points_and_clips_to_the_webview() {
        let mut s = surface();
        assert_eq!(s.clipped(1000.0, 600.0, false), [0.0, 0.0, 240.0, 580.0]);
        assert_eq!(s.clipped(500.0, 300.0, true), [0.0, 10.0, 120.0, 290.0]);
        s.x = -40.0;
        s.height = 1000.0;
        assert_eq!(s.clipped(1000.0, 600.0, false), [0.0, 0.0, 200.0, 580.0]);
        s.width = 0.0;
        assert_eq!(s.clipped(1000.0, 600.0, false)[2], 0.0);
    }

    #[test]
    fn rejects_nonfinite_geometry_and_out_of_range_colours() {
        let mut s = surface();
        s.x = f64::NAN;
        assert!(s.validate().is_err());
        s.x = 0.0;
        s.background[1] = 1.01;
        assert!(s.validate().is_err());
        s.background[1] = 0.5;
        s.viewport_width = 0.0;
        assert!(s.validate().is_err());
    }
}
