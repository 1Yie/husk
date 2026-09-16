# Slint UI Contract

The bridge (`crates/app-desktop/src/bridge.rs`) owns every binding listed here. Rust fills the models; Slint renders declaratively. All `.slint` files live under `crates/app-desktop/ui/`. Theme: Catppuccin Mocha.

## globals.slint — structs (Rust `slint::Struct` mirrors)

```slint
export struct DiffLineData {
    line_type: string,        // "add" | "delete" | "context"
    content: string,
    old_lineno: int,          // -1 when n/a
    new_lineno: int,
}

export struct ActionStepData {
    id: int,
    name: string,             // tool name: "fuzzy_patch", "bash"…
    state: string,            // "running" | "awaiting_confirm" | "success" | "error" | "denied"
    detail: string,           // one-line summary: path, command, match count
    expandable: bool,
}

export struct SessionMessageData {
    id: int,
    role: string,             // "user" | "agent" | "system" | "tool"
    text: string,             // markdown-lite; rendered incrementally
    reasoning: string,        // accumulated ReasoningDelta; shown in collapsible "Thinking…"
    step_ids: [int],          // references into Bridge.active_steps — NOT a nested model
    has_diff: bool,
    streaming: bool,          // caret/typing indicator
}

// NOTE: do NOT put `steps: [ActionStepData]` inside SessionMessageData.
// Nested arrays inside structs map to immutable slices on the Rust side —
// Slint's change notifier can't see deep mutations, so updating one step's
// state would require deep-copying the whole message (re-mount flicker).
// Active steps live in the flat `Bridge.active_steps` model; archived
// messages resolve their step_ids against it at render time.

export struct PluginInfo {
    id: string,
    name: string,
    version: string,
    kind: string,             // "wasm" | "mcp"  → badge style
    enabled: bool,
    permissions: [string],    // flattened manifest list for display
    tool_count: int,
    status: string,           // "loaded" | "error" | "disabled"
    error: string,            // load error detail when status == "error"
}

export struct PluginConsentData {
    plugin_id: string,
    plugin_name: string,
    capability: string,       // e.g. "network: api.github.com" | "write: ./dist"
    reason: string,
}

export struct SuggestionData {
    id: int,
    kind: string,             // "compile_errors" | "test_failures" | "stale_branch"
    text: string,             // "3 compile errors detected — fix now?"
    ttl_secs: int,            // chip auto-expires
}

export struct MemoryFactData {
    id: int,
    kind: string,             // "fact" | "persona" | "episode"
    text: string,
    confidence: float,
}

export struct SlashCommandData {
    name: string,             // "diff" — invoked as /diff; plugin ones prefixed "plugin:name"
    description: string,
    source: string,           // "builtin" | "plugin:<id>" | "mcp:<server>"
}

export struct ContextBadgeData {
    provider: string,         // provider name or plugin id
    title: string,            // "Git Diff", "Memory: 3 facts"
    tokens: int,
    injected: bool,           // false = dropped by budget pressure
}

export struct BranchCandidateData {
    label: string,            // "Plan A: refactor interface"
    state: string,            // "running" | "adopted" | "pruned"
    result: string,           // "cargo check: 0 errors" | rejection reason
}

export struct ModelOption {
    provider_id: string,      // key in config [providers.*]
    provider_type: string,    // "openai_compat" | "anthropic"
    label: string,            // display name, e.g. "grok-4 (xAI)"
    model: string,
    available: bool,          // false when env: key unresolved
}

export struct PendingApprovalData {
    step_id: int,
    tool_name: string,
    title: string,            // "Edit src/main.rs"
    diff_lines: [DiffLineData],   // empty for bash → show command in detail
    command: string,
    risk: string,             // "normal" | "elevated" | "critical" | "network"
    audit_reason: string,     // matched pattern, e.g. "rm -rf flagged as destructive"
}

export struct SandboxTelemetry {
    step_id: int,
    backend: string,          // "bwrap" | "sandbox-exec" | "job-object" | "none"
    elapsed_ms: int,
    peak_memory_mb: int,
    snapshot: string,         // "off" | "cow" | "required"
    sandboxed: bool,          // false → persistent warn styling
}

export struct SessionStats {
    tokens_used: int,
    context_window: int,
    files_changed: int,
    agent_state: string,      // mirror of AgentState for status bar
    permission_mode: string,
    active_provider: string,
    active_model: string,
    degraded: bool,           // true while running on fallback_chain provider
}
```

