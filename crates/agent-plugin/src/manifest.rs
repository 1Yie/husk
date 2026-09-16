//! Plugin manifest — self-describing `manifest.json` + `mcpServers` TOML
//! synthesis (plugin-system.md §Plugin contract + §Standard config format).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

/// Runtime kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// WASM in-process sandbox (Wasmtime; Stage-9 feature-gated).
    Wasm,
    /// MCP child process over stdio JSON-RPC.
    Mcp,
}

/// Declared capabilities — **exhaustive**: anything not here is denied.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginPermissions {
    /// Host allowlist for the host-side `fetch` fn (empty = no network).
    #[serde(default)]
    pub network: Vec<String>,
    /// `read:<path>` / `write:<path>` preopens (WASM) — absent = no fs.
    #[serde(default)]
    pub filesystem: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDecl {
    pub name: String,
    pub description: String,
    /// JSON Schema object — validated at `register_plugin`.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDecl {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandDecl {
    pub name: String,
    pub description: String,
    /// `"tool:<name>"` → dispatch that tool; `"prompt:<text>"` → FeedToAgent.
    pub action: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookDecl {
    /// `before_tool_execute` / `after_tool_execute` / `on_user_input` /
    /// `on_state_transition`.
    pub event: String,
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
}

/// `manifest.json` — one per plugin directory.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "v1")]
    pub version: String,
    pub kind: PluginKind,
    /// wasm: file path; mcp: `{ "command": "npx", "args": [...] }`.
    pub entry: serde_json::Value,
    #[serde(default)]
    pub permissions: PluginPermissions,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// MCP: run under the process sandbox when the server is local-safe.
    #[serde(default)]
    pub sandboxed: bool,
    /// Filesystem dir the manifest lives in (set by discovery).
    #[serde(skip)]
    pub dir: PathBuf,
}

fn v1() -> String { "1.0.0".into() }

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub tools: Vec<ToolDecl>,
    #[serde(default)]
    pub context_providers: Vec<ProviderDecl>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub hooks: Vec<HookDecl>,
}

impl PluginManifest {
    /// Validate structural rules (schema validity, wasm-only hooks…).
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("manifest id empty".into());
        }
        // Hooks are WASM-only (plugin-system.md: MCP has no interception
        // semantics — stdio latency + no interception model).
        if self.kind == PluginKind::Mcp && !self.capabilities.hooks.is_empty() {
            return Err(format!(
                "MCP plugin `{}` declares hooks — MCP cannot hook",
                self.id
            ));
        }
        // Tool parameter schemas must be objects.
        for t in &self.capabilities.tools {
            if !t.parameters.is_object() {
                return Err(format!(
                    "tool `{}` parameters must be a JSON Schema object",
                    t.name
                ));
            }
        }
        Ok(())
    }

    /// Synthesize a manifest from a `[mcp_servers.*]` TOML entry — the
    /// community-compatible shape (Claude Desktop config paste).
    pub fn from_mcp_server(id: &str, e: &McpServerEntry) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            version: "mcp".into(),
            kind: PluginKind::Mcp,
            entry: serde_json::json!({
                "command": e.command, "args": e.args, "env": e.env,
            }),
            permissions: PluginPermissions::default(),
            capabilities: Capabilities::default(),
            sandboxed: e.sandboxed.unwrap_or(false),
            dir: PathBuf::new(),
        }
    }
}

/// `[mcp_servers.<id>]` TOML row.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub sandboxed: Option<bool>,
}
