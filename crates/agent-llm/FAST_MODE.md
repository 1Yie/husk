# Model-level Fast profiles

Fast is an opt-in request setting, independent of thinking. A model supports it
only when its effective configuration contains `fastMode` (`fast_mode` is also
accepted). No model names, suffixes, or provider capabilities are guessed.

```json
{
  "id": "logical-model",
  "fastMode": {
    "mode": "priority"
  }
}
```

Modes:

- `priority`: sends `service_tier: "priority"`.
- `speed`: sends `speed: "fast"`. On Anthropic, also merges
  `fast-mode-2026-02-01` into `anthropic-beta`, preserving existing flags.
- `model`: requires a nonempty `model` wire id, for example an explicitly
  configured turbo/highspeed variant. The logical id does not change.
- `custom`: requires a nonempty `params` object or `headers` map.

All modes accept optional `params` and `headers` overlays. Params shallowly
override the mode defaults and ordinary sampling parameters. Header names are
case-insensitive. Invalid header names/values, non-object params, and incomplete
profiles fail configuration parsing/provider construction with an error.
`model` is only valid in `model` mode.
Model routing must use that mode, not a `params.model` override, so opaque
Responses history stays scoped to the model that actually produced it.

```json
{
  "id": "logical-model",
  "fastMode": {
    "mode": "model",
    "model": "explicit-highspeed-variant",
    "params": { "service_tier": "priority" },
    "headers": { "x-gateway-fast": "enabled" }
  }
}
```

A `modelOverrides` entry without `fastMode` preserves the base profile. When
present, it replaces the entire profile: fields from different modes are never
accidentally mixed. Remove the configured profile to remove the capability.

Existing factory constructors leave Fast off. A conversation may opt in using:

```rust,ignore
ProviderFactory::build_with_fast_mode(&config, observer, conversation, true)?;
```

Direct callers may use `RigProvider::new(...)?.with_fast_mode(true)`. Off restores
ordinary requests; models without a profile remain unchanged even when enabled.
The resolved wire model is also the Responses replay identity, so switching
between ordinary and mapped Fast models cannot replay the other model's opaque
items. Effective route headers also participate in that identity, preventing
replay across gateway accounts selected by a Fast profile.
Responses overlays are applied after the SDK's typed conversion, for HTTP
and WebSockets alike. App-only `fast_mode`/`fastMode` keys are not sent upstream.

Anthropic protocol reference:
<https://platform.claude.com/docs/en/build-with-claude/fast-mode>
(verified during implementation). Fast availability, account permissions,
pricing, and support for any supplied wire id remain deployment-specific.
