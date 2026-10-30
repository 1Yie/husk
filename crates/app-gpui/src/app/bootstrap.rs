use std::sync::{Arc, Mutex};

use gpui_kit::component::{ThemeMode, TitleBar};
use gpui_kit::*;
use husk_terminal::TerminalState;

use super::shell::{
    BackToWorkspace, NewSession, OpenWorkspace, Quit, Shell, ToggleDock, ToggleSettings,
    ToggleSidebar, ToggleTerminal,
};
use crate::ui::assets::Assets;
use crate::{bridge::HostBridge, platform, ui::theme};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let start_in_settings = std::env::args().any(|arg| arg == "--settings");
    let paths = platform::app_paths();
    platform::logging::init(&paths.data_dir);
    agent_llm::factory::set_client_user_agent(concat!("husk/", env!("CARGO_PKG_VERSION")));
    let bridge = Arc::new(HostBridge::spawn(paths)?);
    let terminals = TerminalState::default();
    let startup_error = Arc::new(Mutex::new(None));
    let window_error = startup_error.clone();
    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        crate::features::terminal::bind_keys(cx);
        if let Err(error) = cx.text_system().add_fonts(vec![
            std::borrow::Cow::Borrowed(include_bytes!("../../assets/fonts/MiSansVF.ttf")),
            std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"
            )),
        ]) {
            tracing::warn!(%error, "Could not load application fonts");
        }
        theme::set_mode(ThemeMode::Light, cx);
        cx.bind_keys([
            KeyBinding::new("escape", BackToWorkspace, Some("HuskSettings")),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-shift-b"
                } else {
                    "ctrl-shift-b"
                },
                ToggleDock,
                Some("Husk && !Terminal"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-q"
                } else {
                    "ctrl-q"
                },
                Quit,
                Some("Husk"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-n"
                } else {
                    "ctrl-n"
                },
                NewSession,
                Some("Husk && !Terminal"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-o"
                } else {
                    "ctrl-o"
                },
                OpenWorkspace,
                Some("Husk && !Terminal"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-b"
                } else {
                    "ctrl-b"
                },
                ToggleSidebar,
                Some("Husk && !Terminal"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-`"
                } else {
                    "ctrl-`"
                },
                ToggleTerminal,
                Some("Husk"),
            ),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "cmd-,"
                } else {
                    "ctrl-,"
                },
                ToggleSettings,
                Some("Husk"),
            ),
        ]);
        // GPUI Component 0.8.1's Linux frame reserves 20px of shadow on
        // each edge. Tauri's undecorated 1280×840 is all visible content.
        let frame = if cfg!(target_os = "linux") { 40. } else { 0. };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(1280. + frame), px(840. + frame)),
                cx,
            )),
            window_min_size: Some(size(px(760.), px(520.))),
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("im.ingstar.husk.gpui".into()),
            ..TitleBar::window_options()
        };
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(bridge.clone(), terminals.clone(), window, cx);
                if start_in_settings {
                    shell.open_settings(cx);
                }
                shell
            })
        });
        match result {
            Ok(_) => cx.activate(true),
            Err(error) => {
                tracing::error!(%error, "Could not open the application window");
                *window_error
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(error.to_string());
                cx.quit();
            }
        }
    });
    if let Some(error) = startup_error
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        return Err(std::io::Error::other(error).into());
    }
    Ok(())
}
