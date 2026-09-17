//! app-desktop — iced GUI shell.
//!
//! Elm architecture: `App::update` reduces `Message`s into state changes,
//! `App::view` derives the widget tree each frame. The kernel runs on a
//! dedicated tokio thread; `UiEvent`s cross a `std::sync::mpsc` drained by a
//! 60Hz `Subscription` tick. `--mock` boots with static demo state.

mod bridge;
mod throttler;
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
            ..Default::default()
        })
        .title(title)
        .run()
}

fn theme(_app: &App) -> iced::Theme {
    iced::Theme::Dark
}

fn title(_app: &App) -> String {
    "agent-rs".into()
}

/// iced boot — runs once, returns `(App, Task)`. We capture `--mock` via a
/// static flag (boot is a plain fn, can't capture args).
fn boot() -> (App, Task<Message>) {
    let mock = std::env::args().any(|a| a == "--mock");
    let app = if mock {
        App::demo()
    } else {
        let h = bridge::spawn_kernel();
        App::boot(
            h.cmd_tx,
            h.decision,
            h.steer_tx,
            h.event_rx,
            h.stats,
            h.sandbox_unsafe,
        )
    };
    (app, Task::none())
}

fn tracing_subscriber_init() {
    if std::env::var("RUST_LOG").is_ok() {
        eprintln!("[app-desktop] logging enabled");
    }
}
