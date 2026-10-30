use super::{Attachment, ChatPanel, Command, HostBridge, build_payload};
use crate::bridge::{BridgeEvent, Snapshot};
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
                assert!(Instant::now() < deadline, "snapshot timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn updated(events: &async_channel::Receiver<BridgeEvent>, id: u64) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut accepted = false;
    let mut state = None;
    loop {
        match events.try_recv() {
            Ok(BridgeEvent::QueueUpdated { request_id }) if request_id == id => accepted = true,
            Ok(BridgeEvent::Snapshot {
                request_id,
                snapshot,
            }) if request_id == id => {
                state = Some(snapshot);
            }
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => panic!("{message}"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "queue update timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
        if accepted && let Some(state) = state.take() {
            return state;
        }
    }
}

#[gpui_kit::test]
fn editing_and_reordering_keep_drafts_and_reject_a_racing_change(cx: &mut TestAppContext) {
    const NAME: &str = "features::chat::view::queued_tests::editing_and_reordering_keep_drafts_and_reject_a_racing_change";
    // The host store is process-global. Keep this real-host window test isolated
    // from the existing file-picker test and the bridge's own fixtures.
    if std::env::var("HUSK_GPUI_QUEUE_UI_TEST").as_deref() != Ok(NAME) {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env("HUSK_GPUI_QUEUE_UI_TEST", NAME)
            .env("AGENT_SESSION_DB", dir.path().join("sessions"))
            .env("TMPDIR", dir.path())
            .env("TMP", dir.path())
            .env("TEMP", dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "isolated queue UI test failed");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let paths =
        agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
    husk_host::office::init_store(paths.data_dir.clone());
    let source = workspace.join("queued.txt");
    std::fs::write(&source, "private queued attachment").unwrap();
    let queued_attachment = Attachment::load(source.clone(), &workspace).unwrap();
    let payload = build_payload("original queued text", &[queued_attachment]);
    std::fs::remove_file(source).unwrap();
    let store = agent_store::SessionStore::open(&workspace).unwrap();
    store
        .commit_turn(
            &[],
            agent_store::SessionMeta {
                id: 1,
                title: "queue test".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let initial = vec![payload.clone(), "second".into(), "third".into()];
    store.update_queue(1, initial.clone()).unwrap();
    store.set_last_active(1).unwrap();
    let bridge = Arc::new(HostBridge::spawn(paths).unwrap());
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge
        .send(Command::SwitchWorkspace(workspace.clone()))
        .unwrap();
    let state = snapshot(&events, id);
    let key = state.active.unwrap();
    let normal_path = workspace.join("draft.txt");
    std::fs::write(&normal_path, "normal attachment").unwrap();
    let normal_attachment = Attachment::load(normal_path, &workspace).unwrap();

    cx.update(gpui_kit::init);
    let (handle, panel) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(680.), px(520.)), cx)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut panel = ChatPanel::new(key.clone(), bridge.clone(), &[], window, cx);
                    panel.model.queued_prompts = initial;
                    panel
                        .composer
                        .update(cx, |input, cx| input.set_value("normal draft", window, cx));
                    panel.attachments.push(normal_attachment.clone());
                    panel
                })
            },
        )
        .unwrap()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("move-queued-up", 0usize)).disabled(),
            Some(true)
        );
        assert_eq!(
            window.find(("move-queued-down", 2usize)).disabled(),
            Some(true)
        );
        window.click(("edit-queued", 0usize), cx);
        panel.update(cx, |panel, cx| {
            assert_eq!(
                panel.queue_edit.as_ref().unwrap().input.read(cx).value(),
                "original queued text"
            );
        });
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("edited queued text", cx);
        window.click("save-queued-edit", cx);
        assert_eq!(window.find("save-queued-edit").disabled(), Some(true));
        assert_eq!(window.find("cancel-queued-edit").disabled(), Some(true));
        window.input("must not change while saving", cx);
    })
    .unwrap();
    let request_id = cx.update(|cx| panel.read(cx).queue_edit.as_ref().unwrap().pending.unwrap());
    let state = updated(&events, request_id);
    let edited_payload = payload.replacen("original queued text", "edited queued text", 1);
    assert_eq!(
        state.queued_prompts,
        [edited_payload.clone(), "second".into(), "third".into()]
    );
    cx.update_window(handle.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            assert_eq!(
                panel.queue_edit.as_ref().unwrap().input.read(cx).value(),
                "edited queued text"
            );
            panel.apply_event(
                &agent_ipc::UiEvent::QueuedPrompts {
                    items: state.queued_prompts.clone(),
                },
                cx,
            );
            panel.acknowledge_submission(request_id, window, cx);
            assert!(panel.queue_edit.is_none());
            assert_eq!(panel.composer.read(cx).value(), "normal draft");
            assert_eq!(panel.attachments, [normal_attachment.clone()]);
        });
        window.render_frame(cx);
        assert!(window.try_find("save-queued-edit").is_none());
        window.click(("move-queued-down", 0usize), cx);
    })
    .unwrap();
    // The next accepted queue update belongs to the reorder button.
    let deadline = Instant::now() + Duration::from_secs(20);
    let moved_id = loop {
        match events.try_recv() {
            Ok(BridgeEvent::QueueUpdated { request_id }) => break request_id,
            Ok(BridgeEvent::Error { message, .. }) => panic!("{message}"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "reorder timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    let state = snapshot(&events, moved_id);
    assert_eq!(
        state.queued_prompts,
        ["second".into(), edited_payload, "third".into()]
    );
    assert_eq!(store.meta(1).unwrap().queued_prompts, state.queued_prompts);
    cx.update_window(handle.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.apply_event(
                &agent_ipc::UiEvent::QueuedPrompts {
                    items: state.queued_prompts.clone(),
                },
                cx,
            );
        });
        window.render_frame(cx);
        window.click(("edit-queued", 1usize), cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("unsaved edit", cx);
    })
    .unwrap();
    // Simulate the actor consuming a message before the editor receives its
    // updated queue. The stale save must fail rather than reinsert a prompt.
    let id = bridge
        .send(Command::RemoveQueued {
            key,
            index: 0,
            expected: state.queued_prompts,
        })
        .unwrap();
    let actual = updated(&events, id);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("save-queued-edit", cx)
    })
    .unwrap();
    let failed_id = cx.update(|cx| panel.read(cx).queue_edit.as_ref().unwrap().pending.unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    let message = loop {
        match events.try_recv() {
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == failed_id => break message,
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "stale edit did not fail");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    assert!(message.contains("队列已变化"));
    cx.update_window(handle.into(), |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.apply_event(
                &agent_ipc::UiEvent::QueuedPrompts {
                    items: actual.queued_prompts.clone(),
                },
                cx,
            );
            assert!(panel.submission_failed(failed_id, &message, window, cx));
            let edit = panel.queue_edit.as_ref().unwrap();
            assert!(edit.pending.is_none());
            assert!(edit.error.is_some());
            assert_eq!(edit.input.read(cx).value(), "unsaved edit");
        });
        window.render_frame(cx);
        assert_eq!(window.find("save-queued-edit").disabled(), Some(true));
        window.click("copy-queued-edit", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("unsaved edit")
        );
        window.click("cancel-queued-edit", cx);
        panel.update(cx, |panel, cx| {
            assert!(panel.queue_edit.is_none());
            assert_eq!(panel.composer.read(cx).value(), "normal draft");
            assert_eq!(panel.attachments, [normal_attachment]);
        });
    })
    .unwrap();
    assert_eq!(store.meta(1).unwrap().queued_prompts, actual.queued_prompts);
    bridge.shutdown();
}
