//! `PluginManager` — manifest validation → runtime build → tool routing.
//!
//! Contract (plugin-system.md §PluginManager):
//! - `register_plugin`: validate manifest → build runtime → `export_tools`
//!   → populate `tool_router`. Tools are namespaced `plugin_id__tool`, both
//!   halves sanitized into the wire charset (see [`wire_tool_name`]) — the
//!   `plugin_id:tool` spelling this used to advertise is rejected outright by
//!   strict upstreams, because `:` is not a legal `function.name` character.
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
    /// Advertised tool name → owning plugin. Keyed by the WIRE name
    /// ([`wire_tool_name`]), which is what the model echoes back on a call.
    tool_router: HashMap<String, Arc<dyn Plugin>>,
    /// Advertised name → the server's own tool name (`ui-skills__get_skill` →
    /// `get_skill`), so dispatch can reach the real tool after sanitization
    /// replaced characters the wire charset forbids.
    real_tool_names: HashMap<String, String>,
    /// plugin_id → enabled flag (persisted; disabled plugins unload).
    enabled: HashMap<String, bool>,
    /// Why a plugin is not in `plugins` — set by `load_all` when registration
    /// fails (dead endpoint, timeout). Reported to the settings UI so a broken
    /// server can explain itself without a reconnect attempt.
    errors: HashMap<String, String>,
    pub trust: TrustStore,
}

