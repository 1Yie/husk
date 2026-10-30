use super::{SettingsPage, SettingsPanel};
use crate::bridge::{BridgeEvent, Command, HostBridge, Snapshot};
use gpui_kit::{
    AppContext, TestAppContext, WindowBounds, WindowOptions, px, size, test::TestWindowExt,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn snapshot(events: &async_channel::Receiver<BridgeEvent>, id: u64) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match events.try_recv() {
            Ok(BridgeEvent::Snapshot {
                request_id,
                snapshot,
            }) if request_id == id => return snapshot,
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => panic!("{message}"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "settings snapshot timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[gpui_kit::test]
fn memory_controls_track_pending_requests_and_keep_storage_failures_local(cx: &mut TestAppContext) {
    const NAME: &str = "features::settings::tests::memory_controls_track_pending_requests_and_keep_storage_failures_local";
    if std::env::var("HUSK_GPUI_SETTINGS_UI_TEST").as_deref() != Ok(NAME) {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env("HUSK_GPUI_SETTINGS_UI_TEST", NAME)
            .env("AGENT_SESSION_DB", dir.path().join("sessions"))
            .env("TMPDIR", dir.path())
            .env("TMP", dir.path())
            .env("TEMP", dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "isolated settings UI test failed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let paths =
        agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
    let bridge = Arc::new(HostBridge::spawn(paths).unwrap());
    let events = bridge.events();
    let initial = snapshot(&events, 0);
    let memory = initial.config.memory_preferences.unwrap();
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1000.), px(1100.)), cx)),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| {
                    let mut view = SettingsPanel::new(bridge.clone());
                    view.set_snapshot(initial, cx);
                    view.set_page(SettingsPage::Memory, cx);
                    view
                })
            },
        )
        .unwrap()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("memory-enabled", cx);
        assert_eq!(window.find("memory-enabled").disabled(), Some(true));
        assert_eq!(window.find("memory-distill").disabled(), Some(true));
        window.click("memory-enabled", cx);
        assert_eq!(view.read(cx).pending.len(), 1);
    })
    .unwrap();
    let id = cx.update(|cx| *view.read(cx).pending.iter().next().unwrap());
    let saved = snapshot(&events, id);
    assert_eq!(
        saved.config.memory_preferences.unwrap().enabled,
        !memory.enabled
    );
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |view, cx| {
            // A navigation-superseded snapshot can acknowledge without replacing
            // the currently displayed page data.
            view.acknowledge(id, cx);
            assert!(view.pending.is_empty());
            view.set_snapshot(saved, cx);
        });
        window.render_frame(cx);
        assert!(!window.find("memory-enabled").disabled().unwrap_or(false));
        window.click("memory-distill", cx);
    })
    .unwrap();
    let id = cx.update(|cx| *view.read(cx).pending.iter().next().unwrap());
    let saved = snapshot(&events, id);
    assert_eq!(
        saved.config.memory_preferences.unwrap().distill,
        !memory.distill
    );
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |view, cx| {
            view.acknowledge(id, cx);
            view.set_snapshot(saved, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();

    let pref_path = agent_store::migrate_session_database()
        .unwrap()
        .join("globals/default_preferences.json");
    std::fs::write(&pref_path, b"{invalid").unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("memory-enabled", cx)
    })
    .unwrap();
    let id = cx.update(|cx| *view.read(cx).pending.iter().next().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    let message = loop {
        match events.try_recv() {
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => break message,
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "settings failure timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            assert!(!view.request_failed(id + 100, "unrelated command", cx));
            assert!(view.request_failed(id, &message, cx));
            assert!(view.pending.is_empty());
            assert_eq!(view.error.as_deref(), Some(message.as_str()));
            view.acknowledge(id + 100, cx);
            assert!(
                view.error.is_some(),
                "unrelated snapshots must not clear the error"
            );
        });
    });
    assert_eq!(std::fs::read(&pref_path).unwrap(), b"{invalid");
    let id = bridge.send(Command::Refresh).unwrap();
    let unavailable = snapshot(&events, id);
    assert!(unavailable.config.memory_preferences.is_none());
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |view, cx| view.set_snapshot(unavailable, cx));
        window.render_frame(cx);
        assert_eq!(window.find("memory-enabled").disabled(), Some(true));
        assert_eq!(window.find("memory-distill").disabled(), Some(true));
    })
    .unwrap();
    bridge.shutdown();
}

#[gpui_kit::test]
fn failed_form_request_preserves_draft_and_unrelated_ack_does_not_close_it(
    cx: &mut TestAppContext,
) {
    const NAME: &str = "features::settings::tests::failed_form_request_preserves_draft_and_unrelated_ack_does_not_close_it";
    if std::env::var("HUSK_GPUI_SETTINGS_UI_TEST").as_deref() != Ok(NAME) {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env("HUSK_GPUI_SETTINGS_UI_TEST", NAME)
            .env("AGENT_SESSION_DB", dir.path().join("sessions"))
            .env("TMPDIR", dir.path())
            .env("TMP", dir.path())
            .env("TEMP", dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let paths =
        agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
    let bridge = Arc::new(HostBridge::spawn(paths).unwrap());
    let events = bridge.events();
    let initial = snapshot(&events, 0);
    cx.update(gpui_kit::init);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1000.), px(900.)), cx)),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| {
                    let mut view = SettingsPanel::new(bridge.clone());
                    view.set_snapshot(initial, cx);
                    view
                })
            },
        )
        .unwrap()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |view, cx| {
            let editor = cx.new(|cx| {
                super::editor::Editor::new(
                    super::editor::EditorAction::Instructions("workspace".into()),
                    vec![super::editor::FieldSpec::long(
                        "content",
                        "指令",
                        "unsaved test draft",
                    )],
                    window,
                    cx,
                )
            });
            view.editor = Some(editor.clone());
            view.editor_request = Some(77);
            view.pending.insert(77);
            editor.update(cx, |e, _| e.pending = true);
            view.acknowledge(999, cx);
            assert!(!view.close_editor);
            assert!(editor.read(cx).pending);
            assert!(!view.request_failed(999, "unrelated", cx));
            assert!(view.request_failed(77, "read-only filesystem", cx));
            assert!(!editor.read(cx).pending);
            assert_eq!(
                editor.read(cx).error.as_deref(),
                Some("read-only filesystem")
            );
            match editor.read(cx).command(cx).unwrap() {
                crate::bridge::SettingsCommand::Instructions { scope, content } => {
                    assert_eq!(scope, "workspace");
                    assert_eq!(content, "unsaved test draft");
                }
                other => panic!("unexpected command: {other:?}"),
            }
            assert!(!view.close_editor);
            assert!(view.editor.is_some());
        });
    })
    .unwrap();
    bridge.shutdown();
}
