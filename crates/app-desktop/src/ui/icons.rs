//! Keyline Icons (MIT) — vendored `stroke/rounded` 24px SVGs embedded via
//! `svg::Handle::from_memory`, so the single binary ships its own icon set
//! (no font dep, no runtime assets). Each icon is a `currentColor` stroke —
//! `icon(name, size, color)` recolors via `svg::Style.color`.
//!
//! Add an icon: drop `foo.svg` into `assets/icons/` and add a `pub const`
//! below + a `match` arm.

use iced::widget::svg;
use iced::{Color, Element, Length};

/// The icon names we ship — one per vendored file in `assets/icons/`.
#[derive(Debug, Clone, Copy)]
pub enum Icon {
    Message,     // session / chat
    Wrench,      // tool call
    GitBranch,   // git branch
    Check,       // success / allow
    X,           // close / deny / error
    FileText,    // file read
    Search,      // search tool
    Zap,         // fast / steer
    Folder,      // directory
    Terminal,    // bash tool
    Settings,    // config
    Plus,        // new session
    Copy,        // copy button
    Send,        // submit
    Alert,       // warning
    Loader,      // running spinner
    Cpu,         // model/provider
    SquarePen,   // edit / write tool
    Sparkles,    // agent
    Code,        // code
    GitCommit,   // commit
    ChevronDown, // expanded
    ChevronRight,// collapsed
    Minus,       // window minimize
    Maximize,    // window maximize
}

/// The vendored SVG bytes — `include_bytes!` bakes them into the binary.
fn svg_bytes(i: Icon) -> &'static [u8] {
    match i {
        Icon::Message => include_bytes!("../../assets/icons/message-square.svg"),
        Icon::Wrench => include_bytes!("../../assets/icons/wrench.svg"),
        Icon::GitBranch => include_bytes!("../../assets/icons/git-branch.svg"),
        Icon::Check => include_bytes!("../../assets/icons/check.svg"),
        Icon::X => include_bytes!("../../assets/icons/x.svg"),
        Icon::FileText => include_bytes!("../../assets/icons/file-text.svg"),
        Icon::Search => include_bytes!("../../assets/icons/search.svg"),
        Icon::Zap => include_bytes!("../../assets/icons/zap.svg"),
        Icon::Folder => include_bytes!("../../assets/icons/folder.svg"),
        Icon::Terminal => include_bytes!("../../assets/icons/terminal.svg"),
        Icon::Settings => include_bytes!("../../assets/icons/settings.svg"),
        Icon::Plus => include_bytes!("../../assets/icons/plus.svg"),
        Icon::Copy => include_bytes!("../../assets/icons/copy.svg"),
        Icon::Send => include_bytes!("../../assets/icons/send.svg"),
        Icon::Alert => include_bytes!("../../assets/icons/circle-alert.svg"),
        Icon::Loader => include_bytes!("../../assets/icons/loader.svg"),
        Icon::Cpu => include_bytes!("../../assets/icons/cpu.svg"),
        Icon::SquarePen => include_bytes!("../../assets/icons/square-pen.svg"),
        Icon::Sparkles => include_bytes!("../../assets/icons/sparkles.svg"),
        Icon::Code => include_bytes!("../../assets/icons/code.svg"),
        Icon::GitCommit => include_bytes!("../../assets/icons/git-commit-horizontal.svg"),
        Icon::ChevronDown => include_bytes!("../../assets/icons/chevron-down.svg"),
        Icon::ChevronRight => include_bytes!("../../assets/icons/chevron-right.svg"),
        Icon::Minus => include_bytes!("../../assets/icons/minus.svg"),
        Icon::Maximize => include_bytes!("../../assets/icons/maximize.svg"),
    }
}

/// An icon widget — a square `Svg` recolored to `color`, `size` px.
pub fn icon<'a, M>(i: Icon, size: f32, color: Color) -> Element<'a, M>
where
    M: 'a,
{
    svg(svg::Handle::from_memory(svg_bytes(i)))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_t, _st| svg::Style { color: Some(color) })
        .into()
}
