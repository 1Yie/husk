//! app-desktop — iced GUI shell.
//!
//! Elm architecture: `App::update` reduces `Message`s into state changes,
//! `App::view` derives the widget tree each frame. The kernel runs on a
//! dedicated tokio thread; `UiEvent`s cross a `std::sync::mpsc` drained by a
//! 60Hz `Subscription` tick. `--mock` boots with static demo state.

mod bridge;
mod ui;

use iced::Task;
use ui::{App, Message};

fn main() -> iced::Result {
    tracing_subscriber_init();

    iced::application(boot, App::update, App::view)
        .subscription(App::subscription)
        .theme(theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1080.0, 720.0),
            min_size: Some(iced::Size::new(720.0, 480.0)),
            decorations: false, // custom titlebar replaces the native frame
            ..Default::default()
        })
        .title(title)
        .run()
}

fn theme(_app: &App) -> iced::Theme {
    iced::Theme::Dark
}

fn title(app: &App) -> String {
    if app.workspace_name.is_empty() {
        "agent-rs".into()
    } else {
        format!("agent-rs — {}", app.workspace_name)
    }
}

/// iced boot — runs once, returns `(App, Task)`. `--mock` seeds demo state;
/// otherwise boots the SessionManager (resumes the most recent session or
/// starts fresh in the target workspace).
fn boot() -> (App, Task<Message>) {
    let mock = std::env::args().any(|a| a == "--mock");
    let cli_dir = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(std::path::PathBuf::from);
    let app = if mock {
        App::demo()
    } else {
        let (_b, loud) = agent_sandbox::detect_backend();
        App::boot(bridge::SessionManager::spawn_at(cli_dir), loud)
    };
    // Grab the window Id for the custom titlebar's drag/min/max/close.
    (app, iced::window::latest().map(Message::WindowReady))
}

fn tracing_subscriber_init() {
    if std::env::var("RUST_LOG").is_ok() {
        eprintln!("[app-desktop] logging enabled");
    }
}
