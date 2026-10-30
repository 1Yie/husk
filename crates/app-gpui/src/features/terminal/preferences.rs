use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Native renderer preferences. Unsupported web-only options are intentionally
/// absent, rather than accepted and silently ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalPreferences {
    pub scheme: String,
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub cursor_style: String,
    pub cursor_blink: bool,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            scheme: "husk".into(),
            font_family: String::new(),
            font_size: 14.,
            line_height: 1.2,
            cursor_style: "bar".into(),
            cursor_blink: true,
        }
    }
}

impl Global for TerminalPreferences {}

impl TerminalPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if ![
            "husk",
            "github-light",
            "github-dark",
            "one-dark",
            "solarized-light",
            "dracula",
            "nord",
        ]
        .contains(&self.scheme.as_str())
        {
            return Err("未知终端配色方案".into());
        }
        if !self.font_size.is_finite() || !(10. ..=24.).contains(&self.font_size) {
            return Err("终端字号必须在 10–24 之间".into());
        }
        if !self.line_height.is_finite() || !(1. ..=2.).contains(&self.line_height) {
            return Err("终端行高必须在 1.0–2.0 之间".into());
        }
        if !["block", "bar", "underline"].contains(&self.cursor_style.as_str()) {
            return Err("未知终端光标形态".into());
        }
        if self.font_family.len() > 256 || self.font_family.chars().any(char::is_control) {
            return Err("终端字体名称过长或含控制字符".into());
        }
        Ok(())
    }

    pub fn from_value(value: &Value) -> Result<Self, String> {
        let prefs: Self = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        prefs.validate()?;
        Ok(prefs)
    }

    pub fn line_pixels(&self) -> f32 {
        self.font_size * self.line_height
    }

    pub fn geometry_eq(&self, other: &Self) -> bool {
        self.font_family == other.font_family
            && self.font_size == other.font_size
            && self.line_height == other.line_height
    }

    /// Owner-thread only: invalid/unreadable files are errors, never defaults.
    pub fn load(config_dir: &Path) -> Result<Self, String> {
        let path = config_dir.join("gpui-terminal.json");
        match fs::read(&path) {
            Ok(bytes) => {
                let value = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Self::from_value(&value).map_err(|e| format!("{}: {e}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Reload before patching so a corrupt file cannot be overwritten.
    pub fn save_patch(config_dir: &Path, patch: Value) -> Result<(), String> {
        let patch = patch.as_object().ok_or("终端设置必须是对象")?;
        if patch.is_empty() {
            return Err("终端设置不能为空".into());
        }
        let mut value = serde_json::to_value(Self::load(config_dir)?).map_err(|e| e.to_string())?;
        for (key, field) in patch {
            value[key] = field.clone();
        }
        let prefs = Self::from_value(&value)?;
        fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = config_dir.join(format!(
            ".gpui-terminal-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, config_dir.join("gpui-terminal.json")).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

/// UI-only entry point. No filesystem access and no PTY lifecycle changes.
pub fn apply_preferences(value: &Value, cx: &mut App) -> Result<(), String> {
    let prefs = TerminalPreferences::from_value(value)?;
    if cx.try_global::<TerminalPreferences>() != Some(&prefs) {
        cx.set_global(prefs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_and_validation() {
        let defaults = TerminalPreferences::default();
        assert_eq!(
            TerminalPreferences::from_value(&json!({})).unwrap(),
            defaults
        );
        for patch in [
            json!({"fontSize":9}),
            json!({"fontSize":25}),
            json!({"fontSize":"14"}),
            json!({"lineHeight":0.9}),
            json!({"lineHeight":2.1}),
            json!({"cursorStyle":"circle"}),
            json!({"scheme":"unknown"}),
            json!({"fontFamily":"bad\nfont"}),
            json!({"scrollback":0}),
        ] {
            assert!(TerminalPreferences::from_value(&patch).is_err(), "{patch}");
        }
        assert!(TerminalPreferences::from_value(&json!({"fontSize":24,"lineHeight":2.})).is_ok());
        let mut colors = defaults.clone();
        colors.scheme = "nord".into();
        colors.cursor_blink = false;
        assert!(defaults.geometry_eq(&colors));
    }

    #[test]
    fn persistence_preserves_corrupt_files_and_rejects_invalid_patches() {
        let dir = tempfile::tempdir().unwrap();
        TerminalPreferences::save_patch(dir.path(), json!({"fontSize":18})).unwrap();
        assert_eq!(
            TerminalPreferences::load(dir.path()).unwrap().font_size,
            18.
        );
        let path = dir.path().join("gpui-terminal.json");
        let before = fs::read(&path).unwrap();
        assert!(TerminalPreferences::save_patch(dir.path(), json!({"fontSize":99})).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::write(&path, b"{broken").unwrap();
        assert!(TerminalPreferences::save_patch(dir.path(), json!({"fontSize":16})).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
    }

    #[test]
    fn applying_preferences_preserves_history_selection_modes_and_overrides() {
        use super::super::screen::{TerminalPalette, TerminalScreen};
        use alacritty_terminal::vte::ansi::CursorShape;

        let mut screen = TerminalScreen::new(16, 2);
        screen.feed(b"first\r\nsecond\r\nthird\r\nfourth");
        screen.scroll(1);
        screen.begin_selection(0, 0, false, 3);
        let text = |screen: &TerminalScreen| {
            screen
                .snapshot()
                .cells
                .iter()
                .map(|cell| cell.text.clone())
                .collect::<String>()
        };
        let before = text(&screen);
        let selection = screen.selected_text();
        let modes = screen.mode();
        let prefs = TerminalPreferences {
            font_family: "monospace".into(),
            font_size: 20.,
            line_height: 1.8,
            scheme: "nord".into(),
            cursor_style: "underline".into(),
            cursor_blink: false,
        };
        screen.apply_preferences(&prefs);
        screen.set_palette(TerminalPalette::scheme(&prefs.scheme, true));
        assert_eq!(screen.dimensions(), (16, 2));
        assert_eq!(text(&screen), before);
        assert_eq!(screen.selected_text(), selection);
        assert_eq!(screen.mode(), modes);

        screen.scroll_to_bottom();
        // DECSCUSR and OSC colors remain authoritative after changing defaults.
        screen.feed(b"\x1b[2 q\x1b]10;#123456\x07\x1b]11;#234567\x07\x1b]12;#abcdef\x07\x1b]4;1;#334455\x07\r\n\x1b[31mR\x1b[38;2;4;5;6mT");
        screen.apply_preferences(&TerminalPreferences::default());
        screen.set_palette(TerminalPalette::scheme("dracula", true));
        let snapshot = screen.snapshot();
        assert_eq!(snapshot.foreground, 0x123456);
        assert_eq!(snapshot.background, 0x234567);
        let cursor = snapshot.cursor.unwrap();
        assert_eq!(cursor.shape, CursorShape::Block);
        assert!(!cursor.blinking);
        assert_eq!(cursor.color, 0xabcdef);
        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| cell.text == "R" && cell.foreground == 0x334455)
        );
        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| cell.text == "T" && cell.foreground == 0x040506)
        );
    }

    #[test]
    fn schemes_supply_readable_ansi_colors_and_cursor_defaults() {
        use super::super::screen::{TerminalPalette, TerminalScreen};
        use alacritty_terminal::vte::ansi::CursorShape;
        for id in [
            "husk",
            "github-light",
            "github-dark",
            "one-dark",
            "solarized-light",
            "dracula",
            "nord",
        ] {
            for dark in [false, true] {
                let palette = TerminalPalette::scheme(id, dark);
                assert!(palette.ansi.is_some(), "{id}");
                assert_ne!(palette.foreground, palette.background);
                let mut screen = TerminalScreen::new(8, 2);
                screen.apply_preferences(&TerminalPreferences::default());
                screen.set_palette(palette);
                screen.feed(b"\x1b[31mR");
                let snapshot = screen.snapshot();
                assert_eq!(snapshot.cells[0].foreground, palette.ansi.unwrap()[1]);
                let cursor = snapshot.cursor.unwrap();
                assert_eq!(cursor.shape, CursorShape::Beam);
                assert!(cursor.blinking);
                assert_eq!(cursor.color, palette.cursor.unwrap());
            }
        }
    }
}
