//! Sidebar — workspace-first layout (Codex VIP style).
//! Top: Active Workspace Card (project name, git branch, Open button) + New Session button.
//! Middle: Scrollable list of sessions belonging to this workspace.
//! Bottom: Recent Workspaces quick-switcher.

use iced::widget::{button, column, container, row, scrollable, text, Column, Space};
use iced::{Alignment, Border, Element, Length};

use crate::ui::icons::{icon, Icon};
use crate::ui::message::Message;
use crate::ui::state::{App, SessionRow};
use crate::ui::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut header = Column::new().spacing(0).padding(8);

    // App brand
    header = header.push(
        row![
            text("agent-rs")
                .size(13)
                .color(theme::TEXT_MUTED)
                .font(theme::SANS),
        ]
        .align_y(Alignment::Center),
    );
    header = header.push(Space::new().height(Length::Fixed(6.0)));

    // Active Workspace Card
    let ws_branch = if !app.stats.git_branch.is_empty() {
        Element::from(
            row![
                icon(Icon::GitBranch, 11.0, theme::TEXT_MUTED),
                text(&app.stats.git_branch)
                    .size(11)
                    .color(theme::TEXT_MUTED)
                    .font(theme::SANS),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
    } else {
        let display_path = app
            .workspace_root
            .file_name()
            .map(|_| app.workspace_root.to_string_lossy().to_string())
            .unwrap_or_default();
        Element::from(
            text(display_path)
                .size(10)
                .color(theme::TEXT_MUTED)
                .font(theme::SANS),
        )
    };

    let ws_card = container(
        column![
            row![
                icon(Icon::Folder, 14.0, theme::TEXT_WHITE),
                text(&app.workspace_name)
                    .size(13)
                    .color(theme::TEXT_WHITE)
                    .font(theme::SANS),
                Space::new().width(Length::Fill),
                button(
                    row![
                        icon(Icon::Folder, 11.0, theme::TEXT_SECONDARY),
                        text("Open").size(11).color(theme::TEXT_SECONDARY),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::OpenWorkspace)
                .padding([3, 7])
                .style(|_t, st| button::Style {
                    background: match st {
                        button::Status::Hovered => Some(theme::BG_HOVER.into()),
                        _ => Some(theme::BG_PANEL.into()),
                    },
                    border: Border::default().rounded(theme::RADIUS_SM),
                    ..Default::default()
                }),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            ws_branch,
        ]
        .spacing(4),
    )
    .padding(8)
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(theme::BG_CARD.into()),
        border: Border {
            width: 1.0,
            color: theme::BORDER_HAIRLINE,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    });

    header = header.push(ws_card);
    header = header.push(Space::new().height(Length::Fixed(6.0)));

    // New session in current workspace
    header = header.push(
        button(
            row![
                icon(Icon::Plus, 13.0, theme::TEXT_WHITE),
                text(" New Session").size(12).color(theme::TEXT_WHITE),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .on_press(Message::NewSession)
        .width(Length::Fill)
        .padding(8)
        .style(|_t, st| button::Style {
            background: match st {
                button::Status::Hovered => Some(theme::BG_HOVER.into()),
                _ => Some(theme::BG_CARD.into()),
            },
            border: Border::default().rounded(theme::RADIUS_SM),
            ..Default::default()
        }),
    );
    header = header.push(Space::new().height(Length::Fixed(10.0)));

    // Sessions label + count
    header = header.push(
        row![
            text("SESSIONS").size(11).color(theme::TEXT_MUTED),
            Space::new().width(Length::Fill),
            text(format!("{}", app.sessions.len()))
                .size(11)
                .color(theme::TEXT_MUTED),
        ]
        .align_y(Alignment::Center),
    );
    header = header.push(Space::new().height(Length::Fixed(4.0)));

    // Scrollable session list
    let mut list = Column::new().spacing(2).padding([0, 8]);
    for s in &app.sessions {
        list = list.push(session_row(s, app.tick_count));
    }

    // Recent workspaces footer
    let other_recents: Vec<_> = app
        .recent_workspaces
        .iter()
        .filter(|w| w.path != app.workspace_root)
        .take(4)
        .collect();

    let mut footer = Column::new().spacing(2).padding(8);
    if !other_recents.is_empty() {
        footer = footer.push(
            text("RECENT WORKSPACES")
                .size(10)
                .color(theme::TEXT_MUTED),
        );
        footer = footer.push(Space::new().height(Length::Fixed(2.0)));
        for w in other_recents {
            let path_clone = w.path.clone();
            footer = footer.push(
                button(
                    row![
                        icon(Icon::Folder, 11.0, theme::TEXT_MUTED),
                        text(&w.name).size(12).color(theme::TEXT_SECONDARY),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::SwitchWorkspace(path_clone))
                .width(Length::Fill)
                .padding([4, 6])
                .style(|_t, st| button::Style {
                    background: match st {
                        button::Status::Hovered => Some(theme::BG_HOVER.into()),
                        _ => None,
                    },
                    border: Border::default().rounded(theme::RADIUS_SM),
                    ..Default::default()
                }),
            );
        }
    }

    let body = column![
        header,
        scrollable(list)
            .direction(theme::scrollable_dir())
            .style(theme::scrollbar())
            .height(Length::Fill),
        footer,
    ]
    .height(Length::Fill);

    container(body)
        .width(Length::Fixed(230.0))
        .height(Length::Fill)
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

fn session_row<'a>(s: &'a SessionRow, tick: u64) -> Element<'a, Message> {
    let (bg, fg) = if s.active {
        (theme::BG_ACTIVE, theme::TEXT_WHITE)
    } else {
        (theme::BG_PANEL, theme::TEXT_SECONDARY)
    };

    button(
        row![
            text(&s.title).size(13).color(fg),
            Space::new().width(Length::Fill),
            if s.running {
                Element::from(
                    text(crate::ui::animation::spinner_at(tick).to_string())
                        .size(12)
                        .color(theme::STATUS_RUNNING)
                        .font(theme::MONO),
                )
            } else {
                Element::from(Space::new().width(Length::Fixed(12.0)))
            },
        ]
        .align_y(Alignment::Center),
    )
    .on_press(Message::SelectSession(s.id))
    .width(Length::Fill)
    .padding(8)
    .style(move |_t, _st| button::Style {
        background: Some(bg.into()),
        border: Border::default().rounded(theme::RADIUS_SM),
        text_color: fg,
        ..Default::default()
    })
    .into()
}
