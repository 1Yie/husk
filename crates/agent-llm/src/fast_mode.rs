//! Model-local wire adaptation. Logical ids remain in the app; only these
//! resolved settings cross the transport boundary.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::config::{FastMode, ModelConfig};

pub(crate) struct WireSettings {
    pub(crate) model: String,
    pub(crate) sampling: Map<String, Value>,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) fast_params: Map<String, Value>,
}

impl WireSettings {
    pub(crate) fn resolve(
        model: &str,
        sampling: Option<&Value>,
        headers: &HashMap<String, String>,
        config: Option<&ModelConfig>,
        enabled: bool,
        anthropic: bool,
    ) -> Self {
        let mut wire = Self {
            model: model.to_string(),
            sampling: sampling
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            headers: HashMap::new(),
            fast_params: Map::new(),
        };
        // HTTP names are case-insensitive, including across overlay layers.
        for layer in [Some(headers), config.and_then(|c| c.headers.as_ref())]
            .into_iter()
            .flatten()
        {
            for (name, value) in layer {
                wire.headers
                    .insert(name.to_ascii_lowercase(), value.clone());
            }
        }
        if let Some(fast) = config
            .and_then(|c| c.fast_mode.as_ref())
            .filter(|_| enabled)
        {
            match fast.mode {
                FastMode::Priority => {
                    wire.fast_params
                        .insert("service_tier".into(), json!("priority"));
                }
                FastMode::Speed => {
                    wire.fast_params.insert("speed".into(), json!("fast"));
                }
                FastMode::Model => {
                    // Validated on deserialize and construction; never guess
                    // suffixes such as turbo/highspeed from the logical name.
                    wire.model = fast.model.clone().expect("validated Fast model");
                }
                FastMode::Custom => {}
            }
            if let Some(params) = fast.params.as_ref().and_then(Value::as_object) {
                wire.fast_params.extend(params.clone());
            }
            let mut beta = wire
                .headers
                .get("anthropic-beta")
                .cloned()
                .unwrap_or_default();
            if let Some(headers) = &fast.headers {
                for (name, value) in headers {
                    if name.eq_ignore_ascii_case("anthropic-beta")
                        && anthropic
                        && fast.mode == FastMode::Speed
                    {
                        beta.push(',');
                        beta.push_str(value);
                    }
                    wire.headers
                        .insert(name.to_ascii_lowercase(), value.clone());
                }
            }
            if anthropic && fast.mode == FastMode::Speed {
                // https://platform.claude.com/docs/en/build-with-claude/fast-mode
                // Research-preview fast speed requires this beta. Preserve
                // existing beta capabilities instead of replacing the header.
                let mut flags: Vec<&str> = Vec::new();
                for flag in beta
                    .split(',')
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .chain(std::iter::once("fast-mode-2026-02-01"))
                {
                    if !flags.contains(&flag) {
                        flags.push(flag);
                    }
                }
                wire.headers
                    .insert("anthropic-beta".into(), flags.join(","));
            }
            wire.sampling.extend(wire.fast_params.clone());
        }
        // App-only toggles are never API parameters, including if an old
        // sampling config accidentally contains them.
        wire.sampling.remove("fast_mode");
        wire.sampling.remove("fastMode");
        wire
    }
}