## globals.slint — Theme

```slint
export global Theme {
    in-out property <color> bg:          #181825;
    in-out property <color> surface:     #1e1e2e;
    in-out property <color> surface2:    #313244;
    in-out property <color> accent:      #89b4fa;
    in-out property <color> text:        #cdd6f4;
    in-out property <color> text_dim:    #7f849c;
    in-out property <color> diff_add_bg: #1c3329;
    in-out property <color> diff_del_bg: #3b222c;
    in-out property <color> diff_add_fg: #a6e3a1;
    in-out property <color> diff_del_fg: #f38ba8;
    in-out property <color> warn:        #f9e2af;
    in-out property <color> error:       #f38ba8;
    in-out property <string> mono_font:  "JetBrains Mono";   // font-family is a string, not a color — <color> here fails slint-build
}
```

## globals.slint — Bridge callbacks (Rust implements, Slint invokes)

```slint
export global Bridge {
    // UI → kernel
    callback submit_prompt(string);
    callback approve_tool(int /*step_id*/, bool /*remember*/);
    callback deny_tool(int /*step_id*/, string /*reason*/);
    callback cancel_turn();
    callback undo_turn(int /*turn*/);
    callback set_permission_mode(string);
    callback set_model(string /*provider_id*/, string /*model*/);
    callback set_plugin_enabled(string /*plugin_id*/, bool);
    callback reload_plugins();
    callback approve_plugin_capability(string /*plugin_id*/, string /*capability*/, bool /*always*/);
    callback deny_plugin_capability(string /*plugin_id*/, string /*capability*/);
    callback steer(string);                         // mid-turn course correction
    callback accept_suggestion(int /*id*/);
    callback dismiss_suggestion(int /*id*/);
    callback delete_memory_fact(int /*id*/);
    callback run_command(string /*name*/, string /*args*/);   // slash dispatch

    // kernel → UI (set as properties / model pushes)
    in-out property <[SessionMessageData]> messages;
    in-out property <PendingApprovalData> pending;      // .tool_name == "" → none
    in-out property <SessionStats> stats;
    in-out property <[string]> changed_files;           // hunk tracker feed
    in-out property <[ModelOption]> model_options;      // populated from config.toml
    in-out property <[PluginInfo]> plugins;             // plugin manager feed
    in-out property <PluginConsentData> plugin_consent; // .plugin_id == "" → none
    in-out property <[SandboxTelemetry]> running_sandboxes; // live per-step telemetry
    in-out property <bool> sandbox_unsafe;              // backend == "none" → warn chip
    in-out property <[SuggestionData]> suggestions;     // ambient probes, ≤1 visible
    in-out property <[MemoryFactData]> memory_facts;    // inspectable/deletable
    in-out property <[BranchCandidateData]> branches;   // non-empty while Branching
    in-out property <bool> steered;                     // current turn was steered
    in-out property <[ActionStepData]> active_steps;    // flat model — steps mutate in place here
    in-out property <[SlashCommandData]> slash_commands;   // for the / autocomplete popup
    in-out property <[ContextBadgeData]> context_badges;   // what fed this turn's prompt
}
```

## Components

### components/diff_viewer.slint

Renders `[DiffLineData]`: per-line `Rectangle` tinted `diff_add_bg`/`diff_del_bg`, `+`/`-`/space gutter, mono font 12 px, optional line-number column (`old_lineno`/`new_lineno`, `-1` → blank), `clip: true`, horizontal scroll for long lines. Keep row height fixed (20 px) — use `ListView` for >500 lines so layout stays O(visible).

