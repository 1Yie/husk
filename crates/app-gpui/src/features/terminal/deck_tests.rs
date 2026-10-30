use super::{MAX_PANES, PaneTree, TerminalDeck, bind_keys};
use gpui_kit::{
    App, AppContext, Context, Entity, IntoElement, Render, TestAppContext, Window, WindowBounds,
    WindowOptions,
    component::{
        input::{Textarea, TextareaState},
        v_flex,
    },
    point,
    prelude::*,
    px, size,
    test::TestWindowExt,
};
use husk_terminal::TerminalState;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct Harness {
    deck: Entity<TerminalDeck>,
    other: Entity<TextareaState>,
    show: bool,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(Textarea::new(&self.other).h(px(40.)).aria_label("其他输入"))
            .child(
                gpui_kit::div()
                    .flex_1()
                    .min_h_0()
                    .when(self.show, |this| this.child(self.deck.clone())),
            )
    }
}

fn ready(tree: &PaneTree, cx: &App) -> bool {
    match tree {
        PaneTree::Leaf(pane) => {
            let panel = pane.panel.read(cx);
            if let Some(status) = panel.status() {
                assert_eq!(status, "Starting terminal…", "pane {}: {status}", pane.id);
            }
            panel.title() == Some("READY")
        }
        PaneTree::Split { first, second, .. } => ready(first, cx) && ready(second, cx),
    }
}

