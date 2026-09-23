//! agent-plugin — plugin contract + manager.
//!
//! Phase order (plugin-system.md §Phased rollout): **commands → providers →
//! hooks**. This stage ships the `Plugin` trait, `PluginManifest` +
//! `mcpServers` synthesis, the full MCP stdio lifecycle client, and
//! `PluginManager` (dispatch + namespacing + trust store). WASM lands
//! feature-gated — Wasmtime pulls a C toolchain against the single-binary
//! constraint, so `wasm.rs` is a stub until the policy for it is decided.

pub mod manager;
pub mod manifest;
pub mod mcp;

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

pub use manager::{discover, load_all, PluginInfo, PluginManager, TrustStore};
pub use manifest::{McpServerEntry, PluginKind, PluginManifest};
pub use mcp::McpClient;

/// One plugin — WASM or MCP, mapped onto the same trait so the manager,
/// permission pipeline, and tool router can't tell them apart.
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    fn id(&self) -> &str;
    /// JSON Schema array for the LLM `tools` request.
    fn export_tools(&self) -> Vec<Value>;
    async fn call_tool(&self, name: &str, args: Value) -> Result<String>;
    /// Context-provider hook — injects `<plugin_context>` after the
    /// workspace skeleton.
    async fn provide_context(&self, _workspace: &str) -> Option<String> {
        None
    }
}

/// An MCP plugin — wraps `McpClient`.
pub struct McpPlugin {
    pub manifest: PluginManifest,
    pub client: Arc<McpClient>,
}

#[async_trait::async_trait]
impl Plugin for McpPlugin {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn export_tools(&self) -> Vec<Value> {
        // Cache from tools/list — block briefly; export is on the sync path.
        let tools = self.client.tools.try_lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        tools
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t["name"].as_str().unwrap_or(""),
                        "description": t["description"].as_str().unwrap_or(""),
                        "parameters": t["inputSchema"].clone(),
                    }
                })
            })
            .collect()
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        self.client.call_tool(name, args).await
    }

    async fn provide_context(&self, _workspace: &str) -> Option<String> {
        // MCP resources surface as context — read each declared resource.
        let uris: Vec<String> = self
            .client
            .resources
            .lock()
            .await
            .iter()
            .filter_map(|r| r["uri"].as_str().map(String::from))
            .collect();
        if uris.is_empty() {
            return None;
        }
        let mut out = String::new();
        for uri in uris.iter().take(3) {
            if let Ok(text) = self.client.read_resource(uri).await {
                out.push_str(&format!("── {uri} ──\n{text}\n"));
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
}
