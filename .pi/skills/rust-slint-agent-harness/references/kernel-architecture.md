# Kernel Architecture Spec

Module-by-module contract for the kernel crate. Crate topology and dependency law: `workspace-layout.md` — this document describes `crates/agent-kernel/src/` unless noted. Modeled on Grok Build's actor topology, adapted to a desktop (non-TUI) shell.

## Actor topology

```
┌────────────┐  commands   ┌───────────────┐  SamplingRequest  ┌───────────────┐
│ Slint UI   │────────────▶│ SessionActor  │─────────────────▶│ SamplerActor  │
│ (main loop)│◀────────────│ (engine.rs)   │◀─────────────────│ (llm/)        │
└────────────┘  UiUpdate   └──────┬────────┘   SamplingEvent   └───────────────┘
                                  │ RecordAgentWrite / HandleFileChange
                                  ▼
                          ┌───────────────┐
                          │HunkTracker    │
                          │ (hunks.rs)    │
                          └───────────────┘
```

- One `tokio::mpsc` channel per direction. No shared `Mutex` state between actors except the `StreamThrottler` buffer.
- `SessionActor` owns: conversation history, `AgentState`, permission manager, tool dispatch.
- `SamplerActor` owns: the active `Arc<dyn LlmProvider>`, retry/doom-loop/idle-timeout policy, fallback chain. HTTP/SSE details live inside provider adapters — see `llm-provider-layer.md` for the full spec (normalized `StreamChunk`, `openai_compat` adapter, `config.toml` schema, `env:` key indirection, hot-swap, `MockProvider`).
- `HunkTrackerActor` owns: `file_states: HashMap<PathBuf, FileState>`, `turn_index`, `git_dirty_cache`, `TrackingMode` (`AgentOnly` | `AllDirty`).

## AgentState machine (kernel/engine.rs)

```rust
pub enum AgentState {
    Idle,
    ScanningWorkspace,
    Reasoning,                                   // waiting on first SSE byte
    StreamingToken,                              // text deltas flowing
    AwaitingToolConfirmation { tool_name: String, diff_summary: String },
    AwaitingPluginConsent { plugin_id: String, capability: String },  // plugin escalation beyond manifest
    AwaitingConsent { kind: ConsentKind },   // P2: generalizes tool/plugin/capture approvals
    Branching { candidates: u8 },            // P2: CoW fork-and-verify (capability-roadmap §4)
    ExecutingTool { tool_name: String },
    Compacting,                                  // auto-compact in flight
    Finished,
    Failed(String),
}
```

Transitions: `Idle→ScanningWorkspace→Reasoning→StreamingToken→{Finished | AwaitingToolConfirmation→ExecutingTool→Reasoning}` → loop. `Failed` is terminal per turn, not per session.

**Steering (P1)**: `SessionActor` drains the input channel between tool calls and every ~50 streamed tokens; `UiCommand::Steer(text)` injects a user message mid-turn ("completed tool calls remain valid"), `steered` flag marks provenance — the state machine is hot-patched, not reset.

**Memory (P1)**: `kernel/memory.rs` — LibSQL + FastEmbed; layers = working/session/episodic/semantic/persona; per-workspace partitioning by `hash(canonical_root)`; distiller runs post-turn in background; top-k retrieval injects `<memory>` block (≤2 KB) after the workspace skeleton. Full contract in `capability-roadmap.md` §1.

## Sampler resilience (llm/sampler.rs)

| Mechanism | Value |
|-----------|-------|
| SSE idle timeout | 300 s → `IdleTimeout` error |
| Doom-loop detection | repeated identical generation pattern → abort + resample, budget `doom_max_retries` (default 3) |
| Retry | exponential backoff on transport + 429/5xx; never on other 4xx |
| Mid-stream failure | salvage partial text (`interrupted` marker) + continuation-prompt retry, ≤3 attempts — see production-hardening §4 |
| Fallback | retry budget exhausted → next `fallback_chain` provider; emit `system` degrade message to UI |
| APIs | `LlmProvider` trait; `openai_compat` first (OpenAI/xAI/DeepSeek/Ollama/vLLM), `anthropic` v2 |

## Tool registry (tools/registry.rs)

```rust
pub struct ToolSpec {
    pub name: &'static str,
    pub schema: serde_json::Value,      // JSON Schema for the LLM
    pub readonly: bool,                 // skips confirmation in `default` mode
    pub exec: fn(Args, &ToolCtx) -> BoxFuture<ToolResult>,
}
```

Built-ins: `read_file`, `list_dir`, `grep`, `search_replace`, `apply_patch`, `bash`. Registry produces the `tools` array for the LLM request and dispatches by name.

## Permission modes (kernel/permissions.rs)