#[gpui_kit::test]
fn split_resizing_and_scoped_shortcuts_preserve_state_and_reap_ptys(cx: &mut TestAppContext) {
    const NAME: &str = "features::terminal::deck::ui_tests::split_resizing_and_scoped_shortcuts_preserve_state_and_reap_ptys";
    if std::env::var("HUSK_GPUI_TERMINAL_UI_TEST").as_deref() != Ok(NAME) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let shell = dir.path().join("test-shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\nstty raw -echo\nprintf '\\033]0;READY\\007'\nexec cat >> \"$HUSK_GPUI_TERMINAL_INPUT\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .current_dir(dir.path())
            .env("HUSK_GPUI_TERMINAL_UI_TEST", NAME)
            .env("HUSK_GPUI_TERMINAL_INPUT", dir.path().join("input"))
            .env("HOME", home)
            .env("SHELL", shell)
            .env("PATH", "/usr/bin:/bin")
            .env("HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS", "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated terminal UI test failed");
        return;
    }

    let cwd = std::env::current_dir().unwrap();
    let input = PathBuf::from(std::env::var_os("HUSK_GPUI_TERMINAL_INPUT").unwrap());
    let registry = TerminalState::default();
    let terminal_state = registry.clone();
    // PTY reader/reaper threads are real OS I/O, not deterministic GPUI tasks.
    cx.background_executor.allow_parking();
    cx.update(gpui_kit::init);
    cx.update(bind_keys);
    let (handle, root) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1000.), px(600.)), cx)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let deck = cx.new(|cx| {
                    TerminalDeck::new(registry, "terminal-ui-test".into(), cwd, window, cx)
                });
                let other = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 1));
                cx.new(|_| Harness {
                    deck,
                    other,
                    show: true,
                })
            },
        )
        .unwrap()
    });
    let deck = cx.update(|cx| root.read(cx).deck.clone());
    let key = |linux: &'static str, macos: &'static str| {
        if cfg!(target_os = "macos") {
            macos
        } else {
            linux
        }
    };
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        deck.update(cx, |deck, cx| deck.focus(window, cx));
        window.render_frame(cx);
        window.press(key("ctrl-shift-d", "cmd-d"), cx);
        assert_eq!(deck.read(cx).count(), 2);
    })
    .unwrap();
    cx.run_until_parked();
    let split_state = cx.update(|cx| {
        let PaneTree::Split { state, .. } = &deck.read(cx).tabs[0].root else {
            panic!("expected column split");
        };
        state.clone()
    });
    let before = cx.update(|cx| split_state.read(cx).sizes().clone());
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let divider = window.find(("resizable-handle", 0usize)).bounds().center();
        window.drag(divider, point(divider.x + px(120.), divider.y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let resized = cx.update(|cx| split_state.read(cx).sizes().clone());
    assert!(
        resized[0] > before[0] + px(60.),
        "divider did not resize: {before:?} -> {resized:?}"
    );
    assert!(resized[1] < before[1]);

    // A nested split and its removal must not replace the ancestor's size state.
    cx.update_window(handle.into(), |_, window, cx| {
        deck.update(cx, |deck, cx| deck.focus(window, cx));
        window.render_frame(cx);
        window.press(key("ctrl-shift-e", "cmd-shift-d"), cx);
        assert_eq!(deck.read(cx).count(), 3);
        window.press(key("ctrl-shift-w", "cmd-w"), cx);
        assert_eq!(deck.read(cx).count(), 2);
        let PaneTree::Split { state, .. } = &deck.read(cx).tabs[0].root else {
            panic!("ancestor split was removed");
        };
        assert_eq!(state.entity_id(), split_state.entity_id());
        root.update(cx, |root, cx| {
            root.show = false;
            cx.notify();
        });
        window.render_frame(cx);
        root.update(cx, |root, cx| {
            root.show = true;
            cx.notify();
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let restored = cx.update(|cx| split_state.read(cx).sizes().clone());
    assert!((restored[0] - resized[0]).abs() < px(2.));

    cx.update_window(handle.into(), |_, window, cx| {
        deck.update(cx, |deck, cx| deck.focus(window, cx));
        window.render_frame(cx);
        window.press(key("ctrl-shift-t", "cmd-t"), cx);
        let second_tab = deck.read(cx).active_tab.unwrap();
        window.press(key("ctrl-pageup", "cmd-shift-["), cx);
        assert_ne!(deck.read(cx).active_tab, Some(second_tab));
        assert!((split_state.read(cx).sizes()[0] - resized[0]).abs() < px(2.));
        window.press(key("ctrl-pagedown", "cmd-shift-]"), cx);
        assert_eq!(deck.read(cx).active_tab, Some(second_tab));
        window.press(key("ctrl-shift-w", "cmd-w"), cx);
        assert_eq!(deck.read(cx).count(), 2);
        for _ in 2..MAX_PANES {
            window.press(key("ctrl-shift-t", "cmd-t"), cx);
        }
        assert_eq!(deck.read(cx).count(), MAX_PANES);
        window.press(key("ctrl-shift-t", "cmd-t"), cx);
        window.press(key("ctrl-shift-d", "cmd-d"), cx);
        assert_eq!(deck.read(cx).count(), MAX_PANES);
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cx.update(|cx| deck.read(cx).tabs.iter().all(|tab| ready(&tab.root, cx))) {
        assert!(Instant::now() < deadline, "isolated PTYs did not start");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(key("ctrl-shift-t", "cmd-t"), cx);
        window.press(key("ctrl-shift-d", "cmd-d"), cx);
        window.input("z", cx);
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read(&input).unwrap_or_default().is_empty() {
        assert!(
            Instant::now() < deadline,
            "normal text did not reach the PTY"
        );
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(&input).unwrap(),
        b"z",
        "shortcut bytes leaked into a PTY"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        for _ in 0..MAX_PANES {
            window.press(key("ctrl-shift-w", "cmd-w"), cx);
        }
        assert_eq!(deck.read(cx).count(), 0);
        // Closing the last pane keeps the deck focusable so Ctrl+Shift+T works.
        window.press(key("ctrl-shift-t", "cmd-t"), cx);
        assert_eq!(deck.read(cx).count(), 1);
        let other = root.read(cx).other.clone();
        other.update(cx, |other, cx| other.focus(window, cx));
        window.render_frame(cx);
        window.press(key("ctrl-shift-t", "cmd-t"), cx);
        assert_eq!(
            deck.read(cx).count(),
            1,
            "terminal shortcut escaped its scope"
        );
        deck.update(cx, |deck, cx| deck.focus(window, cx));
        window.render_frame(cx);
        window.press(key("ctrl-shift-w", "cmd-w"), cx);
        assert_eq!(deck.read(cx).count(), 0);
    })
    .unwrap();
    terminal_state.shutdown();
    cx.run_until_parked();
    assert!(terminal_state.shutdown_complete());
}
