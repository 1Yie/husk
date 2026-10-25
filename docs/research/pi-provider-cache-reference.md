# Pi `packages/ai` provider / prompt-cache reference (pinned upstream)

Research note. Scope: **only** upstream Pi source. No Husk code read or modified, no Rust builds,
no other agents launched.

- Upstream: <https://github.com/badlogic/pi-mono>
- **Pinned commit SHA: `38ff99177fba21e9c57a670f4e95adcf407c99c7`** (`refs/heads/main` at fetch time)
- Reusable checkout: **`/tmp/husk-pi-reference-1792913212-2744089`** (shallow, detached at the SHA)
- Package under study: `packages/ai` (`@earendil-works/pi-ai` v1.4.0)

Source links below use
`https://github.com/badlogic/pi-mono/blob/38ff99177fba21e9c57a670f4e95adcf407c99c7/<path>#L<line>`.

Tooling note: the checkout is remote TS, so no Serena index was available. All navigation was
done with targeted `read` / `grep` on the pinned checkout, per instructions.

---

## 1. File map (what actually implements the protocols)

| Concern | File |
| --- | --- |
| `openai-responses` transport | `packages/ai/src/api/openai-responses.ts` (453 L) |
| `openai-responses` message/tool/stream core (shared with azure/codex) | `packages/ai/src/api/openai-responses-shared.ts` (891 L) |
| `openai-completions` transport | `packages/ai/src/api/openai-completions.ts` (1728 L) |
| `anthropic-messages` transport | `packages/ai/src/api/anthropic-messages.ts` (1690 L) |
| `google-generative-ai` | `packages/ai/src/api/google-generative-ai.ts` + `google-shared.ts` |
| compat capability schema (zod) | `packages/ai/src/providers/compat-schema.ts` |
| message transformation / cross-provider normalization | `packages/ai/src/api/transform-messages.ts` |
| mid-conversation system message folding | `packages/ai/src/utils/transcript.ts` |
| `prompt_cache_key` clamping | `packages/ai/src/api/openai-prompt-cache.ts` |
| model / message types | `packages/ai/src/types.ts` |

There is **no** `openai-responses.ts`-equivalent "provider factory" in this package; the API string
on `Model` selects one of the `api/*.ts` `stream` / `streamSimple` implementations via
`packages/ai/src/api/lazy.ts`.

---

## 2. The six compatibility flags, item by item

All six live on `Model.compat` and are **per-model**, not per-API. Each transport re-reads them with
its own local default. Definitions are in
`packages/ai/src/providers/compat-schema.ts`; zod schemas are split per API, so the same flag name
can appear in more than one schema with different wording.

### 2.1 `supportsMidConvoEffort`

- **Defined:** `compat-schema.ts:399` (OpenAI-completions-ish schema), `:473`
  (Responses/Anthropic-ish schema), also listed in the shared optional block at `:552`.
- **Semantics (from the Responses schema description at `compat-schema.ts:402`):** *"Whether
  thinking level changes are sent as `configuration_update` input items instead of changing
  `reasoning.effort`, so a change keeps the prompt cache. Only used by the `openai-responses` API,
  and ignored with Sign in with ChatGPT on the `openai` provider."*
- **"Actual read locations":**
  - `api/openai-responses.ts:103` — default `false` in `getCompat()`.
  - `api/openai-responses.ts:114` — inside `getConfigurationUpdateEffort()`, the only gate.
  - `api/anthropic-messages.ts:1148`, `:1203`, `:1253` — Anthropic has its own consumption of the
    same flag name (see 2.7).
- **Protocol scope:** `openai-responses` (and by sharing, `azure-openai-responses`,
  `openai-codex-responses` — they reuse `OpenAIResponsesCompatSchema`). `openai-completions` does
  **not** reference it at all (grep-confirmed: no hit in `openai-completions.ts`).

### 2.2 `supportsMidConvoSystemMessages`

