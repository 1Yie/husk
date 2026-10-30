//! Only local application webviews may operate their own PTYs. Tauri's
//! existing capability/origin gate still applies; do not grant remote access.
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    Manager, State, WebviewWindow,
};

use crate::terminal::{OpenedTerminal, TerminalEvent, TerminalState};

fn owner(window: &WebviewWindow) -> Result<String, String> {
    let url = window.url().map_err(|e| e.to_string())?;
    let bundled = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"));
    let mut local =
        bundled && url.port().is_none() && url.username().is_empty() && url.password().is_none();
    #[cfg(debug_assertions)]
    if let Some(dev_url) = &window.app_handle().config().build.dev_url {
        local |= url.origin() == dev_url.origin();
    }
    if !local {
        return Err("terminal IPC requires a local application webview".into());
    }
    Ok(window.label().to_owned())
}

#[derive(Debug, serde::Serialize)]
pub struct TerminalFont {
    family: String,
    monospace: bool,
}

fn collect_font_families(fonts: &fontdb::Database) -> Vec<TerminalFont> {
    let mut families = std::collections::BTreeMap::<String, TerminalFont>::new();
    for face in fonts.faces() {
        // fontdb puts the English typographic family first when available;
        // neither a file stem nor a face's PostScript name is a CSS family.
        let Some((family, _)) = face.families.first() else {
            continue;
        };
        let family = family.trim();
        if family.is_empty() || family.chars().count() > 200 || family.chars().any(char::is_control)
        {
            continue;
        }
        families
            .entry(family.to_lowercase())
            .or_insert_with(|| TerminalFont {
                family: family.to_owned(),
                monospace: false,
            })
            .monospace |= face.monospaced;
    }
    let mut families: Vec<_> = families.into_values().collect();
    // Stable sorting retains the case-insensitive family order within groups.
    families.sort_by_key(|font| !font.monospace);
    families
}

#[tauri::command]
pub async fn terminal_fonts(window: WebviewWindow) -> Result<Vec<TerminalFont>, String> {
    owner(&window)?;
    tauri::async_runtime::spawn_blocking(|| {
        let mut fonts = fontdb::Database::new();
        fonts.load_system_fonts();
        collect_font_families(&fonts)
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn terminal_open(
    window: WebviewWindow,
    state: State<'_, TerminalState>,
    cwd: Option<String>,
    cols: u32,
    rows: u32,
    output: Channel,
) -> Result<OpenedTerminal, String> {
    let owner = owner(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.open(
            owner,
            cwd,
            cols,
            rows,
            std::sync::Arc::new(move |event| {
                output.send(event_body(event)?).map_err(|e| e.to_string())
            }),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Output bytes cross as a raw body, so the webview gets an `ArrayBuffer`.
/// As JSON every byte became a decimal number, several times the size, on
/// the one stream that runs at full speed (`cat` of a large file). Exit and
/// error stay JSON; the client tells them apart by type.
fn event_body(event: TerminalEvent) -> Result<InvokeResponseBody, String> {
    Ok(match event {
        TerminalEvent::Data { data } => InvokeResponseBody::Raw(data),
        other => {
            InvokeResponseBody::Json(serde_json::to_string(&other).map_err(|e| e.to_string())?)
        }
    })
}

#[tauri::command]
pub async fn terminal_write(
    window: WebviewWindow,
    state: State<'_, TerminalState>,
    id: String,
    data: String,
) -> Result<(), String> {
    let owner = owner(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.write(&owner, &id, &data))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn terminal_resize(
    window: WebviewWindow,
    state: State<'_, TerminalState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    let owner = owner(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.resize(&owner, &id, cols, rows))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn terminal_close(
    window: WebviewWindow,
    state: State<'_, TerminalState>,
    id: String,
) -> Result<(), String> {
    let owner = owner(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.close(&owner, &id))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_families_use_real_names_merge_faces_and_prefer_monospace() {
        let mut fonts = fontdb::Database::new();
        fonts.load_font_data(
            include_bytes!("../../frontend/assets/font/JetBrainsMonoNerdFontMono-Regular.ttf")
                .to_vec(),
        );
        let face = fonts.faces().next().unwrap().clone();
        let bundled = collect_font_families(&fonts);
        assert!(bundled.iter().any(|font| {
            font.family.contains("JetBrains") && font.family.contains("Nerd") && font.monospace
        }));

        let mut fonts = fontdb::Database::new();
        for (family, monospace) in [
            ("Arial", false),
            ("Zed Mono", false),
            ("Zed Mono", true),
            ("arial", false),
            ("Iosevka Nerd Font Mono", true),
            ("中文等宽", true),
            ("", true),
            ("bad\nfamily", true),
        ] {
            let mut info = face.clone();
            info.families = vec![(family.into(), fontdb::Language::English_UnitedStates)];
            info.monospaced = monospace;
            fonts.push_face_info(info);
        }
        let families = collect_font_families(&fonts);
        assert_eq!(
            families
                .iter()
                .map(|font| font.family.as_str())
                .collect::<Vec<_>>(),
            ["Iosevka Nerd Font Mono", "Zed Mono", "中文等宽", "Arial"]
        );
        assert!(families[..3].iter().all(|font| font.monospace));
        let serialized = serde_json::to_value(&families).unwrap();
        assert_eq!(
            serialized[0],
            serde_json::json!({
                "family": "Iosevka Nerd Font Mono", "monospace": true
            })
        );
    }

    #[test]
    fn output_crosses_raw_and_lifecycle_events_stay_json() {
        match event_body(TerminalEvent::Data {
            data: vec![0xe4, 0xbd, 0xa0],
        })
        .unwrap()
        {
            InvokeResponseBody::Raw(bytes) => assert_eq!(bytes, [0xe4, 0xbd, 0xa0]),
            other => panic!("data must be raw: {other:?}"),
        }
        match event_body(TerminalEvent::Exit {
            code: Some(7),
            signal: None,
        })
        .unwrap()
        {
            InvokeResponseBody::Json(json) => {
                assert_eq!(json, r#"{"type":"exit","code":7,"signal":null}"#)
            }
            other => panic!("exit must be JSON: {other:?}"),
        }
    }
}
