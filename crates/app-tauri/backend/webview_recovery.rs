//! Renderer-crash recovery for the Linux WebKitGTK webview.
//!
//! WebKitGTK runs the page in a separate `WebKitWebProcess`. When that process
//! dies (the NVIDIA/EGL crash loop this machine sees) the window keeps the last
//! frame — or turns white — and Tauri has no built-in reload. Without this hook
//! the only recovery is killing the whole app.
//!
//! Recovery policy:
//! - `TerminatedByApi` (app shutdown) is ignored.
//! - A crash reloads the page after a short delay; the React app boots again
//!   and resumes the last session from the backend.
//! - Three crashes in ten minutes are treated as a renderer/GPU crash loop:
//!   step down one rendering level in `settings.toml` for the next launch and
//!   show a static diagnostics page instead of reloading forever. The first
//!   step is CPU rasterisation (`cpu_raster = true`), which keeps GPU
//!   compositing; only a crash loop that already runs CPU raster falls to
//!   full software rendering (`gpu_acceleration = false`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::glib;
use tauri::Manager;
use webkit2gtk::{WebProcessTerminationReason, WebViewExt};

/// Crashes inside this window count toward the crash-loop threshold.
const CRASH_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Reloads before falling back to software rendering.
const MAX_RELOADS: usize = 3;
/// Delay before reloading: WebKit is still tearing the dead process down.
const RELOAD_DELAY: Duration = Duration::from_millis(400);
/// WebKitGTK's switch for Skia CPU rasterisation; set by
/// `pin_linux_rendering_env` before GTK init.
pub const CPU_RASTER_ENV: &str = "WEBKIT_SKIA_ENABLE_CPU_RENDERING";

/// The rendering level the crash recovery stepped down to.
#[derive(Clone, Copy)]
enum Fallback {
    CpuRaster,
    Software,
}

/// Install the `web-process-terminated` listener on the main window.
pub fn install(app: &tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        tracing::warn!("webview recovery: main window not found");
        return;
    };
    let crashes = Arc::new(Mutex::new(Vec::<Instant>::new()));
    let safe_mode_written = Arc::new(AtomicBool::new(false));

    let result = window.with_webview(move |webview| {
        let view = webview.inner();
        let crashes = crashes.clone();
        let safe_mode_written = safe_mode_written.clone();
        view.connect_web_process_terminated(move |view, reason| {
            let label = match reason {
                WebProcessTerminationReason::Crashed => "crashed",
                WebProcessTerminationReason::ExceededMemoryLimit => "exceeded-memory-limit",
                WebProcessTerminationReason::TerminatedByApi => "terminated-by-api",
                _ => "unknown",
            };
            // Normal teardown (window/app shutdown) must not trigger a reload.
            if reason == WebProcessTerminationReason::TerminatedByApi {
                tracing::info!(reason = label, "web process terminated by API");
                return;
            }
            tracing::error!(reason = label, "web process terminated; recovering webview");

            let recent = {
                let mut crashes = crashes.lock().unwrap_or_else(|error| error.into_inner());
                crashes.retain(|at| at.elapsed() < CRASH_WINDOW);
                crashes.push(Instant::now());
                crashes.len()
            };
            tracing::warn!(
                crashes = recent,
                window = ?CRASH_WINDOW,
                "web process crash count"
            );

            if recent >= MAX_RELOADS {
                // Software rendering only addresses a GPU/driver crash; an
                // exceeded-memory-limit crash must not silently rewrite the
                // user's renderer settings.
                let mut fallback = None;
                if reason == WebProcessTerminationReason::Crashed
                    && !safe_mode_written.swap(true, Ordering::Relaxed)
                {
                    match step_down_rendering() {
                        Ok((step, path)) => {
                            fallback = Some(step);
                            tracing::warn!(
                                path = %path.display(),
                                fallback = step.setting(),
                                "renderer crash loop: rendering fallback written for the next launch"
                            );
                        }
                        Err(error) => {
                            tracing::error!(%error, "could not persist rendering fallback")
                        }
                    }
                }
                view.load_html(&crash_page(label, fallback), None);
                return;
            }

            let view = view.clone();
            glib::timeout_add_local_once(RELOAD_DELAY, move || {
                tracing::warn!("reloading webview after renderer crash");
                view.reload();
            });
        });
    });
    if let Err(error) = result {
        tracing::error!(%error, "could not install web-process crash recovery");
    } else {
        tracing::info!("web-process crash recovery installed");
    }
}

