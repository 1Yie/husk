use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, WindowAppearance, px, rgb};
use husk_host::prefs::AppearanceSettings;

pub fn effective_mode(preference: &str, appearance: WindowAppearance) -> ThemeMode {
    match preference {
        "light" => ThemeMode::Light,
        "dark" => ThemeMode::Dark,
        _ if matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ) =>
        {
            ThemeMode::Dark
        }
        _ => ThemeMode::Light,
    }
}

pub fn panel_background(cx: &App) -> Hsla {
    let theme = Theme::global(cx);
    if theme.is_dark() {
        theme.sidebar
    } else {
        rgb(0xfbfbfb).into()
    }
}

pub fn set_mode(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = "MiSans VF".into();
        theme.mono_font_family = "JetBrainsMono Nerd Font Mono".into();
        theme.font_size = px(13.);
        theme.radius = px(10.);
        theme.radius_lg = px(22.);
        theme.motion.duration_normal = super::motion::DIALOG_DURATION;
        if mode == ThemeMode::Dark {
            theme.colors.background = rgb(0x16161a).into();
            theme.colors.foreground = rgb(0xe4e4e7).into();
            theme.colors.sidebar = rgb(0x1b1b21).into();
            theme.colors.sidebar_foreground = rgb(0xe4e4e7).into();
            theme.colors.border = rgb(0x34343d).into();
            theme.colors.muted = rgb(0x1e1e24).into();
            theme.colors.muted_foreground = rgb(0x9a9aa4).into();
            theme.colors.secondary = rgb(0x2b2b32).into();
            theme.colors.popover = rgb(0x2b2b32).into();
            theme.colors.input = rgb(0x26262d).into();
        } else {
            theme.colors.background = rgb(0xffffff).into();
            theme.colors.foreground = rgb(0x1a1c1f).into();
            theme.colors.sidebar = rgb(0xf3f3f5).into();
            theme.colors.sidebar_foreground = rgb(0x3f3f46).into();
            theme.colors.border = rgb(0xe4e4e7).into();
            theme.colors.muted = rgb(0xf4f4f5).into();
            theme.colors.muted_foreground = rgb(0x71717a).into();
            theme.colors.secondary = rgb(0xe5e5e7).into();
            theme.colors.popover = rgb(0xffffff).into();
            theme.colors.input = rgb(0xffffff).into();
        }
        theme.colors.popover_foreground = theme.colors.foreground;
        theme.colors.secondary_foreground = theme.colors.foreground;
        theme.colors.secondary_hover = theme.colors.muted;
        theme.colors.secondary_active = theme.colors.secondary;
        theme.colors.primary = theme.colors.foreground;
        theme.colors.primary_foreground = theme.colors.background;
        theme.colors.primary_hover = theme.colors.foreground;
        theme.colors.primary_active = theme.colors.foreground;
    });
}

pub fn apply_appearance(mode: ThemeMode, appearance: &AppearanceSettings, cx: &mut App) {
    set_mode(mode, cx);
    let fonts = cx.text_system().all_font_names();
    let ui_fallback = if fonts.iter().any(|name| name.eq_ignore_ascii_case("MiSans")) {
        "MiSans"
    } else {
        "MiSans VF"
    };
    let ui_font = resolve_font(&appearance.ui_font, &fonts, ui_fallback);
    let code_font = resolve_font(
        &appearance.code_font,
        &fonts,
        "JetBrainsMono Nerd Font Mono",
    );
    Theme::update(cx, |theme| {
        theme.font_family = ui_font.into();
        theme.mono_font_family = code_font.into();
        let dark = mode == ThemeMode::Dark;
        let background = if dark {
            appearance.dark_background.as_deref()
        } else {
            Some(appearance.background.as_str())
        };
        let foreground = if dark {
            appearance.dark_foreground.as_deref()
        } else {
            Some(appearance.foreground.as_str())
        };
        if let Some(background) = background.and_then(parse_color) {
            theme.colors.background = background;
        }
        if let Some(foreground) = foreground.and_then(parse_color) {
            theme.colors.foreground = foreground;
        }
        let accent = if dark {
            appearance
                .dark_accent
                .as_deref()
                .unwrap_or(&appearance.accent)
        } else {
            &appearance.accent
        };
        if let Some(accent) = parse_color(accent) {
            // Husk's primary actions stay neutral; accent belongs to focus
            // and selection, not a replacement colour for the send button.
            theme.colors.ring = accent;
            theme.colors.selection = Hsla { a: 0.24, ..accent };
        }
        theme.colors.primary = theme.colors.foreground;
        theme.colors.primary_foreground = theme.colors.background;
    });
}

fn parse_color(value: &str) -> Option<Hsla> {
    let value = value.strip_prefix('#')?;
    let number = match value.len() {
        3 => {
            let digits = u32::from_str_radix(value, 16).ok()?;
            ((digits & 0xf00) << 8 | (digits & 0xf0) << 4 | digits & 0xf) * 17
        }
        6 => u32::from_str_radix(value, 16).ok()?,
        _ => return None,
    };
    Some(rgb(number).into())
}

fn resolve_font(list: &str, available: &[String], fallback: &str) -> String {
    list.split(',')
        .map(|name| name.trim().trim_matches(['"', '\'']))
        .find_map(|requested| {
            available
                .iter()
                .find(|font| font.eq_ignore_ascii_case(requested))
        })
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_wins_and_system_tracks_window_appearance() {
        assert_eq!(
            effective_mode("light", WindowAppearance::Dark),
            ThemeMode::Light
        );
        assert_eq!(
            effective_mode("dark", WindowAppearance::Light),
            ThemeMode::Dark
        );
        assert_eq!(
            effective_mode("system", WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
        assert_eq!(
            effective_mode("system", WindowAppearance::VibrantLight),
            ThemeMode::Light
        );
        assert_eq!(
            effective_mode("unknown", WindowAppearance::Dark),
            ThemeMode::Dark
        );
    }

    #[test]
    fn stored_colors_accept_hex_without_treating_css_as_a_native_color() {
        assert_eq!(parse_color("#abc"), parse_color("#aabbcc"));
        assert_eq!(parse_color("#FFFFFF"), Some(rgb(0xffffff).into()));
        assert_eq!(parse_color("url(file:///etc/passwd)"), None);
        assert_eq!(parse_color("#gggggg"), None);
    }

    #[test]
    fn font_stacks_choose_an_installed_family_or_the_bundled_fallback() {
        let fonts = vec!["MiSans VF".into(), "Iosevka Nerd Font Mono".into()];
        assert_eq!(
            resolve_font("Missing, \"MiSans VF\", sans-serif", &fonts, "fallback"),
            "MiSans VF"
        );
        assert_eq!(
            resolve_font("'iosevka nerd font mono'", &fonts, "fallback"),
            "Iosevka Nerd Font Mono"
        );
        assert_eq!(resolve_font("nonexistent", &fonts, "fallback"), "fallback");
    }
}
