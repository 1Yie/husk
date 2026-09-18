//! Custom titlebar — drag area + app name + min/max/close. Replaces the
//! OS-native frame (`decorations: false`).

use iced::widget::{button, container, mouse_area, row, text, Space};
use iced::{Alignment, Border, Element, Length};

use crate::ui::icons::{icon, Icon};
use crate::ui::message::{Message, WinAction};
use crate::ui::state::App;
use crate::ui::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let btn = |ic: Icon, action: WinAction, danger: bool| {
        let hover_bg = if danger { theme::DIFF_DEL } else { theme::BG_HOVER };
        button(icon(ic, 16.0, theme::TEXT_SECONDARY))
            .on_press(Message::WindowAction(action))
            .padding([6.0, 16.0])
            .style(move |_t, st| button::Style {
                background: match st {
                    button::Status::Hovered => Some(hover_bg.into()),
                    _ => None,
                },
                ..Default::default()
            })
    };
    let title_str = if app.workspace_name.is_empty() {
        "agent-rs".to_string()
    } else {
        format!("agent-rs  —  {}", app.workspace_name)
    };
    // The whole middle band drags — label + a Fill spacer inside the
    // mouse_area so the strip between the title and the buttons is also
    // draggable (a Space outside the area would swallow the drag).
    let drag_area = mouse_area(
        row![
            text(title_str)
                .size(13)
                .color(theme::TEXT_MUTED)
                .font(theme::SANS),
            Space::new().width(Length::Fill),
        ]
        .width(Length::Fill)
        .padding([7.0, 12.0])
        .align_y(Alignment::Center),
    )
    .on_press(Message::WindowAction(WinAction::Drag));
    container(
        row![
            drag_area,
            btn(Icon::Minus, WinAction::Minimize, false),
            btn(Icon::Maximize, WinAction::ToggleMaximize, false),
            btn(Icon::X, WinAction::Close, true),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
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
