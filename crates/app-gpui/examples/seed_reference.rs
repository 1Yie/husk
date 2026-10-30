use std::path::PathBuf;
use std::time::{Duration, Instant};

use husk_gpui::bridge::{BridgeEvent, Command, HostBridge, Snapshot};

fn snapshot(
    events: &async_channel::Receiver<BridgeEvent>,
    request: u64,
) -> Result<Snapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(BridgeEvent::Snapshot {
                request_id,
                snapshot,
            }) if request_id == request => {
                return Ok(snapshot);
            }
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == request => {
                return Err(message.into());
            }
            Err(async_channel::TryRecvError::Closed) => return Err("host disconnected".into()),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    Err("host snapshot timed out".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("expected an isolated profile directory")?,
    );
    if !root.is_absolute() || !root.join("native-smoke-profile").is_file() {
        return Err("refusing to seed a directory without the smoke-profile marker".into());
    }
    let paths =
        agent_paths::AppPaths::new(root.join("config/husk"), root.join("data/husk"), ".husk");
    let bridge = HostBridge::spawn(paths)?;
    let events = bridge.events();
    snapshot(&events, 0)?;
    let request = bridge.send(Command::SwitchWorkspace(root.join("workspace")))?;
    snapshot(&events, request)?;
    for prompt in [
        "核对工作区改动与文件预览",
        "检查终端的字体、配色和分屏",
        "整理本次会话的操作与验证结果",
        "核对不同工作区之间的会话隔离",
        "验证长标题在窄侧栏内保持正确截断且不会遮挡固定操作",
        "验证原生界面的布局与流式消息",
    ] {
        let request = bridge.send(Command::NewSession)?;
        let state = snapshot(&events, request)?;
        let key = state.active.ok_or("new session has no active key")?;
        bridge.send(Command::Send {
            key: key.clone(),
            text: prompt.into(),
        })?;
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut received_text = false;
        let mut finished = false;
        while Instant::now() < deadline {
            match events.try_recv() {
                Ok(BridgeEvent::Ui { key: owner, event }) if owner == key => match event {
                    agent_ipc::UiEvent::AssistantMessage(text) => received_text |= !text.is_empty(),
                    agent_ipc::UiEvent::StateChanged(state) => finished = !state.is_active(),
                    agent_ipc::UiEvent::Error(error) => return Err(error.into()),
                    _ => {}
                },
                Ok(BridgeEvent::Error { message, .. }) => return Err(message.into()),
                Err(async_channel::TryRecvError::Closed) => return Err("host disconnected".into()),
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
            if finished && received_text {
                break;
            }
        }
        if !finished || !received_text {
            return Err("local streamed turn did not complete".into());
        }
        println!("Completed session {} in {}", key.id, key.root);
    }
    bridge.shutdown();
    std::thread::sleep(Duration::from_millis(250));
    Ok(())
}