Precedence: `deny > ask > allow`. Rules from CLI flags + `~/.config/<app>/config.toml` + `<repo>/.agent/config.toml` (+ `.claude/settings.json` compatibility optional).

| Mode | Behavior |
|------|----------|
| `default` | ask for non-readonly tools; readonly tools auto-run |
| `acceptEdits` | file edits auto-approve; shell still asks |
| `auto` | auto-approve what passes safety checks; escalate the rest |
| `dontAsk` | only pre-approved tools + readonly shell (`ls`, `cat`, `git status`…) |
| `bypassPermissions` | approve everything except `deny` rules and destructive shell `ask` rules |

Plugin tools flow through the same pipeline (`readonly` defaults `false`); capability escalation beyond the manifest pauses in `AwaitingPluginConsent` → native dialog. See `plugin-system.md`.

Readonly shell whitelist: `ls cat head tail grep rg find git{status,diff,log,show} kubectl-get`… — configurable.

## Extension pipeline (hooks / commands / providers)

Three interception surfaces wrap the ReAct loop (full spec: `plugin-system.md` §Extension points):

- **Commands** (`commands.rs`): input starting with `/` is intercepted by `CommandRegistry` **before** the LLM loop — zero tokens. `CommandResult::{Reply, ControlAction, FeedToAgent}`. MCP `prompts/*` and WASM `command_execute` both register here under `plugin_id:name`.
- **Hooks** (`hooks.rs`): ordered `AgentHook` chain — `on_user_input` → `before_tool_execute` (veto/rewrite, runs *before* the permission gate and cannot approve) → `after_tool_execute` → `on_state_transition`. Per-hook 2 s timeout, degradation = `Continue`. WASM/built-in only.
- **Providers** (`agent-context/src/providers.rs`): `ContextProvider` trait returns `ContextChunk{priority, title, content, tokens}`; built-ins (workspace/git/memory) are ordinary registrations. Assembly sorts by priority, fills budget, drops the rest with UI-visible badges. MCP `resources/*` maps here.

## Execution sandbox (`crates/agent-sandbox/`)

The `bash` tool NEVER calls `Command` directly — always through `SandboxBackend::run_command`. Full spec in `sandbox-model.md`. Contract points:

- `SandboxConfig` per call: workspace dir, `allow_network`, `max_memory_mb` (2048), `timeout_secs` (≤600), sanitized env, `snapshot: Off|Cow|Required`.
- Backend detection at startup: Linux `bwrap`→`landlock`, macOS `sandbox-exec`, Windows Job Object; fallback `none` requires `sandbox.allow_unsandboxed` + confirms every call + warn chip.
- Four enforced dimensions: fs (workspace+tmp rw only, sensitive dirs denied), network (`--unshare-net`/`deny network*` when off), resources (mem cap, wall timeout, **process-tree kill**), env (allowlist + `*_KEY/_TOKEN/_SECRET` denylist).
- Pre-execution `audit.rs` classifies commands: critical/elevated/network-mutating/scope-violation → forced confirmation + `risk` level on the approval card. Even approved criticals run inside the sandbox (CoW-forced), never raw host.
- `SnapshotMode::Cow` reflink-clones workspace (APFS/Btrfs/XFS), agent mutates the clone, merge-back diff goes through `AwaitingToolConfirmation` + HunkTracker (`origin: "sandbox-merge"`).

## Truncation table (kernel/context.rs + tools)

| Source | Cap | Strategy |
|--------|-----|----------|
| Tool result → model | 40 KB (≈10k tok), per-tool overridable | head 30% + `… [truncated N bytes] …` + tail 70% (errors land in tail) |
| `bash` result | 20 KB chars (≈5k tok) | same head/tail fold |
| `read_file` | 1000 lines OR 25k tok | hard cut + continuation hint `offset=` |
| `grep` | 5 MiB stdout, 20 s | count matches, then truncate lines |
| `list_dir` | entry budget | "too large to list fully" marker |
| Plugin tool result | same 40 KB default | same head/tail fold — plugins are not exempt |
| Conversation | context_window e.g. 256k | auto-compact at ~80% |

## Compaction (kernel/compaction.rs)

Two-pass: split history into prefix/suffix → summarize prefix to `NOTE₁` → condense `NOTE₁` + suffix. Optional prefire: start Pass 1 in background at `PREFIRE_LEAD_PERCENT` (e.g. 70%) so compaction latency is hidden. Sticky suppression (`SUPPRESS_STICKY`, `SUPPRESS_UNTIL_SUCCESS`) prevents compaction retry storms. Sanitize pipeline before sampling: flatten tool calls, strip reasoning blocks, replace images with placeholders, `fit_conversation_to_budget`.

## Bridge throttling (crates/app-desktop/src/throttler.rs)

