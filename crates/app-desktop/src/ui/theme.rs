//! CodexTheme → iced consts. Industrial cold-grey, near-black. No gradients,
//! no shadows — 1px hairlines separate zones. Accent scarcity: purple =
//! executing only, green/red = diff semantics only.

#![allow(dead_code)] // palette is the full CodexTheme set; some tokens are
                     // reserved for surfaces not yet built (hover, risk banner,
                     // focused borders) — keeping them canonical.

use iced::{Color, Font};
use iced::font::Family;

pub const BG_WORKSPACE: Color     = Color::from_rgb8(0x09, 0x09, 0x0b);
pub const BG_PANEL: Color         = Color::from_rgb8(0x12, 0x12, 0x15);
pub const BG_CARD: Color          = Color::from_rgb8(0x18, 0x18, 0x1b);
pub const BG_HOVER: Color         = Color::from_rgb8(0x27, 0x27, 0x2a);
pub const BG_ACTIVE: Color        = Color::from_rgb8(0x3f, 0x3f, 0x46);

pub const BORDER_HAIRLINE: Color  = Color::from_rgb8(0x27, 0x27, 0x2a);
pub const BORDER_FOCUS: Color     = Color::from_rgb8(0x52, 0x52, 0x5b);

pub const TEXT_WHITE: Color       = Color::from_rgb8(0xfa, 0xfa, 0xfa);
pub const TEXT_SECONDARY: Color   = Color::from_rgb8(0xa1, 0xa1, 0xaa);
pub const TEXT_MUTED: Color       = Color::from_rgb8(0x52, 0x52, 0x5b);
pub const TEXT_DIM: Color         = Color::from_rgb8(0x3f, 0x3f, 0x46);

pub const ACCENT: Color           = Color::from_rgb8(0xe4, 0xe4, 0xe7);
pub const DIFF_ADD: Color         = Color::from_rgb8(0x22, 0xc5, 0x5e);
pub const DIFF_ADD_SURFACE: Color = Color::from_rgb8(0x05, 0x2e, 0x16);
pub const DIFF_DEL: Color         = Color::from_rgb8(0xef, 0x44, 0x44);
pub const DIFF_DEL_SURFACE: Color = Color::from_rgb8(0x45, 0x0a, 0x0a);
pub const STATUS_RUNNING: Color   = Color::from_rgb8(0xa8, 0x55, 0xf7);
pub const WARN: Color             = Color::from_rgb8(0xf5, 0x9e, 0x0b);
pub const ERROR: Color            = Color::from_rgb8(0xef, 0x44, 0x44);

pub const MONO: Font = Font { family: Family::Name("JetBrains Mono"), ..Font::MONOSPACE };
pub const SANS: Font = Font { family: Family::Name("Inter"), ..Font::DEFAULT };

// Corner radii — the modern pass trades the TUI's uniform 4px hairline
// chips for softer, layered rounding: cards/pills bigger than inline bits.
pub const RADIUS_SM: f32 = 6.0;
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_LG: f32 = 14.0;
pub const RADIUS_PILL: f32 = 999.0;

// ---------------------------------------------------------------------------
// Style helpers — the shared surface/button/text-input looks the UI reuses.
// Returning `impl Fn(&Theme) -> Style` so they drop straight into
// `.style(theme::card)` without a closure at every call site.
// ---------------------------------------------------------------------------

use iced::widget::{button, container, text_input};
use iced::Border;

/// Hairline border used on every elevated surface.
fn hairline() -> Border {
    Border {
        width: 1.0,
        color: BORDER_HAIRLINE,
        ..Default::default()
    }
}

/// Flat workspace surface — no border (the main stream column background).
pub fn surface() -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(BG_WORKSPACE.into()),
        ..Default::default()
    }
}

/// Panel surface — sidebar/statusbar background + a hairline edge.
pub fn panel() -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(BG_PANEL.into()),
        border: hairline(),
        ..Default::default()
    }
}

/// Card — a softly-rounded surface for tool capsules, session rows, and
/// inline panels. RADIUS_MD replaces the old 4px TUI chip.
pub fn card() -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(BG_CARD.into()),
        border: hairline().rounded(RADIUS_MD),
        ..Default::default()
    }
}

/// A chat bubble — the modern message surface. User bubbles tint with the
/// accent surface; agent bubbles use the panel card. Big radius, no
/// hairline (the fill alone separates it from the stream).
pub fn bubble(tinted: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(if tinted { BG_HOVER } else { BG_CARD }.into()),
        border: Border::default().rounded(RADIUS_LG),
        ..Default::default()
    }
}

/// Pill button — a fully-rounded action chip (send/allow/deny).
pub fn pill(bg: Color, fg: Color) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, _st| button::Style {
        background: Some(bg.into()),
        border: Border::default().rounded(RADIUS_PILL),
        text_color: fg,
        ..Default::default()
    }
}

/// Card variant with an accent border — the awaiting-confirmation capsule.
pub fn card_accent() -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(BG_CARD.into()),
        border: Border {
            width: 1.0,
            color: ACCENT,
            ..Default::default()
        }
        .rounded(4.0),
        ..Default::default()
    }
}

/// Invisible button — used for toggle rows (reasoning header, step capsule)
/// where the whole area is clickable but has no chrome.
pub fn ghost_button() -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, _st| button::Style {
        background: None,
        ..Default::default()
    }
}

/// Composer text input — a soft rounded-rect field (RADIUS_LG), no harsh
/// hairline: the card fill carries the shape.
pub fn composer_input() -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_t, _st| text_input::Style {
        background: BG_CARD.into(),
        border: hairline().rounded(RADIUS_LG),
        icon: TEXT_MUTED,
        placeholder: TEXT_DIM,
        value: TEXT_WHITE,
        selection: BG_ACTIVE,
    }
}

/// Slim modern scrollbar — a thin rounded thumb on a transparent rail, no
/// gutter background. Replaces the chunky default rail. Use
/// `theme::scrollable_dir()` for the direction (it carries the rail/thumb
/// metrics, which aren't part of the Style) + `theme::scrollbar()`.
use iced::widget::scrollable;

/// Vertical scrollbar metrics — rail 8px, thumb 6px. Apply via
/// `.direction(theme::scrollable_dir())` on any vertical scrollable.
pub fn scrollable_dir() -> scrollable::Direction {
    scrollable::Direction::Vertical(
        scrollable::Scrollbar::new().width(8).scroller_width(6),
    )
}

/// Horizontal scrollbar metrics for wide code/diff panes.
pub fn scrollable_dir_h() -> scrollable::Direction {
    scrollable::Direction::Horizontal(
        scrollable::Scrollbar::new().width(8).scroller_width(6),
    )
}

pub fn scrollbar() -> impl Fn(&iced::Theme, scrollable::Status) -> scrollable::Style {
    move |_t, _st| {
        // Thumb must contrast the dark panel — BG_HOVER (0x27) was nearly
        // invisible against BG_PANEL/BG_CARD, so use the lighter BORDER_FOCUS
        // (0x52) — visible without shouting.
        let rail = scrollable::Rail {
            background: None, // transparent track — no slab beside content
            border: Border::default().rounded(RADIUS_SM),
            scroller: scrollable::Scroller {
                background: BORDER_FOCUS.into(),
                border: Border::default().rounded(RADIUS_SM),
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            auto_scroll: scrollable::AutoScroll {
                background: BG_CARD.into(),
                border: hairline().rounded(RADIUS_SM),
                shadow: Default::default(),
                icon: TEXT_SECONDARY,
            },
        }
    }
}