**Nested-scroll rule**: a `ListView` inside a `ListView` row breaks virtualization (inner list can't get a definite height constraint → outer row measures it → layout explodes on large diffs, wheel events fight). Therefore:
- Inline diffs inside chat bubbles cap at **50 lines collapsed** with `max-height: 400px` and their own internal scroll — never an unconstrained inner `ListView`.
- Large diffs (>50 lines) open in the **`workspace_view` slide-over** (full-height, dedicated scroll), not inline in the bubble.

### components/tool_status.slint

Card per `ActionStepData`: spinner while `running`, accent border `awaiting_confirm`, green check `success`, red `error`, strikethrough `denied`. `expandable` opens inline `DiffViewer` or command output preview (≤200 lines, mono, `max-height: 400px` — see nested-scroll rule); anything larger routes to `workspace_view`.

**Live sandbox telemetry**: while a `bash` step is `running`, its card foot shows `SandboxTelemetry` — elapsed s, peak MB, backend badge (`bwrap`/`sandbox-exec`/`job-object`), snapshot mode chip; `sandboxed: false` forces a warn-colored "UNSANDBOXED" badge.

**Approval risk tiers**: `risk == "critical"` → red banner + `audit_reason` + mandatory reason on deny; `"elevated"`/`"network"` → amber banner; `"normal"` → default card.

**Reasoning block**: when `SessionMessageData.reasoning` is non-empty, render a collapsible dimmed "Thinking…" panel above the text — collapsed by default once `streaming` ends.

### components/chat_bubble.slint

`role`-styled bubble: user right-aligned accent-tinted, agent full-width, tool collapsible. `streaming: true` shows blinking caret. Text is appended whole-chunk (already throttled to 30 fps by `StreamThrottler`) — do not animate per character.

### views/session_view.slint

Layout: message `ListView` (auto-scroll pinned to bottom unless user scrolled up), `PendingApprovalData` banner docked above input when `.tool_name != ""` (Approve / Approve-all-session / Deny+reason), input `TextEdit`, status bar: `stats.agent_state`, token meter `tokens_used/context_window` (amber >70%, red >90%), permission mode chip, **model picker** (combo of `model_options` → `Bridge.set_model`; amber `⚠` when `degraded`), `UNSANDBOXED` chip when `sandbox_unsafe`.

**Steering**: while `stats.agent_state` is `Reasoning`/`StreamingToken`/`ExecutingTool`, the input box stays enabled as "Steer…" mode — Enter → `Bridge.steer(text)` (placeholder swaps to "Steer the agent…"); cancel remains a separate button/shortcut. `steered` turns render a subtle ⟲ marker.

**Ambient suggestions**: `suggestions` renders as a quiet status-bar chip (not a modal) — click → `accept_suggestion` (becomes a normal prompt); × → `dismiss_suggestion`. Auto-expires on `ttl_secs`.

**Branch card**: while `agent_state == "Branching"`, `branches` renders per-candidate rows (label, spinner/check/✂, result line); adopted branch's diff then flows through the normal approval path.

**Memory panel**: settings sub-page lists `memory_facts` grouped by kind (fact/persona/episode) with confidence bar and per-row delete → `delete_memory_fact`.

**Slash autocomplete**: typing `/` in the input opens a popup over it listing `slash_commands` (name + description + source badge); Tab/Enter completes → `Bridge.run_command(name, args)`. Unknown `/x` submits as normal text with a hint.

**Context badges**: row above input shows `context_badges` — `provider:title (N tok)`; `injected: false` renders dimmed with a strikethrough so the user sees budget drops; click → remove that provider for subsequent turns.

**Hook trace**: when a hook mutates or vetoes, the affected `ActionStepData` card gains a dimmed `⚡ hook <id>` footer line — silent interventions are always visible.

### views/plugin_settings_view.slint

List of `plugins`: name + version + kind badge (WASM = lock/accent, MCP = process/warn), permission chips, enable toggle → `set_plugin_enabled`, `status == "error"` rows expand to show `error`. Header: "Reload plugins" → `reload_plugins`.

**Consent dialog**: modal when `plugin_consent.plugin_id != ""` — `Plugin "{plugin_name}" requests {capability}. {reason}` with Allow once / Always allow / Deny → `approve_plugin_capability(id, cap, always)` or `deny_plugin_capability`.

### views/workspace_view.slint

Side panel: `changed_files` list grouped by turn, per-file added/removed line counts, click → `DiffViewer` of recorded hunks, "Undo turn" button → `Bridge.undo_turn`.

## app_window.slint — routing

Root `Window` with `session_view` default; `workspace_view` slides in from right; global hotkey (registered in `main.rs` via `global-hotkey`) toggles visibility; tray icon menu: Show / Quit.

**Typed tool-result cards**: when a tool result JSON carries `ui_type` ∈ {`diff`, `table`, `markdown`}, `tool_status` expanded area renders the matching card (`diff` → `DiffViewer`, `table` → column grid, `markdown` → rich text). Missing/unknown `ui_type` → plain mono text fallback.

## Binding rules for the Rust side

- All model mutation goes through `slint::invoke_from_event_loop` — never touch `Slint` objects off the UI thread.
- `messages` is a `VecModel<SessionMessageData>`; streaming updates mutate `text` of the last row in place (`set_row_data`), not push/remove.
- **`set_row_data` lifecycle (mandatory 3-step)**: (1) turn start → `VecModel.push(SessionMessageData{streaming: true, ..})` reserves the row; (2) each 33 ms throttle flush → `set_row_data(len-1, updated)` — calling it before the push is an out-of-bounds panic; (3) turn end → `set_row_data(len-1, final{streaming: false})`. Same lifecycle for `active_steps` rows.
- `active_steps` is a flat `VecModel<ActionStepData>` keyed by `step_id` — step state changes `set_row_data` the matching row only; never rebuild the array per tick.
- `pending` set → UI blocks input focus to approval banner; `approve_tool`/`deny_tool` resumes the kernel loop.
- `stats.tokens_used` updated once per sampling turn end (from `StreamChunk::Done` usage), not per token.
- `model_options` loaded at startup from `config.toml`; `set_model` applies to the next turn and updates `stats.active_provider`/`active_model`; a provider with unresolved `env:` key stays listed but `available: false` (greyed out).
- On fallback degrade the kernel pushes a `role: "system"` message ("grok unavailable → running on ollama") and sets `stats.degraded` until the primary recovers.
- `plugin_consent` blocks the turn exactly like `pending` — input disabled until a callback resolves it.
- `running_sandboxes` updated at ~4 Hz by the bridge (same `invoke_from_event_loop` path); entries keyed by `step_id`, removed on step end.
- `sandbox_unsafe` shows a persistent status-bar chip; when true every bash approval is forced regardless of permission mode.
- `plugins` list refreshes on startup + `reload_plugins`; toggling `enabled` takes effect on the next LLM request (schema list rebuilt).

## Implementation phasing (for AI-assisted builds)

1. **Mock data first**: before writing `bridge.rs`, populate `main.rs` with static stub models — two messages, one `critical` `PendingApprovalData`, one `SandboxTelemetry` — and visually verify Catppuccin theme + layout. Backend wiring comes after the UI is confirmed.
2. **Core callbacks first**: wire only `submit_prompt`, `approve_tool`/`deny_tool`, and `steer` end-to-end before touching the rest. `delete_memory_fact`, `reload_plugins`, `set_model` are secondary — stub them to no-ops initially.
3. **Model primitives**: `VecModel` for flat lists only; `MapModel`/`FilterModel` adapters in Slint rather than restructuring Rust data per view.