- `StreamThrottler`: 33 ms `tokio::interval`, drain `Mutex<String>` buffer → `slint::invoke_from_event_loop(move |ui| append_delta(text))`. Never call `invoke_from_event_loop` per SSE chunk.
- Tool status and diff payloads are data, not text: send through a separate `UiUpdate::ToolCall` / `UiUpdate::DiffReady` variant on the same UI channel.

## Hunk tracking (kernel/hunks.rs)

- `RecordAgentWrite {path, old, new, origin}` after every successful write (built-in or plugin) → hunks keyed by turn; `origin` distinguishes `agent` vs `plugin:<id>`.
- `HandleFileChange {path}` from a `notify` watcher → attribute external edits (don't merge them into agent undo history).
- Powers: per-turn "files changed" list in UI, Undo/Rewind (`git apply -R` of recorded hunks), dirty-file warnings on session start (`AllDirty` mode).

## Crate choices

| Need | Crate |
|------|-------|
| UI | `slint` (native backend, not winit+skia software) |
| Async | `tokio` (multi-thread, fs + net features) |
| HTTP/SSE | `reqwest` (rustls, stream, `tcp_nodelay`) + `eventsource-stream` |
| Async trait/stream | `async-trait`, `futures` (BoxStream) |
| Walk | `ignore::WalkBuilder` |
| Grep | `grep-searcher` + `grep-regex` (in-process; fallback: shell `rg`) |
| Diff | `similar` (unified + line ops) |
| Patch apply | `diffy` or hand-rolled on `similar` hunks |
| Git | `git2` (status/diff), or shell `git` for v1 simplicity |
| Watch | `notify` |
| Schema/serde | `serde`, `serde_json`, `schemars` for tool schemas |
| Tokens | `tiktoken-rs` or char/4 heuristic (configurable) |
| Errors | `anyhow` (app), `thiserror` (libs) |
| Config | `toml` + `dirs` |
| Tray/hotkey | `tray-icon`, `global-hotkey` |
| Sandbox | `tokio::process` + platform: `bwrap` binary or `landlock` crate (Linux), `sandbox-exec` (macOS), Job Objects via `windows` crate; `reflink-copy` for CoW |
| WASM plugins | `wasmtime` + `wasmtime-wasi` (async, fuel) |
| MCP plugins | stdio JSON-RPC client (hand-rolled or `rmcp`) |
| Memory (P1) | `libsql` (embedded) + `fastembed-rs` (ONNX embeddings) |
| Vision (P2) | `screenshots`/`xcap` capture + local VLM via llama.cpp or `candle` |
| Secrets | `keyring` (OS credential store) + egress masker in agent-llm |
| Persistence | `libsql`/`sqlx` WAL + single-writer actor; `refinery` migrations |
| Self-update | `self_update` (GitHub Releases, atomic replace) |

## Workspace scanner contract (crates/agent-context/src/workspace.rs)

`WorkspaceScanner::build_file_tree(root, max_depth=3)` → `Vec<String>` relative paths; `ignore::WalkBuilder` with `.hidden(true).git_ignore(true).git_exclude(true)`; cap ~2000 entries with fold marker. `git_snapshot()` → `git status --porcelain` + `git diff --stat` truncated to ~4 KB.

## Acceptance checklist

- [ ] `cargo build --release` → single binary, no runtime deps beyond system libs
- [ ] Cold start to interactive window < 300 ms on M-series
- [ ] 30 fps token rendering sustained at 200 tok/s without dropped frames
- [ ] Every write tool passes through `AwaitingToolConfirmation` in `default` mode
- [ ] `deny` rule honored in `bypassPermissions`
- [ ] Auto-compact fires near 80% and session continues coherently
- [ ] Undo restores last-turn file state from hunk tracker
- [ ] Idle RAM < 40 MB after 5 min
- [ ] Permission-less WASM plugin cannot open file/socket (hostile test)
- [ ] Plugin consent dialog round-trips through `AwaitingPluginConsent`
- [ ] Plugin context provider output appears in prompt within 2 s timeout
- [ ] Sandboxed `bash` cannot read `~/.ssh`, cannot see `*_TOKEN` env vars
- [ ] `rm -rf` audit → critical confirm card; approved run still sandboxed + CoW
- [ ] Command timeout kills the whole process tree (no orphans)
- [ ] `UiCommand::Steer` mid-turn adjusts plan without re-running completed tools
- [ ] Memory recall injects relevant facts in session 2 without re-prompting
- [ ] Ambient compile-error probe emits suggestion chip, never auto-starts a turn
- [ ] `kill -9` host mid-tool → zero orphan processes (pgrep clean)
- [ ] Configured secret never appears raw in outbound request body or logs
- [ ] 200 concurrent DB writes + reads → zero `database is locked`
- [ ] v0.2 binary boots a v0.1 `memory.db` — migrates, zero loss
