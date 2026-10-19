//! Desktop-shell developer settings — `<config dir>/settings.toml`.

use std::path::{Path, PathBuf};

use agent_llm::config::ConfigError;
use serde::{Deserialize, Serialize};

/// User-level developer overrides — `<config dir>/settings.toml`.
///
/// **Why a dedicated file:** these settings can prevent the GUI from
/// starting (`renderer = "wayland"` on a driver that can't do it,
/// `gpu_acceleration = false` on a stack that needs hardware GL). Keeping
/// them out of `config.toml` (which holds providers/keys) means a bad
/// value lives beside the file a user is already expected to edit by
/// hand — and the recovery path is a single obvious file, not a buried
/// section of the provider config.
///
/// All fields `Option`: `None` = "not set — follow the compiled default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevConfig {
    /// Rendering backend for GTK/WebKitGTK: `"x11"` (default, XWayland-safe) |
    /// `"wayland"` (native — fixes some HiDPI/fractional-scale paths, breaks
    /// on drivers without a working Wayland GL stack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<String>,
    /// `false` → software GL: `WEBKIT_DISABLE_DMABUF_RENDERER=1` and
    /// `LIBGL_ALWAYS_SOFTWARE=1`. `true` / `None` → hardware acceleration
    /// on (dmabuf renderer, GPU compositing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_acceleration: Option<bool>,
    /// `true` → Skia rasterises tiles on the CPU while compositing stays on
    /// the GPU (`WEBKIT_SKIA_ENABLE_CPU_RENDERING=1`). `None` → on for
    /// NVIDIA, whose EGL driver segfaults in Skia's GL teardown; off
    /// elsewhere. Written by the renderer-crash recovery as its first step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_raster: Option<bool>,
    /// `true` → the Shift+Ctrl+P monitor overlay is enabled (the window's
    /// keyboard shortcut hook is wired only when this is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_panel: Option<bool>,
    /// `true` → F12 / Ctrl+Shift+I/C/J/K shortcuts reach the WebKitGTK
    /// inspector in a packaged build (dev mode keeps them anyway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devtools: Option<bool>,
    /// `true` → the "开发者选项" settings entry is visible in the nav.
    /// Hidden by default — a typical user never touches renderer flags,
    /// and the only way to surface this pane is hand-editing this file
    /// (`developer_ui = true`) and relaunching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_ui: Option<bool>,
}

impl DevConfig {
    /// The canonical user-level path: `<config dir>/settings.toml` — same
    /// directory as `config.toml`, separate file so a bad renderer flag never
    /// sits next to provider secrets. The directory is the host's to name.
    pub fn default_path(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.toml")
    }

    /// Load `<config dir>/settings.toml`; missing file → default.
    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let path = Self::default_path(config_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;
        toml::from_str(&raw).map_err(|e| ConfigError::Parse(path.clone(), e.to_string()))
    }

    /// Write `<config dir>/settings.toml` — the whole file is ours, so this
    /// overwrites rather than merges.
    pub fn save(config_dir: &Path, dev: &DevConfig) -> Result<(), ConfigError> {
        let path = Self::default_path(config_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Read(parent.to_path_buf(), e))?;
        }
        let out = toml::to_string_pretty(dev)
            .map_err(|e| ConfigError::Parse(path.clone(), e.to_string()))?;
        std::fs::write(&path, out).map_err(|e| ConfigError::Read(path.clone(), e))
    }
}