- **Defined:** `compat-schema.ts:303`, `:358`, `:480`, `:515`, `:544`.
- **Default:** `false` in `api/openai-responses.ts:94`.
- **"Actual read locations":**
  - `api/openai-responses.ts:94` (default), `:160` (`resolveTranscript(...)`), `:350` (passed into
    `convertResponsesMessages` as `supportsMidConvoSystemMessages`).
  - `api/openai-responses-shared.ts:130` (option field), `:172` (gates `getResponsesToolSupport`),
    `:198` (`resolveTranscript`).
  - `api/openai-completions.ts:181`, `:188` (re-exported into its own option type), `:316`
    (`resolveTranscript`), `:798` (combined with `supportsMidConvoToolAdditions`).
  - `utils/transcript.ts:117-119` — the actual behavior: when the flag is **not** true, the
    transcript is passed through `collapseSystemMessages(context)`; when true, the context is used
    as-is so mid-conversation `system`/`developer` messages stay in place.
- **Protocol scope:** `openai-responses`, `openai-completions`. Anthropic has a same-named flag in
  its schema (`:358`, `:544`) and reads it at `anthropic-messages.ts:1199`.

### 2.3 `supportsMidConvoToolChanges`

- **Defined:** `compat-schema.ts:485` only. Description (`:488`): *"Whether the exact model accepts
  mid-conversation `tool_addition` and `tool_removal` blocks. Requires
  `supportsMidConvoSystemMessages`. The generated catalog enables this for capable models."*
- **Default:** `false` in `api/anthropic-messages.ts:235`.
- **Single read location:** `api/anthropic-messages.ts:1199`, in the conjunction
  `compat.supportsMidConvoSystemMessages && compat.supportsMidConvoToolChanges && initialTools.length > 0`.
- **Protocol scope: `anthropic-messages` only.** There is no `supportsMidConvoToolChanges` read in
  either OpenAI transport. The Responses-side analogue is `supportsAdditionalTools` /
  `supportsToolSearch` (see 2.8).

### 2.4 `forceAdaptiveThinking`

- **Defined:** `compat-schema.ts:453` (Anthropic schema).
- **No default constant**: every read site uses `=== true`, so the effective default is "not
  forced".
- **Read locations:** `api/anthropic-messages.ts:978`, `:1143`, `:1293`; plus a consumer outside
  `packages/ai` — `packages/coding-agent/src/core/cache-warmer.ts:57`.
- **Protocol scope: `anthropic-messages` only.**

### 2.5 `supportsTemperature`

- **Defined:** `compat-schema.ts:446` (Anthropic schema).
- **Default:** **`true`** — `api/anthropic-messages.ts:231`.
- **Read location:** `api/anthropic-messages.ts:1254` — gates whether `temperature` is emitted.
- **Protocol scope: `anthropic-messages` only.** Notably the `openai-responses` transport sends
  `params.temperature` with **no compat gate** (`openai-responses.ts:381-383`); only the ChatGPT
  sign-in path suppresses it.

### 2.6 `supportsStrictTools`

- **Defined:** `compat-schema.ts:466` (Anthropic schema).
- **Default:** **`false`** — `api/anthropic-messages.ts:233`.
- **Read locations:** `anthropic-messages.ts:1183` (passed to the tool conversion helper),
  `:1626` (parameter), `:1637` (feeds `resolveJsonSchemaStrictSampling`).
- **Protocol scope: `anthropic-messages` only.** The OpenAI transports use a different flag,
  `supportsStrictMode` (Responses default `false`, `openai-responses.ts:97`).

### 2.7 Summary table

| Flag | Defined (compat-schema.ts) | Default site | Default | Protocols that read it |
| --- | --- | --- | --- | --- |
| `supportsMidConvoEffort` | `:399`, `:473`, `:552` | `openai-responses.ts:103` | `false` | `openai-responses` (+ azure/codex via shared schema), `anthropic-messages` |
| `supportsMidConvoSystemMessages` | `:303`, `:358`, `:480`, `:515`, `:544` | `openai-responses.ts:94` | `false` | `openai-responses`, `openai-completions`, `anthropic-messages` |
| `supportsMidConvoToolChanges` | `:485` | `anthropic-messages.ts:235` | `false` | `anthropic-messages` only |
| `forceAdaptiveThinking` | `:453` | none (`=== true` checks) | effectively `false` | `anthropic-messages` only |
| `supportsTemperature` | `:446` | `anthropic-messages.ts:231` | **`true`** | `anthropic-messages` only |
| `supportsStrictTools` | `:466` | `anthropic-messages.ts:233` | `false` | `anthropic-messages` only |

