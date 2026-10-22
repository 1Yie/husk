//! Delegated children's model routing.
//!
//! The kernel's `ChildRoute` carries one provider and a table of model ids a
//! child may run on. This module widens both to every configured provider:
//! the table holds the parent provider's bare ids plus a `provider/model` key
//! for every provider's models, and the route's provider is a
//! [`RoutedProvider`] that sends each key to the backend that serves it.
//!
//! On top of that sits the global subagent default model: when set, it
//! replaces the route's own model, so an unnamed child (or a manifest without
//! `model:`) runs on the cheap default instead of the parent's model.
//! Precedence: manifest `model:` > subagent default > parent model.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_kernel::tools::delegate::{ChildModel, ChildRoute};
use agent_llm::types::ChatMessage;
use agent_llm::{AppConfig, BoxStream, Capabilities, LlmProvider, ModelParams, ProviderConfig};
use agent_llm::{ProviderFactory, StreamChunk};
use async_trait::async_trait;

/// The ids a provider serves: its declared models, or its `default_model`
/// when it lists none.
fn declared_models(pcfg: &ProviderConfig) -> Vec<String> {
    let ids = pcfg.model_ids();
    if ids.is_empty() {
        pcfg.default_model.iter().cloned().collect()
    } else {
        ids
    }
}

fn child_model(pcfg: &ProviderConfig, id: &str) -> ChildModel {
    let d = pcfg.model_opts(id);
    ChildModel {
        context_window: d.context_window.unwrap_or(256_000) as usize,
        thinking_level_map: d.thinking_level_map.clone(),
        model_params: ProviderFactory::model_params_from(&d),
        model_input: d.input.clone(),
    }
}

/// The key a child route uses for `(provider, model)`: the bare id when the
/// parent's provider serves it, `provider/model` otherwise.
pub fn route_key(parent_provider: &str, provider: &str, model: &str) -> String {
    if provider == parent_provider {
        model.to_string()
    } else {
        format!("{provider}/{model}")
    }
}

/// Every model a child may name, keyed as the manifest writes it, plus where
/// each non-parent key is served: `key → (provider name, real model id)`.
fn model_table(
    cfg: &AppConfig,
    parent_provider: &str,
) -> (
    HashMap<String, ChildModel>,
    HashMap<String, (String, String)>,
) {
    let mut models = HashMap::new();
    let mut targets = HashMap::new();
    if let Some(pcfg) = cfg.providers.get(parent_provider) {
        for id in declared_models(pcfg) {
            models.insert(id.clone(), child_model(pcfg, &id));
        }
    }
    for (pname, pcfg) in &cfg.providers {
        for id in declared_models(pcfg) {
            let key = format!("{pname}/{id}");
            // A parent model whose own id looks qualified keeps its meaning.
            if models.contains_key(&key) {
                continue;
            }
            models.insert(key.clone(), child_model(pcfg, &id));
            targets.insert(key, (pname.clone(), id));
        }
    }
    (models, targets)
}

/// Point `route` at every configured provider, then apply the subagent
/// default `(provider, model)` when it names a configured model.
///
/// `parent` is the parent session's own provider instance (unwrapped), and
/// the route's model fields must already describe the parent's model — they
/// stay as they are when no default applies. `conversation` is the parent
/// session's routing identity: providers built here on first use advertise it
/// too, so a delegated child's requests stay in the same conversation for the
/// gateway's routing and prompt cache.
pub fn install(
    route: &mut ChildRoute,
    cfg: &AppConfig,
    parent_name: &str,
    parent: Arc<dyn LlmProvider>,
    default: Option<&(String, String)>,
    thinking: Option<&str>,
    conversation: &str,
) {
    let (models, targets) = model_table(cfg, parent_name);
    let providers = cfg
        .providers
        .iter()
        .map(|(n, p)| (n.clone(), p.clone()))
        .collect();
    route.provider = Arc::new(RoutedProvider {
        parent,
        parent_name: parent_name.to_string(),
        targets,
        providers,
        conversation: conversation.to_string(),
        built: Mutex::new(HashMap::new()),
    });
    route.models = models;
    // A manifest `thinking:` is applied per call and wins over this; here is
    // the level a child runs at when its own file names none. Empty means
    // "inherit the parent's", which is what the route already carries. Set
    // before the early return below — the level is independent of the model.
    if let Some(level) = thinking.map(str::trim).filter(|l| !l.is_empty()) {
        route.thinking_level = Some(level.to_string());
    }
    let Some((provider, model)) = default else {
        return;
    };
    let key = route_key(parent_name, provider, model);
    match route.models.get(&key) {
        Some(m) => {
            route.model = key;
            route.context_window = m.context_window;
            route.thinking_level_map = m.thinking_level_map.clone();
            route.model_params = m.model_params.clone();
            route.model_input = m.model_input.clone();
        }
        None => tracing::warn!(
            %provider, %model,
            "subagent default model is not configured; children follow the parent"
        ),
    }
}

