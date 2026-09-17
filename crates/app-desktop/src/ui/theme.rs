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