Flags *not* in the user's list but adjacent and relevant to caching/tool churn:
`supportsDeveloperRole` (Responses default `true`), `supportsAdditionalTools` (`false`),
`supportsToolSearch` (`false`), `supportsExplicitPromptCacheMode` (`false`),
`supportsLongCacheRetention` (**`true`** via `openai-responses.ts:96`),
`supportsMaxOutputTokens` (`true`), `supportsOpenAIGrammarTools` (`false`) — all resolved in
`getCompat()` at `openai-responses.ts:91-105`.

---

## 3. `configuration_update` — it exists, and only on Responses

The item type is real and produced by Pi, not by the model.

- Constructed at `api/openai-responses-shared.ts:266-272`:
  ```ts
  const pushConfigurationUpdate = (effort: string): void => {
      messages.push({
          type: "configuration_update",
          reasoning: { effort: effort as ... },
      } satisfies ResponseConfigurationUpdateItemParam);
  };
  ```
  The param type is imported from the `openai` SDK (`ResponseConfigurationUpdateItemParam`,
  `openai-responses-shared.ts:5`), i.e. it is a genuine Responses input item.
- **Two emission points** in `convertResponsesMessages`:
  1. `:402-409` — immediately **before** each earlier assistant message that carries
     `providerThinkingLevel`, and only when `isSameModel` (same provider **and** same API **and**
     same `model.id`). Each item replays that turn's recorded level.
  2. `:426` — one final item at the very end of `input`, carrying `options.configurationUpdateEffort`
     (the level for the reply being generated).
- The request-level effort is then **frozen**: `openai-responses.ts:396-400`
  ```ts
  const effort = configurationUpdateEffort === undefined ? requestedEffort : CONFIGURATION_UPDATE_REQUEST_EFFORT;
  ```
  with `CONFIGURATION_UPDATE_REQUEST_EFFORT = "medium"` (`openai-responses.ts:43`).
- Gating (`getConfigurationUpdateEffort`, `openai-responses.ts:107-121`): requires
  `compat.supportsMidConvoEffort === true`, a non-empty `options.reasoningEffort`, and **not**
  Sign in with ChatGPT. The ChatGPT check is `isChatGPTSignIn()` (`:49-56`): provider `openai`,
  `baseUrl === "https://api.openai.com/v1"`, and an API key not starting with `sk-`.
  Comment at `:116`: "Sign in with ChatGPT rejects configuration_update items."
- The effort string is mapped through `model.thinkingLevelMap` first
  (`openai-responses.ts:120`), so the item carries the provider's own vocabulary.
- Provenance recorded on the reply for later replay:
  `openai-responses.ts:172` — `...(configurationUpdateEffort === undefined ? {} : { providerThinkingLevel: configurationUpdateEffort })`,
  typed at `types.ts:594`.

### 3.1 Why: prefix stability (the actual cache argument in-source)

Doc comment `openai-responses-shared.ts:133-142`:

> Effort for the reply being generated. When set, the thinking level is sent as
> `configuration_update` items instead of a changing `reasoning.effort`, which would invalidate the
> prompt cache. Each item sets the effort from that point on: one goes before every earlier reply
> with the level recorded on it (`providerThinkingLevel`), and one with this effort goes at the end.
> Earlier input therefore never changes. OpenAI documents the item before user messages only. After
> tool results it was verified on gpt-6-luna (October 2026) to be accepted and to apply to the next
> reply.

So the design intent is explicit: **move effort out of the request prefix and into append-only
input items**.

### 3.2 Effect of an effort change on the input prefix

With `supportsMidConvoEffort: true`, changing the thinking level between turns:

- Request prefix (`input[0..n-1]`) is **unchanged**: the replayed `configuration_update` items and
  the terminal one are positionally stable, and `reasoning.effort` stays `"medium"` for every
  request. Only the *value inside the last appended item* differs.
- With the flag `false`, the level is carried in `reasoning.effort` (`openai-responses.ts:396-410`),
  i.e. in request-level parameters — outside `input`, so it is not literally part of the token
  prefix, but Pi's own comment treats a changing `reasoning.effort` as cache-invalidating.

---

## 4. Tool changes and the input prefix

`getResponsesToolSupport` (`openai-responses-shared.ts:167-176`):

```ts
if (options.supportsMidConvoSystemMessages && options.supportsAdditionalTools) return "changes";
return options.supportsToolSearch ? "additions" : "none";
```

