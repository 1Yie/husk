//! `serena` — Serena's semantic code tools behind one meta-tool.
//!
//! `serena start-mcp-server` runs as an MCP child over stdio (newline-delimited
//! JSON-RPC — the MCP stdio transport, so the kernel keeps its "no reqwest"
//! constraint), and the agent reaches its whole suite through a single `serena`
//! tool instead of ~40 registered entries.
//!
//! ```text
//! workspace  →  SerenaManager (one slot per root)
//!                 └── SerenaBridge   — the child process + JSON-RPC plumbing
//!                 └── SerenaToolCatalog — cached `tools/list`
//! ```
//!
//! Graceful degradation: `uvx`/serena missing, or the server failing to come
//! up, errors only on that call — everything else keeps working, a failed call
//! carries the child's stderr tail, and a dead child is replaced on the next
//! call instead of being cached forever.
//!
//! Bootstrap vs execution: the first `uvx --from git+…` resolve fetches Serena
//! and therefore needs network + `uvx`; the tool's `network: false` describes
//! *execution* (local stdio), not that bootstrap.

pub mod bridge;
pub mod catalog;
pub mod manager;
pub mod tool;

pub use bridge::{BridgeFactory, McpBridge, SerenaBridge, SerenaLaunch, SerenaToolInfo};
pub use catalog::SerenaToolCatalog;
pub use manager::{manager_for, shutdown, SerenaManager};

pub fn spec() -> super::registry::ToolSpec {
    tool::spec()
}