impl Fallback {
    fn setting(self) -> &'static str {
        match self {
            Fallback::CpuRaster => "cpu_raster = true",
            Fallback::Software => "gpu_acceleration = false",
        }
    }
}

/// Step one rendering level down without disturbing the other dev flags:
/// GPU raster → CPU raster (GPU compositing kept) → software rendering.
/// The current level is read from the env this process launched with, so a
/// per-GPU default that already enabled CPU raster goes straight to software.
fn step_down_rendering() -> Result<(Fallback, std::path::PathBuf), String> {
    let paths = crate::kernel::app_paths();
    let path = crate::dev_config::DevConfig::default_path(&paths.config_dir);
    let mut dev =
        crate::dev_config::DevConfig::load(&paths.config_dir).map_err(|e| e.to_string())?;
    let step = if std::env::var_os(CPU_RASTER_ENV).is_some() {
        dev.gpu_acceleration = Some(false);
        Fallback::Software
    } else {
        dev.cpu_raster = Some(true);
        Fallback::CpuRaster
    };
    crate::dev_config::DevConfig::save(&paths.config_dir, &dev).map_err(|e| e.to_string())?;
    Ok((step, path))
}

fn crash_page(reason: &str, fallback: Option<Fallback>) -> String {
    let log = crate::logging::log_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~/.local/share/husk/logs/husk.log".into());
    let fallback = match fallback {
        Some(Fallback::CpuRaster) => "<p>已为下次启动写入 <code>cpu_raster = true</code>（CPU 光栅化，保留 GPU 合成）。请<strong>退出并重新打开 Husk</strong>，该设置才会生效。</p>",
        Some(Fallback::Software) => "<p>已为下次启动写入 <code>gpu_acceleration = false</code>（软件渲染）。请<strong>退出并重新打开 Husk</strong>，该设置才会生效。</p>",
        None => "<p>请<strong>退出并重新打开 Husk</strong>。如果再次出现，可在 <code>~/.config/husk/settings.toml</code> 中设置 <code>cpu_raster = true</code>，仍不行再设 <code>gpu_acceleration = false</code>。</p>",
    };
    format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>Husk · 渲染进程崩溃</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{ margin:0; min-height:100vh; display:flex; align-items:center; justify-content:center;
         font:14px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif; background:#fafafa; color:#1a1c1f; }}
  main {{ max-width:560px; padding:32px; }}
  h1 {{ font-size:18px; margin:0 0 12px; }}
  p {{ margin:0 0 12px; color:#52525b; }}
  code {{ background:#f1f1f3; border-radius:4px; padding:1px 5px; font-size:12px; }}
  button {{ margin-top:8px; padding:7px 14px; border:1px solid #d4d4d8; border-radius:6px;
            background:#fff; color:#1a1c1f; font-size:13px; cursor:pointer; }}
  button:hover {{ background:#f4f4f5; }}
  .hint {{ margin-top:16px; font-size:12px; color:#71717a; }}
</style></head>
<body><main>
  <h1>渲染进程连续崩溃，已停止自动重载</h1>
  <p>页面进程（WebKitWebProcess）在短时间内崩溃了 {max} 次（最近一次原因：<code>{reason}</code>）。</p>
  {fallback}
  <button onclick="location.reload()">仍然重试本页</button>
  <p class="hint">诊断日志：<code>{log}</code><br>
  渲染相关配置：<code>~/.config/husk/settings.toml</code>（<code>cpu_raster</code> / <code>gpu_acceleration</code> / <code>renderer</code>）。</p>
</main></body></html>"#,
        max = MAX_RELOADS,
        reason = escape_html(reason),
        fallback = fallback,
        log = escape_html(&log),
    )
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