- **`"changes"`**: every tool is declared inside `additional_tools` input items rather than the
  top-level `tools` array. `appendSystemToolAdditions` (`:230-265`) emits, for each system message
  that anchors tool churn, an `additional_tools` item with `role: "developer"`, preceded (after the
  first one) by an `incremental tools update` text notice
  (`renderIncrementalToolsNotice`, `:182-186`). Removals cannot be expressed structurally — they are
  announced as text via `renderToolRemovalNotice` (`:188-190`) inside the system message body.
- **`"additions"`**: a synthetic `tool_search_call` / `tool_search_output` pair
  (`:243-265`) with `call_id = pi_tool_load_<shortHash(seed:names)>`.
- **`"none"`**: top-level `params.tools` only (`openai-responses.ts:390-395`).

Cache consequence as designed in-source (`openai-responses-shared.ts:161-166`):
`additional_tools` **accumulate and replace same-name definitions but never remove**, so new tools
are appended instead of rewriting an earlier tool block. `resolveTranscriptTools(...)` decides which
system messages are "folded" vs "anchoring" (`foldedSystemMessages`, used at `:290`).
Top-level `tools` is only used for the non-churn case; comment at `:161-163` notes removing a
top-level tool is impossible, hence the append-only scheme.

## 5. System-message changes and the input prefix

- `resolveTranscript` (`utils/transcript.ts:117-119`): if `supportsMidConvoSystemMessages` is not
  true → `collapseSystemMessages(context)`, i.e. later system text is merged upward (prefix-stable by
  construction, since nothing is appended mid-list). If true → context passes through unchanged, so
  a new system message appears at the tail.
- Role choice: `openai-responses-shared.ts:273-274` — `instructionRole = model.reasoning && compat?.supportsDeveloperRole !== false ? "developer" : "system"`.
- A non-leading system message is rendered via `renderSystemMessageUpdate(msg)` and appended at its
  transcript position (`:288-310`), so the *tail* grows; earlier items are untouched.
- Anthropic analogue: `anthropic-messages.ts:1199` requires both
  `supportsMidConvoSystemMessages` **and** `supportsMidConvoToolChanges` and a non-empty initial tool
  list before emitting mid-conversation tool blocks.

---

## 6. Reasoning item replay: `id` / `encrypted_content` / `phase`

- **Storage is whole-item JSON in a string signature.** On the way in
  (`openai-responses-shared.ts:336-339`):
  ```ts
  if (block.type === "thinking") {
      if (block.thinkingSignature) {
          const reasoningItem = JSON.parse(block.thinkingSignature) as ResponseReasoningItem;
          output.push(reasoningItem);
      }
  }
  ```
  The entire `ResponseReasoningItem` — including `id`, `encrypted_content`, `summary` — is serialized
  into `thinkingSignature` and pushed back verbatim. Nothing is regenerated.
- On the way out, the terminal response's reasoning items are used to **backfill**
  `encrypted_content` when `response.output_item.done` omitted it (Azure):
  `openai-responses-shared.ts:606-620` — comment: "persisted reasoning signature from the terminal
  response to keep `store:false`"; skips non-`reasoning` items and items lacking
  `encrypted_content`, and does not overwrite an already-stored value.
- **`phase`** belongs to assistant *text* messages, not reasoning items:
  `encodeTextSignatureV1(item.id, item.phase)` (`openai-responses-shared.ts:55`, set at `:...` in the
  stream handler, `:...` region around 700-760) and `parseTextSignature`
  (`:61`) on replay, used at `:346-364` when re-emitting `{type:"message", role:"assistant", id: msgId, phase}`.
  So `phase` round-trips through `TextContent.textSignature`, `id` through `thinkingSignature`
  (reasoning) or `textSignature` (message).
- **Tool-call item ids** round-trip through a composite `callId|itemId` string
  (`openai-responses-shared.ts:218-226`), with `fc_` forced and 64-char clamping; cross-provider ids
  become `fc_<shortHash>`, and non-matching prefixes / different-model replay drop the id entirely
  (`:361-372`).
- Since `store: false` is always sent (`openai-responses.ts:370`), replay is fully client-side: no
  server-side conversation state is relied on.

---