impl PluginManager {
    /// Empty manager. `load_all` is the usual entry point.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            tool_router: HashMap::new(),
            real_tool_names: HashMap::new(),
            enabled: HashMap::new(),
            errors: HashMap::new(),
            trust: TrustStore {
                trusted: HashMap::new(),
            },
        }
    }

    /// Live status per plugin — identity and tool names captured during the
    /// boot handshake, plus the reason for any plugin that failed to load.
    /// Reads memory only: the settings pane polls this instead of reconnecting,
    /// which used to open a fresh MCP session on every visit (and trip the
    /// provider's rate limit).
    pub fn status(&self) -> Vec<Value> {
        let mut ids: Vec<&String> = self.plugins.keys().chain(self.errors.keys()).collect();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let plugin = self.plugins.get(id);
                let (name, version) = plugin.map(|p| p.server_info()).unwrap_or_default();
                serde_json::json!({
                    "id": id,
                    "connected": plugin.is_some(),
                    "enabled": self.enabled.get(id).copied().unwrap_or(false),
                    "serverName": name,
                    "serverVersion": version,
                    "tools": plugin.map(|p| p.tool_names()).unwrap_or_default(),
                    "error": self.errors.get(id).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Every enabled plugin's tools, named `plugin_id__tool` — exactly what the
    /// engine advertises to the model. Both halves pass through
    /// [`wire_tool_name`], so the result is a legal `function.name` on every
    /// OpenAI-shaped wire; the old `plugin_id:tool` spelling was rejected as
    /// `invalid_argument` by strict upstreams (`:` is outside the charset).
    pub fn exported_tools(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for (id, plugin) in &self.plugins {
            if !self.enabled.get(id).copied().unwrap_or(false) {
                continue;
            }
            for mut tool in plugin.export_tools() {
                let Some(name) = tool
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(String::from)
                else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                tool["function"]["name"] = Value::String(wire_tool_name(id, &name));
                out.push(tool);
            }
        }
        out
    }

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

// Export tools → router, keyed by the SAME name `exported_tools` puts in
        // the request. The two must agree exactly, or the model calls a name
        // the router has never heard of.
        for t in plugin.export_tools() {
            let name = t["function"]["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let advertised = wire_tool_name(&id, &name);
            self.real_tool_names.insert(advertised.clone(), name);
            self.tool_router.insert(advertised, plugin.clone());
        }
        self.plugins.insert(id.clone(), plugin);
        self.enabled.insert(id.clone(), true);
        info!(plugin = %id, "plugin registered");
        Ok(())
    }

    /// Route a tool call → owning plugin → truncates to the 40 KB budget.
    ///
    /// Resolution order: the advertised wire name (`ui-skills__get_skill`),
    /// then a bare name (`get_skill`) when exactly one plugin owns it, then a
    /// legacy `plugin_id:tool` spelling from a snapshot written before the
    /// colon became wire-illegal. Whichever spelling matched, the SERVER is
    /// called with its own tool name, never with the advertised one.
    pub async fn dispatch_tool_call(
        &self,
        name: &str,
        args: Value,
    ) -> Result<String, String> {
        let (plugin, real) = self
            .tool_router
            .get(name)
            .map(|p| (p.clone(), name.to_string()))
            .or_else(|| self.resolve_bare(name))
            .or_else(|| self.resolve_legacy(name))
            .ok_or_else(|| format!("no plugin owns tool `{name}`"))?;
        // The tool name the MCP server knows — the advertised name was passed
        // through the wire charset, which is lossy.
        let real = self.real_tool_names.get(&real).cloned().unwrap_or(real);

        let out = plugin
            .call_tool(&real, args)
            .await
            .map_err(|e| e.to_string())?;
        Ok(truncate(&out, PLUGIN_OUTPUT_CAP))
    }

    /// A bare (`get_skill`) or legacy (`ui-skills:get_skill`) call name →
    /// owning plugin + the advertised name to look the real tool up under.
    /// Unique-ness is judged against `real_tool_names`, built at registration.
    fn resolve_bare(&self, name: &str) -> Option<(Arc<dyn Plugin>, String)> {
        let advertised = self
            .real_tool_names
            .iter()
            .filter(|(_, real)| real.as_str() == name)
            .map(|(adv, _)| adv.clone())
            .collect::<Vec<_>>();
        if advertised.len() != 1 {
            return None; // unknown, or ambiguous across plugins
        }
        let adv = advertised.into_iter().next()?;
        self.tool_router.get(&adv).map(|p| (p.clone(), adv))
    }

    /// `plugin_id:tool` → the sanitized advertised name. Snapshots persisted
    /// before the colon was removed still replay these names. The plugin id
    /// may itself contain colons (`ui:skills:list_skills`), so EVERY split
    /// point is tried rather than just the first.
    fn resolve_legacy(&self, name: &str) -> Option<(Arc<dyn Plugin>, String)> {
        for (i, _) in name.match_indices(':') {
            let (pid, tool) = (&name[..i], &name[i + 1..]);
            let adv = wire_tool_name(pid, tool);
            if let Some(p) = self.tool_router.get(&adv) {
                return Some((p.clone(), adv));
            }
        }
        None
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
        // The advertised → real name map is keyed by the advertised name only,
        // so it must be purged alongside the router — a stale entry would let
        // `resolve_bare` match a tool the plugin no longer advertises.
        let plugin = self.plugins.get(id);
        if let Some(plugin) = plugin {
            for tool in plugin.export_tools() {
                if let Some(name) = tool["function"]["name"].as_str() {
                    self.real_tool_names.remove(&wire_tool_name(id, name));
                }
            }
        }
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

/// The name a plugin tool is ADVERTISED under — `plugin_id__tool`, every
/// character outside `[A-Za-z0-9_-]` folded to `_`.
///
/// The `tools[].function.name` charset on every OpenAI-shaped wire is
/// `^[a-zA-Z0-9_-]{1,64}$`: a `:` separator (the old spelling,
/// `ui-skills:list_skills`) is not in it, and a strict upstream rejects the
/// ENTIRE request with `invalid_argument` before the model ever runs — which
/// is exactly how a registered MCP server bricked every turn. `/`, `.` and
/// spaces are just as illegal, and a server is free to name its tools that way.
///
/// `__` is the separator because a single `_` is already common inside tool
/// names (`list_skills`), so one underscore could not be told apart from the
/// namespace boundary when resolving a call. Both halves are folded and
/// truncated so the joined name stays inside 64 chars.
pub fn wire_tool_name(plugin_id: &str, tool: &str) -> String {
    const MAX: usize = 64;
    const SEP: &str = "__";
    let id = sanitize_name(plugin_id);
    let tool = sanitize_name(tool);
    // The tool half matters more for a human scanning a picker, so trim the
    // namespace (not the tool) when the pair overflows.
    if id.len() + SEP.len() + tool.len() <= MAX {
        return format!("{id}{SEP}{tool}");
    }
    let room = MAX.saturating_sub(SEP.len() + tool.len());
    // `room` is a byte budget and the sanitized half is ASCII, so this is also
    // a char boundary — no need for the `is_char_boundary` walk.
    let id = &id[..room.min(id.len())];
    format!("{id}{SEP}{tool}")
}

/// Fold every character the wire charset forbids to `_`. ASCII-only by
/// construction (a non-ASCII char is itself illegal on the wire).
fn sanitize_name(s: &str) -> String {
    let folded: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if folded.is_empty() {
        "_".to_string()
    } else {
        folded
    }
}

/// Discover plugins: `~/.config/husk/plugins/*/manifest.json` +
/// `<repo>/.agent/plugins/*/manifest.json`.
pub fn discover(repo: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".config/husk/plugins"));
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

/// Discover the standard plugin dirs and register every MCP server.
///
/// Bounded on purpose: a dead endpoint is logged and skipped, so one broken
/// server can neither stall the app's boot nor keep the other plugins from
/// loading. Called once at startup (see `SessionManager::spawn_at`).
pub async fn load_all(repo: &Path) -> PluginManager {
    let mut mgr = PluginManager::new();
    for mpath in discover(repo) {
        let Ok(text) = std::fs::read_to_string(&mpath) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<PluginManifest>(&text) else {
            continue;
        };
        if !matches!(manifest.kind, PluginKind::Mcp) {
            continue;
        }
        let id = manifest.id.clone();
        let registered = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            mgr.register_plugin(manifest, repo),
        )
        .await;
        match registered {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(plugin = %id, "MCP registration failed: {e}");
                mgr.errors.insert(id, e);
            }
            Err(_) => {
                warn!(plugin = %id, "MCP registration timed out (8s)");
                mgr.errors
                    .insert(id, "连接超时（8 秒）— 端点无响应".to_string());
            }
        }
    }
    mgr
}

#[cfg(test)]
mod tests {
    use super::{sanitize_name, wire_tool_name};
    use crate::Plugin;

    /// The regression this whole module exists to prevent: an advertised tool
    /// name carrying a character outside `^[a-zA-Z0-9_-]{1,64}$` makes a strict
    /// upstream reject the ENTIRE request as `invalid_argument` — before the
    /// model runs, so the session simply cannot answer. The old spelling was
    /// `plugin_id:tool`.
    #[test]
    fn advertised_names_are_wire_legal() {
        let legal = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };

        assert!(legal(&wire_tool_name("ui-skills", "list_skills")));
        assert_eq!(wire_tool_name("ui-skills", "list_skills"), "ui-skills__list_skills");

        // The reported shape — a colon is the character that broke it.
        assert!(!wire_tool_name("ui-skills", "list_skills").contains(':'));

        // Servers name tools with dots, slashes and spaces too.
        for (id, tool) in [
            ("github.com/server", "repos.list"),
            ("my plugin", "read file"),
            ("srv", "path/to/thing"),
            ("plugin", "配置"),
        ] {
            let out = wire_tool_name(id, tool);
            assert!(legal(&out), "illegal advertised name: {out:?}");
        }
    }

    /// Namespacing must not leak into the 64-char cap — an over-long pair is
    /// trimmed at the namespace, keeping the tool half intact.
    #[test]
    fn long_names_are_trimmed_at_the_namespace() {
        let long_id = "p".repeat(80);
        let out = wire_tool_name(&long_id, "do_the_thing");
        assert_eq!(out.len(), 64);
        assert!(out.ends_with("__do_the_thing"));
    }

    /// The sanitized halves round-trip: what `sanitize_name` produces is what
    /// `wire_tool_name` joins, so resolution can rebuild the same key.
    #[test]
    fn sanitize_folds_only_forbidden_chars() {
        assert_eq!(sanitize_name("ui-skills_2"), "ui-skills_2");
        assert_eq!(sanitize_name("a:b/c.d e"), "a_b_c_d_e");
        // Empty halves must still yield a usable segment.
        assert_eq!(sanitize_name(""), "_");
    }

    /// Dispatch reaches the SERVER's own tool name — the advertised name is
    /// lossy, so calling the server with it would 404 on the MCP side.
    #[tokio::test]
    async fn dispatch_resolves_advertised_and_bare_names() {
        use std::sync::Arc;

        struct Fake {
            id: String,
            tools: Vec<&'static str>,
            called: std::sync::Mutex<Vec<String>>,
        }

        #[async_trait::async_trait]
        impl Plugin for Fake {
            fn id(&self) -> &str {
                &self.id
            }
            fn export_tools(&self) -> Vec<serde_json::Value> {
                self.tools
                    .iter()
                    .map(|t| serde_json::json!({"type":"function","function":{"name":t}}))
                    .collect()
            }
            async fn call_tool(&self, name: &str, _args: serde_json::Value) -> anyhow::Result<String> {
                self.called.lock().unwrap().push(name.to_string());
                Ok(format!("called {name}"))
            }
        }

        // A colon-bearing plugin id — the exact shape that used to break.
        let fake = Arc::new(Fake {
            id: "ui:skills".into(),
            tools: vec!["list_skills", "get_skill"],
            called: std::sync::Mutex::new(Vec::new()),
        });
        let advertised: Vec<String> = fake
            .export_tools()
            .iter()
            .map(|t| wire_tool_name(fake.id(), t["function"]["name"].as_str().unwrap()))
            .collect();
        assert_eq!(advertised, vec!["ui_skills__list_skills", "ui_skills__get_skill"]);

        // Drive the real registration/dispatch path via a manager.
        let mut mgr = super::PluginManager::new();
        let plugin: Arc<dyn Plugin> = fake.clone();
        for (real, adv) in [("list_skills", &advertised[0]), ("get_skill", &advertised[1])] {
            mgr.real_tool_names.insert(adv.clone(), real.to_string());
            mgr.tool_router.insert(adv.clone(), plugin.clone());
        }
        mgr.plugins.insert(fake.id().to_string(), plugin);
        mgr.enabled.insert(fake.id().to_string(), true);

        // Advertised spelling.
        assert_eq!(
            mgr.dispatch_tool_call("ui_skills__get_skill", serde_json::json!({})).await.unwrap(),
            "called get_skill"
        );
        // Bare spelling (a model that drops the namespace).
        assert_eq!(
            mgr.dispatch_tool_call("list_skills", serde_json::json!({})).await.unwrap(),
            "called list_skills"
        );
        // Legacy colon spelling from a pre-fix snapshot.
        assert_eq!(
            mgr.dispatch_tool_call("ui:skills:list_skills", serde_json::json!({})).await.unwrap(),
            "called list_skills"
        );
        // An unknown tool is refused, not silently misrouted.
        assert!(mgr.dispatch_tool_call("nope", serde_json::json!({})).await.is_err());
    }

    /// End-to-end on the real manager (not hand-populated maps): registration
    /// builds the router from `export_tools`, and `exported_tools` — the exact
    /// array the engine puts in the request — must be wire-legal and must be
    /// dispatchable by the name it advertises.
    #[tokio::test]
    async fn registration_round_trips_every_advertised_name() {
        use std::sync::Arc;

        struct Fake {
            id: String,
            tools: Vec<&'static str>,
        }

        #[async_trait::async_trait]
        impl Plugin for Fake {
            fn id(&self) -> &str {
                &self.id
            }
            fn export_tools(&self) -> Vec<serde_json::Value> {
                self.tools
                    .iter()
                    .map(|t| serde_json::json!({"type":"function","function":{"name":t}}))
                    .collect()
            }
            async fn call_tool(&self, name: &str, _a: serde_json::Value) -> anyhow::Result<String> {
                Ok(name.to_string())
            }
        }

        // Two plugins, one with a colon in its id AND its tool names — every
        // character the wire forbids, in the place that used to break it.
        let mut mgr = super::PluginManager::new();
        for (id, tools) in [
            ("ui:skills", vec!["list_skills", "get_skill"]),
            ("fs.reader", vec!["read/file", "list.dir"]),
        ] {
            let p: Arc<dyn Plugin> = Arc::new(Fake { id: id.into(), tools });
            mgr.plugins.insert(id.into(), p.clone());
            mgr.enabled.insert(id.into(), true);
            for t in p.export_tools() {
                let real = t["function"]["name"].as_str().unwrap().to_string();
                let adv = wire_tool_name(id, &real);
                mgr.real_tool_names.insert(adv.clone(), real);
                mgr.tool_router.insert(adv, p.clone());
            }
        }

        let advertised = mgr.exported_tools();
        assert_eq!(advertised.len(), 4);
        for t in &advertised {
            let name = t["function"]["name"].as_str().unwrap();
            assert!(
                !name.is_empty()
                    && name.len() <= 64
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "the engine would send an illegal function.name: {name:?}"
            );
            // …and the advertised name is actually callable.
            assert!(mgr.dispatch_tool_call(name, serde_json::json!({})).await.is_ok());
        }
        assert!(advertised
            .iter()
            .any(|t| t["function"]["name"] == "ui_skills__list_skills"));

        // Disabling purges both maps — no phantom tool survives.
        mgr.disable("ui:skills");
        assert_eq!(mgr.exported_tools().len(), 2);
        assert!(mgr
            .dispatch_tool_call("ui_skills__list_skills", serde_json::json!({}))
            .await
            .is_err());
    }
}
