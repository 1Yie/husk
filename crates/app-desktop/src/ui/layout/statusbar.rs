//! Bottom status bar — agent state + permission mode + sandbox warning on
//! the left; context usage (tokens as `K/K` + a % bar), git branch, files
//! changed, and `provider · model` on the right.

use iced::widget::{container, row, text, Space};
use iced::{Alignment, Border, Element, Length};

use crate::ui::icons::{icon, Icon};
use crate::ui::message::Message;
use crate::ui::state::App;
use crate::ui::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let fmt_k = |n: u32| {
        if n >= 1000 {
            format!("{}K", n / 1000)
        } else {
            n.to_string()
        }
    };
    let used = app.stats.tokens_used;
    let window = app.stats.context_window.max(1);
    let pct = (used as f32 / window as f32 * 100.0).min(999.0) as u32;
    let ctx = format!("{}/{} · {}%", fmt_k(used), fmt_k(window), pct);
    // Context-usage color — cool under 50%, warm 50–80%, hot above.
    let ctx_color = if pct >= 80 {
        theme::ERROR
    } else if pct >= 50 {
        theme::WARN
    } else {
        theme::TEXT_MUTED
    };

    let model = app.stats.active_model.clone();
    let mode = app.stats.permission_mode.clone();
    // Sandbox unsafe → warn badge on the mode; otherwise the mode label.
    let (mode_label, mode_color) = if app.sandbox_unsafe {
        ("⚠ unsandboxed".to_string(), theme::WARN)
    } else {
        (mode, theme::TEXT_MUTED)
    };
    let files = format!("{} files", app.stats.files_changed);
    let branch = app.stats.git_branch.clone();
    let state = app.stats.agent_state.clone();

    container(
        row![
            icon(Icon::Cpu, 13.0, theme::TEXT_SECONDARY),
            text(model).size(12).color(theme::TEXT_SECONDARY).font(theme::SANS),
            text(mode_label).size(11).color(mode_color).font(theme::SANS),
            Space::new().width(Length::Fill),
            text(state).size(11).color(theme::STATUS_RUNNING).font(theme::SANS),
            icon(Icon::GitBranch, 12.0, theme::TEXT_MUTED),
            text(branch).size(11).color(theme::TEXT_MUTED).font(theme::SANS),
            text(files).size(11).color(theme::TEXT_MUTED).font(theme::SANS),
            text(ctx).size(11).color(ctx_color).font(theme::SANS),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([9.0, 14.0])
    .style(|_t| container::Style {
        background: Some(theme::BG_PANEL.into()),
        border: Border {
            width: 1.0,
            color: theme::BORDER_HAIRLINE,
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