## 7. Request fields: `store` / `include` / `previous_response_id` / `prompt_cache_*`

From `buildParams` (`openai-responses.ts:357-...`):

| Field | Value / rule | Line |
| --- | --- | --- |
| `store` | always `false` | `:370` |
| `prompt_cache_key` | `clampOpenAIPromptCacheKey(options.sessionId)`, or `undefined` when `cacheRetention === "none"` | `:367` |
| `prompt_cache_retention` | `"24h"` iff `cacheRetention === "long"` **and** `supportsLongCacheRetention` **and** *not* `supportsExplicitPromptCacheMode`; suppressed entirely on ChatGPT sign-in | `:368`, `:123-129` |
| `prompt_cache_options` | only when `supportsExplicitPromptCacheMode`: `{mode:"explicit"}` for `cacheRetention==="none"`, `{ttl:"30m"}` for `"long"` | `:369`, `:131-139` |
| `include` | `["reasoning.encrypted_content"]` when reasoning effort is set; also forced for `model.provider === "xai"` | `:407`, `:413` |
| `previous_response_id` | **not sent by this transport.** Only `api/openai-codex-responses.ts` uses it (WebSocket continuation) | `:...` (absent); codex at `:108`, `:1491` |
| `temperature` | forwarded unconditionally except on ChatGPT sign-in | `:381-383` |
| `max_output_tokens` | `max(maxTokens, 16)` when `supportsMaxOutputTokens` | `:377-379` |
| `reasoning` | `{effort, summary: options.reasoningSummary || "auto"}`; else `{effort: thinkingLevelMap.off ?? "none"}` unless provider is `github-copilot` or `thinkingLevelMap.off === null` | `:396-410` |

`resolveCacheRetention` (`:81-89`): explicit option wins; else `PI_CACHE_RETENTION=long` (backward
compat) → `"long"`; else `"short"`.

### 7.1 Session headers (affinity, not cache keys)

`createClient` (`openai-responses.ts:311-327`), only when `sessionId` survives the
`cacheRetention === "none"` filter (`:176`):

- `sessionAffinityFormat === "openrouter"` → `x-session-id`
- `sessionAffinityFormat === "openai"` → `session_id` **and** `x-client-request-id`
- other formats → `x-client-request-id` only

`detectSessionAffinityFormat` (`:79-81`) picks `openrouter` when provider is openrouter or the base
URL contains `openrouter.ai`, otherwise `openai`. `compat-schema.ts:332` documents the three formats
(`openai`, `openai-nosession`, `openrouter`) and states explicitly: **"Does not affect
`prompt_cache_key`."**

### 7.2 `cached_tokens` parsing

- Responses: `openai-responses-shared.ts:634-637` — reads
  `usage.input_tokens_details?.cached_tokens` and `cache_write_tokens`, defaulting to `0`.
- Completions (richer fallback chain): `openai-completions.ts:1514-1534` — resolves
  `prompt_tokens_details.cached_tokens ?? prompt_cache_hit_tokens ?? cached_tokens ?? 0`, with an
  in-source comment that `cached_tokens` means **cache read** per documented OpenAI/OpenRouter
  semantics (DeepSeek uses `prompt_cache_hit_tokens`, Kimi a top-level `usage.cached_tokens`), and an
  explicit warning not to fold writes into reads.

---

## 8. What this implies for a Husk-style cache review (facts only, no recommendation)

1. Three distinct mechanisms coexist upstream and must not be conflated: append-only
   `configuration_update` items (effort), append-only `additional_tools` items (tools), and system
   message folding vs in-place system messages.
2. Upstream's cache-preserving effort path is **opt-in per model**
   (`supportsMidConvoEffort`, default `false`) and is hard-disabled for ChatGPT sign-in.
3. Upstream always sends `store: false` and replays reasoning items from a client-side JSON
   signature, so `encrypted_content` presence is load-bearing for reasoning continuity.
4. `previous_response_id` is not used by the standard Responses transport at all.
5. `prompt_cache_key` is derived from `sessionId` only and is orthogonal to affinity headers by
   explicit documentation.
6. Anthropic's mid-conversation tool/system churn needs **two** flags
   (`supportsMidConvoSystemMessages` + `supportsMidConvoToolChanges`); OpenAI Responses uses a
   different pair (`supportsMidConvoSystemMessages` + `supportsAdditionalTools`).

