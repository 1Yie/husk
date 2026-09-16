//! `PluginManager` — manifest validation → runtime build → tool routing.
//!
//! Contract (plugin-system.md §PluginManager):
//! - `register_plugin`: validate manifest → build runtime → `export_tools`
//!   → populate `tool_router`. Name collisions namespaced `plugin_id:tool`.
//! - `dispatch_tool_call`: route → `plugin.call_tool` → **same 40 KB
//!   truncation budget as built-ins** (plugin output is not exempt).
//! - `collect_dynamic_contexts`: fan out `provide_context` with per-plugin
//!   2 s timeout, tagged `<plugin_context id="…">`.
//! - Repo-local plugins inert until trusted (path-keyed consent store).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use tracing::{info, warn};

use crate::manifest::PluginKind;
use crate::{McpClient, McpPlugin, Plugin, PluginManifest};

/// Tool result cap — same as built-ins (kernel-architecture.md §truncation).
const PLUGIN_OUTPUT_CAP: usize = 40 * 1024;
/// Per-plugin `provide_context` budget.
const CONTEXT_TIMEOUT: Duration = Duration::from_secs(2);

/// Path-keyed trust store — repo-local plugins are inert until approved.
#[derive(Default)]
pub struct TrustStore {
    /// `<plugin_id>@<repo_path>` → trusted.
    trusted: HashMap<String, bool>,
}

impl TrustStore {
    fn key(id: &str, repo: &Path) -> String {
        format!("{}@{}", id, repo.display())
    }
    pub fn is_trusted(&self, id: &str, repo: &Path) -> bool {
        self.trusted.get(&Self::key(id, repo)).copied().unwrap_or(false)
    }
    pub fn trust(&mut self, id: &str, repo: &Path) {
        self.trusted.insert(Self::key(id, repo), true);
    }
}

#[derive(Default)]
pub struct PluginManager {
    plugins: HashMap<String, Arc<dyn Plugin>>,
    /// tool name → owning plugin (namespaced `plugin_id:tool` on collision).
    tool_router: HashMap<String, Arc<dyn Plugin>>,
    /// plugin_id → enabled flag (persisted; disabled plugins unload).
    enabled: HashMap<String, bool>,
    pub trust: TrustStore,
}

impl PluginManager {
    /// Register one manifest — validates, builds its runtime, exports tools.
    /// `repo` is the workspace root for repo-local trust scoping.
    pub async fn register_plugin(
        &mut self,
        manifest: PluginManifest,
        repo: &Path,
    ) -> Result<(), String> {
        manifest.validate()?;
        let id = manifest.id.clone();

        // Repo-local plugins need explicit trust before they're live.
        if !manifest.dir.as_os_str().is_empty()
            && manifest.dir.starts_with(repo.join(".agent"))
            && !self.trust.is_trusted(&id, repo)
        {
            return Err(format!(
                "plugin `{id}` is repo-local — trust consent required before load"
            ));
        }

        let plugin: Arc<dyn Plugin> = match manifest.kind {
            PluginKind::Mcp => {
                let client = McpClient::start(&manifest)
                    .await
                    .map_err(|e| format!("MCP `{id}` start: {e}"))?;
                Arc::new(McpPlugin { manifest, client })
            }
            PluginKind::Wasm => {
                // Feature-gated: wasmtime pulls a C toolchain — kept behind
                // `feature = "wasm"` until the single-binary policy settles.
                return Err("WASM plugins not built (feature `wasm` off)".into());
            }
        };

        // Export tools → router with collision namespacing.
        for t in plugin.export_tools() {
            let name = t["function"]["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let key = if self.tool_router.contains_key(&name) {
                format!("{}:{}", id, name) // namespaced on collision
            } else {
                name
            };
            self.tool_router.insert(key, plugin.clone());
        }
        self.plugins.insert(id.clone(), plugin);
        self.enabled.insert(id.clone(), true);
        info!(plugin = %id, "plugin registered");
        Ok(())
    }

    /// Route a tool call → owning plugin → truncates to the 40 KB budget.
    pub async fn dispatch_tool_call(
        &self,
        name: &str,
        args: Value,
    ) -> Result<String, String> {
        let plugin = self
            .tool_router
            .get(name)
            .or_else(|| {
                // Namespaced fallback: `plugin_id:tool` hits a plugin whose
                // bare name lost the collision.
                name.split_once(':')
                    .and_then(|(pid, _)| self.tool_router.get(name).or_else(|| {
                        self.plugins.get(pid)
                    }))
            })
            .ok_or_else(|| format!("no plugin owns tool `{name}`"))?;

        let out = plugin
            .call_tool(name.rsplit(':').next().unwrap_or(name), args)
            .await
            .map_err(|e| e.to_string())?;
        Ok(truncate(&out, PLUGIN_OUTPUT_CAP))
    }

    /// `<plugin_context>` blocks appended after the workspace skeleton —
    /// per-plugin 2 s timeout, tagged with the provider id.
    pub async fn collect_dynamic_contexts(&self, workspace: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (id, plugin) in &self.plugins {
            if !self.enabled.get(id).copied().unwrap_or(false) {
                continue;
            }
            let ws = workspace.to_string();
            let p = plugin.clone();
            let ctx = tokio::time::timeout(
                CONTEXT_TIMEOUT,
                p.provide_context(&ws),
            )
            .await;
            match ctx {
                Ok(Some(text)) => {
                    out.push(format!("<plugin_context id=\"{id}\">\n{text}\n</plugin_context>"));
                }
                Ok(None) => {}
                Err(_) => warn!(plugin = %id, "provide_context timed out"),
            }
        }
        out
    }

    /// Disable = drop plugin + purge router (its tools vanish next request).
    pub fn disable(&mut self, id: &str) {
        self.enabled.insert(id.to_string(), false);
        self.tool_router.retain(|_, p| p.id() != id);
    }

    pub fn enable(&mut self, id: &str) {
        self.enabled.insert(id.to_string(), true);
        // Re-registering rebuilds the router — tools reappear next request.
    }

    /// Plugin metadata for the settings panel.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|(id, p)| PluginInfo {
                id: id.clone(),
                kind: "mcp".into(), // wasm when that runtime lands
                enabled: self.enabled.get(id).copied().unwrap_or(false),
                tool_count: self.tool_router.values().filter(|r| r.id() == p.id()).count(),
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct PluginInfo {
    pub id: String,
    pub kind: String,
    pub enabled: bool,
    pub tool_count: usize,
}

/// Discover plugins: `~/.config/agent-rs/plugins/*/manifest.json` +
/// `<repo>/.agent/plugins/*/manifest.json`.
pub fn discover(repo: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".config/agent-rs/plugins"));
    }
    dirs.push(repo.join(".agent/plugins"));
    let mut manifests = Vec::new();
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let m = e.path().join("manifest.json");
                if m.exists() {
                    manifests.push(m);
                }
            }
        }
    }
    manifests
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut end = n;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n… [truncated {} bytes]", &s[..end], s.len() - end)
    }
}
