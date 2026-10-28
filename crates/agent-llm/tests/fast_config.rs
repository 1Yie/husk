use agent_llm::{
    AppConfig, FastMode, FastModeConfig, ModelConfig, ModelOverride, ProviderConfig,
    ProviderFactory,
};
use serde_json::{json, Value};

fn profile(value: Value) -> FastModeConfig {
    serde_json::from_value(value).unwrap()
}

#[test]
fn fast_profiles_roundtrip_and_defaults_stay_absent() {
    let bare: ModelConfig = serde_json::from_value(json!({"id":"base"})).unwrap();
    assert!(!bare.supports_fast_mode());
    let serialized = serde_json::to_value(&bare).unwrap();
    assert!(serialized.get("fastMode").is_none());
    assert!(serialized.get("fast_mode").is_none());
    let bare_override: ModelOverride = serde_json::from_value(json!({})).unwrap();
    assert!(bare_override.fast_mode.is_none());

    for value in [
        json!({"mode":"priority"}),
        json!({"mode":"speed", "headers":{"x-gateway":"fast"}}),
        json!({"mode":"model", "model":"explicit-turbo", "params":{"top_p":0.8}}),
        json!({"mode":"custom", "params":{"latency":"low"}}),
        json!({"mode":"custom", "headers":{"x-fast":"1"}}),
    ] {
        for key in ["fastMode", "fast_mode"] {
            let model: ModelConfig =
                serde_json::from_value(json!({"id":"base", (key): value.clone()})).unwrap();
            assert!(model.supports_fast_mode());
            assert_eq!(serde_json::to_value(&model).unwrap()["fastMode"], value);
            let toml = toml::to_string(&model).unwrap();
            assert_eq!(toml::from_str::<ModelConfig>(&toml).unwrap(), model);
            let override_: ModelOverride =
                serde_json::from_value(json!({(key): value.clone()})).unwrap();
            assert_eq!(serde_json::to_value(override_).unwrap()["fastMode"], value);
        }
    }
}

#[test]
fn overrides_preserve_or_replace_the_whole_fast_profile() {
    let base: ModelConfig = serde_json::from_value(json!({
        "id":"base", "reasoning":true,
        "fastMode":{"mode":"model", "model":"turbo", "params":{"old":true}, "headers":{"x-old":"1"}}
    }))
    .unwrap();
    assert_eq!(base.with_override(&ModelOverride::default()), base);
    let override_: ModelOverride = serde_json::from_value(json!({
        "fast_mode":{"mode":"priority", "params":{"new":true}}
    }))
    .unwrap();
    let effective = base.with_override(&override_);
    assert_eq!(
        effective.fast_mode.as_ref().unwrap().mode,
        FastMode::Priority
    );
    assert_eq!(effective.fast_mode.as_ref().unwrap().model, None);
    assert_eq!(effective.fast_mode.as_ref().unwrap().headers, None);
    assert_eq!(
        effective.fast_mode.as_ref().unwrap().params,
        Some(json!({"new":true}))
    );
    assert_eq!(effective.reasoning, Some(true));
    let cfg: ProviderConfig = serde_json::from_value(json!({
        "kind":"openai_completions", "base_url":"http://localhost",
        "models":[base], "modelOverrides":{"base":override_}
    }))
    .unwrap();
    assert_eq!(cfg.model_opts("base"), effective);
}

#[test]
fn invalid_profiles_have_actionable_errors_even_inside_model_entries() {
    for (value, message) in [
        (json!({}), "mode"),
        (json!({"mode":"guess"}), "variant"),
        (json!({"mode":"model"}), "nonempty model id"),
        (json!({"mode":"model", "model":"  "}), "nonempty model id"),
        (json!({"mode":"priority", "model":"turbo"}), "only valid"),
        (json!({"mode":"custom"}), "requires nonempty"),
        (
            json!({"mode":"custom", "params":{}, "headers":{}}),
            "requires nonempty",
        ),
        (json!({"mode":"priority", "params":[]}), "JSON object"),
        (json!({"mode":"priority", "params":"fast"}), "JSON object"),
        (
            json!({"mode":"custom", "params":{"model":"other-model"}}),
            "model routing",
        ),
        (
            json!({"mode":"custom", "params":{"fast_mode":true}}),
            "app-only",
        ),
        (
            json!({"mode":"priority", "headers":{"bad name":"v"}}),
            "header name",
        ),
        (
            json!({"mode":"speed", "headers":{"x-fast":"v\r\ninjected: true"}}),
            "header value",
        ),
    ] {
        let error = serde_json::from_value::<ProviderConfig>(json!({
            "kind":"openai_completions", "base_url":"http://localhost",
            "models":[{"id":"base", "fastMode":value}]
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains(message), "{message}: {error}");
    }
}

#[test]
fn rust_created_invalid_profiles_fail_provider_construction_even_when_off() {
    let cfg = ProviderConfig {
        models: vec![ModelConfig {
            id: "base".into(),
            fast_mode: Some(FastModeConfig {
                mode: FastMode::Custom,
                model: None,
                params: Some(json!(3)),
                headers: None,
            }),
            ..Default::default()
        }
        .into()],
        ..Default::default()
    };
    let error = ProviderFactory::build(&cfg).err().expect("invalid config");
    assert!(error
        .to_string()
        .contains("model \"base\": fastMode params must be a JSON object"));
}

#[test]
fn save_roundtrips_and_removes_fast_profiles_without_resurrecting_aliases() {
    for extension in ["toml", "json"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("config.{extension}"));
        let mut cfg: AppConfig = serde_json::from_value(json!({
            "providers":{"p":{"kind":"openai_completions", "base_url":"http://localhost",
                "models":[{"id":"base", "fast_mode":{"mode":"priority"}}],
                "modelOverrides":{"base":{"fastMode":{"mode":"model","model":"turbo"}}}
            }}
        }))
        .unwrap();
        AppConfig::save_to(&path, &cfg).unwrap();
        let loaded = AppConfig::load(dir.path(), Some(&path)).unwrap();
        assert_eq!(
            loaded.providers["p"].model_opts("base").fast_mode,
            Some(profile(json!({"mode":"model","model":"turbo"})))
        );
        if extension == "toml" {
            let saved = std::fs::read_to_string(&path)
                .unwrap()
                .replace("fastMode", "fast_mode");
            std::fs::write(&path, saved).unwrap();
        }
        let provider = cfg.providers.get_mut("p").unwrap();
        provider.models = vec![ModelConfig {
            id: "base".into(),
            ..Default::default()
        }
        .into()];
        provider
            .model_overrides
            .as_mut()
            .unwrap()
            .get_mut("base")
            .unwrap()
            .fast_mode = None;
        AppConfig::save_to(&path, &cfg).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            !saved.contains("fastMode") && !saved.contains("fast_mode"),
            "{saved}"
        );
        assert!(
            !AppConfig::load(dir.path(), Some(&path)).unwrap().providers["p"]
                .model_opts("base")
                .supports_fast_mode()
        );
    }
}