---

## 9. Verification status / gaps

**Fully read and confirmed line-by-line:** `api/openai-responses.ts` (all 453 L, several passes) and
`api/openai-responses-shared.ts` conversion path (`:120-441`) plus stream tail (`:833-891`).

**Located by grep with exact line refs, body not exhaustively read** (treat numbers as confirmed,
semantics as strongly indicated but not paragraph-verified):

- `providers/compat-schema.ts` — flag definitions at the cited lines; surrounding descriptions only
  partially read (`:250-631`).
- `api/anthropic-messages.ts` — the flag read sites (`:231`, `:233`, `:235`, `:978`, `:1143`,
  `:1148`, `:1183`, `:1199`, `:1203`, `:1253`, `:1254`, `:1293`, `:1626`, `:1637`). The
  `cache_control` / `ttl: "1h"` and adaptive-thinking request shapes were **not** read; only
  `compat-schema.ts:336` documents that Anthropic-style retention uses `cache_control.ttl: "1h"`.
- `api/openai-completions.ts` — `:316`, `:798`, `:811`, `:816`, `:1514-1534` only. Its full
  `buildParams`, prefix-trimming, and `chat completions` message conversion were **not** read.
- `utils/transcript.ts` — only `:117-119`; `collapseSystemMessages` and
  `resolveTranscriptTools` bodies not read.
- `api/transform-messages.ts` (256 L) — not read.
- `types.ts` — only `sessionId` (`:257`) and `providerThinkingLevel` (`:594`); the full
  `OpenAIResponsesCompat` / `Model` interfaces not read.
- Official OpenAI/Anthropic documentation was **not** fetched; every statement above is sourced from
  the pinned Pi source. The two doc claims Pi itself asserts (`configuration_update` documented
  "before user messages only"; `additional_tools` accumulate-and-replace semantics "verified …
  October 2026") are reported as Pi's in-source claims, not independently confirmed.

No claims in this note were inferred from a field's name; where a field's behavior is not obvious
from its read sites, that is stated as unverified above.

## 10. Supplemental verification by the main implementation review

The following extends the initial research scope; it does not erase the gaps recorded at the time above.

- Official OpenAI caching, reasoning and Responses-create pages and Anthropic thinking documentation were subsequently fetched. Cache routing keys are not a universal prerequisite for hits; newer model generations have different cache lifetime/options rules. Completed reasoning items, not incomplete added items, are the authoritative replay payload.
- `openai-completions.ts:803-830` was read: Pi's Completions key policy is first-party OpenAI with non-none retention, or long retention with declared support; long retention sends `24h`. This is materially the same conservative policy Husk already used for Completions. The demonstrated short-retention gateway-key defect is therefore specifically a **Responses** difference, not a claim that Pi always sends keys on all compatible APIs.
- `openai-completions.ts:1638-1683` was read: defaults use provider/URL inference, including nonstandard-provider gates; long retention defaults off for Together/Cloudflare/Nvidia/AntLing and on otherwise. Husk keeps an explicit opt-in long-retention capability instead of copying this provider-name table.
- `utils/transcript.ts:67-120` was read: folding joins system content, updates named sections, resolves tools, and removes later system rows. A true supportsMidConvoSystemMessages keeps the original transcript in place. Folding is not proof that later system edits preserve the initial prefix.
- `anthropic-messages.ts:224-236`, `:1160-1315`, and `:1622-1665` were read. Its temperature/strict/mid-conversation defaults and native tool/thinking request branches are confirmed. `supportsStrictTools` gates a tool's constrained-sampling request; it does not make every tool strict merely because the model supports it.
- The model catalog imports generated `providers/data/*.json`. The pinned raw GitHub URLs for Anthropic/OpenAI data returned HTTP 404 in this review, so this note does **not** claim an exhaustive concrete-model compat inventory from those build artifacts. Claude Opus 4.7/Mythos references are source comments/examples, not a substituted inventory.
- Husk `SetThinkingLevel` was directly checked: it changes engine settings and appends a UI-only notice, not the system prompt. Permission/agent-mode switches do rewrite their corresponding system blocks. They must not be lumped together as unconditional thinking-level prompt churn.

Implementation findings and the authoritative final test status are recorded in [the Husk audit](husk-provider-cache-audit.md).