/// The children's provider: a table key served by another provider goes
/// there under its real id; anything else goes to the parent unchanged.
pub struct RoutedProvider {
    parent: Arc<dyn LlmProvider>,
    parent_name: String,
    targets: HashMap<String, (String, String)>,
    providers: HashMap<String, ProviderConfig>,
    /// The parent session's routing identity, handed to every provider this
    /// route builds so a child never starts its own cache line.
    conversation: String,
    /// Other providers, built on first use — a session that never delegates
    /// to them never resolves their keys.
    built: Mutex<HashMap<String, Arc<dyn LlmProvider>>>,
}

impl RoutedProvider {
    fn target(&self, model: &str) -> anyhow::Result<(Arc<dyn LlmProvider>, String)> {
        let Some((pname, real)) = self.targets.get(model) else {
            return Ok((self.parent.clone(), model.to_string()));
        };
        if *pname == self.parent_name {
            return Ok((self.parent.clone(), real.clone()));
        }
        let mut built = self.built.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(p) = built.get(pname) {
            return Ok((p.clone(), real.clone()));
        }
        let pcfg = self
            .providers
            .get(pname)
            .ok_or_else(|| anyhow::anyhow!("provider `{pname}` is not in the config"))?;
        let p = ProviderFactory::build_with_conversation(pcfg, None, Some(&self.conversation))
            .map_err(|e| anyhow::anyhow!("provider unavailable: `{pname}`: {e}"))?;
        built.insert(pname.clone(), p.clone());
        Ok((p, real.clone()))
    }
}

#[async_trait]
impl LlmProvider for RoutedProvider {
    fn id(&self) -> &'static str {
        self.parent.id()
    }

    /// The parent's table. Every built-in wire agrees on the bits the engine
    /// reads (structured tool calls, reasoning); the per-wire differences are
    /// applied inside each adapter's own `chat_stream`.
    fn capabilities(&self) -> Capabilities {
        self.parent.capabilities()
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let (provider, real) = self.target(model)?;
        provider
            .chat_stream(
                &real,
                messages,
                tools,
                temperature,
                reasoning_effort,
                params,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        toml::from_str(
            r#"
            [providers.main]
            kind = "anthropic"
            base_url = "https://example.invalid"
            models = ["big"]

            [providers.cheap]
            kind = "openai_completions"
            base_url = "https://example.invalid"
            models = [{ id = "small", context_window = 64000 }]
            "#,
        )
        .expect("test config")
    }

    fn route() -> ChildRoute {
        ChildRoute {
            provider: Arc::new(agent_llm::provider::UnconfiguredProvider),
            model: "big".into(),
            temperature: 1.0,
            context_window: 200_000,
            thinking_level: None,
            thinking_level_map: None,
            model_params: ModelParams::default(),
            model_input: Vec::new(),
            models: HashMap::new(),
        }
    }

    #[test]
    fn table_keys_parent_bare_and_every_provider_qualified() {
        let (models, targets) = model_table(&cfg(), "main");
        assert!(models.contains_key("big"));
        assert!(models.contains_key("main/big"));
        assert!(models.contains_key("cheap/small"));
        assert!(!models.contains_key("small"));
        assert_eq!(
            targets.get("cheap/small"),
            Some(&("cheap".to_string(), "small".to_string()))
        );
    }

    #[test]
    fn default_replaces_the_route_model_and_its_settings() {
        let mut r = route();
        let default = ("cheap".to_string(), "small".to_string());
        install(
            &mut r,
            &cfg(),
            "main",
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            Some(&default),
            None,
            "1:/tmp/workspace",
        );
        assert_eq!(r.model, "cheap/small");
        assert_eq!(r.context_window, 64_000);
    }

    #[test]
    fn default_on_the_parent_provider_uses_the_bare_id() {
        let mut r = route();
        r.model = "other".into();
        let default = ("main".to_string(), "big".to_string());
        install(
            &mut r,
            &cfg(),
            "main",
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            Some(&default),
            None,
            "1:/tmp/workspace",
        );
        assert_eq!(r.model, "big");
    }

    /// A child may keep the parent's model and still think less. Blank is
    /// "inherit", not "clear" — the settings row's empty choice must not
    /// silently drop a level the parent session is running at.
    #[test]
    fn a_subagent_thinking_default_overrides_the_parents_level() {
        let mut r = route();
        r.thinking_level = Some("high".into());
        install(
            &mut r,
            &cfg(),
            "main",
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            None,
            Some("low"),
            "1:/tmp/workspace",
        );
        assert_eq!(r.thinking_level.as_deref(), Some("low"));

        let mut r = route();
        r.thinking_level = Some("high".into());
        install(
            &mut r,
            &cfg(),
            "main",
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            None,
            Some("   "),
            "1:/tmp/workspace",
        );
        assert_eq!(r.thinking_level.as_deref(), Some("high"));
    }

    #[test]
    fn unknown_default_leaves_the_parent_model() {
        let mut r = route();
        let default = ("cheap".to_string(), "gone".to_string());
        install(
            &mut r,
            &cfg(),
            "main",
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            Some(&default),
            None,
            "1:/tmp/workspace",
        );
        assert_eq!(r.model, "big");
        assert_eq!(r.context_window, 200_000);
    }
}
