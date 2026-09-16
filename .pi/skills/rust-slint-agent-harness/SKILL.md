---
name: rust-slint-agent-harness
description: Build a Codex/Grok-Build-style desktop coding agent with Rust + Slint + a native agent kernel. Use when scaffolding, extending, or reviewing this project's workspace scanner, tool registry, permission/diff-review flow, streaming bridge, context budgeting, compaction, or Slint UI wiring. Contains the kernel system prompt and UI/Kernel contracts.
---

# Rust + Slint Desktop Agent Harness

This skill turns the assistant into the lead engineer for this repository: a **single-binary desktop agent harness** (Rust + Slint UI + native async kernel) that borrows Grok Build CLI's core loop — workspace awareness, diff review, command execution, strict token budgets — inside a native shell that cold-starts in milliseconds and idles at 15–30 MB.

## Activation

Apply this skill whenever the task involves `Cargo.toml`, `build.rs`, `crates/**`, `ui/**.slint`, or `src/**` in this repository, or when the user asks to design, implement, debug, or review any part of the agent (kernel loop, tools, permissions, diff viewer, streaming, context, compaction).

## Operating Principles

These are invariants. Violating any of them breaks the product thesis.

1. **Single static binary.** No Node, Python, or external runtime deps. Ripgrep-style speed via `ignore` + `grep-searcher` crates, not shelling out unless the design says so.
2. **Tool calls are structured.** Models emit `search_replace` / `apply_patch` blocks — never full-file rewrites. Edits apply locally via `similar`-verified patches, tracked by the hunk tracker.
3. **Everything destructive requires review.** File writes, patches, and non-readonly shell commands pause in `AwaitingToolConfirmation` and render a `DiffViewer` in the UI until the user approves (unless the permission mode says otherwise).
4. **Token budgets are hard limits.** Tool output: ~40 KB default (~20 KB for shell). Context window: auto-compact near 80%. Never stream raw unbounded output into the model.
5. **The UI never blocks.** LLM SSE deltas funnel through `StreamThrottler` (~30 fps flush). Slint's event loop is touched only via `slint::invoke_from_event_loop`.
6. **Actor-model kernel.** `SessionActor` (orchestrator), `SamplerActor` (LLM streaming + retries), `HunkTrackerActor` (file-change attribution) communicate over `tokio::mpsc`. No shared mutable state outside actors.
7. **Providers are pluggable.** All LLM access goes through `Arc<dyn LlmProvider>` emitting normalized `StreamChunk`s. Kernel/UI contain zero vendor-specific fields. A `GenericOpenAiProvider` covers OpenAI/xAI/DeepSeek/Ollama/vLLM; Anthropic gets its own adapter. Providers hot-swap via config without restart.
8. **Plugins are sandboxed, dual-track.** WASM (Wasmtime, capability-gated WASI) for trusted/high-perf extensions; MCP stdio for ecosystem breadth. Both map to one `Plugin` trait (tools + context providers). Manifest permissions are exhaustive maximums — anything undeclared is physically denied. Plugin output obeys the same truncation budgets as built-ins.
9. **Every command runs in a sandbox.** `bash` never spawns a bare host process: L2 tier (bubblewrap/Landlock on Linux, `sandbox-exec` on macOS, Job Objects on Windows) restricts fs to workspace+tmp rw, sanitizes env secrets, caps memory/timeout, kills process trees. Pre-execution audit escalates `rm -rf`/`git push -f`/`curl|sh` to critical-confirm. Optional CoW snapshots give instant rollback. `none` backend is opt-in and loud.
10. **Sessions are steerable, memory is layered.** Mid-turn user input hot-patches the plan via `UiCommand::Steer` without resetting the state machine. Long-term cognition lives in local LibSQL + FastEmbed: episodic (turn outcomes), semantic (project conventions), persona (user style) — all inspectable, deletable, scoped per workspace.
11. **Death and secrets are engineered.** Every spawned child registers in a `ChildRegistry` with parent-death wiring (prctl/pgid/Job Object) — `kill -9` on the host leaves zero orphans. Secrets resolve via `keyring:`/`env:` indirection, never cross to the model raw: an egress masker redacts them from every outbound request.

## Build Order

Crate topology is a Cargo workspace — see `references/workspace-layout.md`. Implement in this sequence — each stage is independently testable:

| # | Stage | Key files | Done when |
|---|-------|-----------|-----------|
| 1 | Workspace + skeleton | `crates/agent-context/src/workspace.rs`, `git.rs` | file tree respects `.gitignore`, git status/diff snapshot works |
| 2 | LLM provider layer | `crates/agent-llm/src/*` | `LlmProvider` trait + normalized `StreamChunk`; `openai_compat` adapter parses text + reasoning + `tool_call` deltas; idle timeout 300 s; retry w/ doom-loop guard; `config.toml` loads ≥2 providers and hot-swaps; `MockProvider` replays scripted chunks |
| 3 | Tools + registry | `crates/agent-kernel/src/tools/*` | `read_file`, `list_dir`, `grep`, `search_replace`, `apply_patch`, `bash` dispatch by name with JSON schemas; output truncated per budget table |
| 4 | Agent loop | `crates/agent-kernel/src/engine.rs`, `session.rs`, `channels.rs` + `crates/agent-ipc/src/events.rs` | `AgentState` machine runs prompt → stream → tool → inject → re-sample **headless via MockProvider in `cargo test -p agent-kernel`**; budgets enforced |
| 5 | Bridge + UI | `crates/app-desktop/src/{bridge,throttler}.rs`, `crates/app-desktop/ui/**` | streamed text appears at 30 fps; tool calls show status cards; diffs render in `DiffViewer`; approval round-trip works |
| 6 | Permissions + hunk tracking | `crates/agent-kernel/src/permissions.rs`, `crates/agent-context/src/hunks.rs` | permission modes gate tools; every agent write recorded as hunks; Undo/Rewind possible |
| 7 | Compaction | `crates/agent-kernel/src/compaction.rs` | two-pass summarize near 80% window; prefire optional |
| 8 | Execution sandbox | `crates/agent-sandbox/src/*` | `SandboxBackend` trait + platform backends; env sanitization + resource limits verified; audit gate escalates dangerous commands; CoW snapshot/merge works |
| 9 | Plugin system | `crates/agent-plugin/src/*` | `PluginManager` routes plugin tools + context providers; WASM sandbox enforces manifest permissions; MCP stdio bridge works; consent dialog round-trip |
| 10 | Memory + steering | `crates/agent-context/src/memory/*`, `crates/agent-kernel/src/steering.rs` | LibSQL store + FastEmbed recall injects `<memory>` into prompts; `UiCommand::Steer` mid-turn hot-patches plan; ambient error probe suggests fixes |
| 11 | Production hardening | `agent-llm/src/masking.rs`, `agent-context/src/memory/store.rs`, boot sequence | WAL + single-writer actor live; `keyring:` secrets + egress masker verified; child-registry orphan test passes; mid-stream salvage + offline degrade work; migrations + self-update safe; CJK/HiDPI correct on fresh VM |
| 12 | Polish | tray, hotkeys, `~/.config` | single `cargo build --release -p app-desktop` binary; cold start < 300 ms |

## Reference Files

Load these on demand — do not re-derive their contents from memory:

- **`references/kernel-system-prompt.md`** — the exact system prompt to ship inside the kernel (tool-use protocol, output budget rules, diff-review etiquette, safety). Render it at session start with workspace variables substituted.
- **`references/llm-provider-layer.md`** — pluggable LLM engine spec: `LlmProvider` trait, normalized `StreamChunk`, OpenAI-compat/Anthropic adapters, `config.toml` schema with `env:` key indirection, hot-swap, fallback chains, mock provider for tests.
- **`references/plugin-system.md`** — dual-track plugin spec: manifest contract, `Plugin` trait, Wasmtime capability sandbox (fs preopens, network allowlist, fuel + timeouts), MCP stdio bridge, `PluginManager` routing, consent flow, typed UI cards, hostile-plugin test checklist.
- **`references/sandbox-model.md`** — layered execution isolation: L1 WASM / L2 native process (bwrap, Landlock, sandbox-exec, Job Objects) / L3 microVM; `SandboxBackend` trait, four control dimensions (fs/net/resources/env), audit levels, CoW snapshot merge-back, degradation policy.
- **`references/production-hardening.md`** — six delivery-grade blind spots: process-tree lifecycle (pdeathsig/pgid/Job Object, ChildRegistry, ordered shutdown), keyring secrets + egress masker, SQLite WAL + single-writer actor, mid-stream salvage + offline degrade, refinery migrations + self-update, embedded CJK fonts + HiDPI. Includes crate landing map and boot order.
- **`references/capability-roadmap.md`** — the five deep-water upgrades with priorities: P1 layered memory (LibSQL+FastEmbed, distiller, per-workspace scoping) + mid-turn steering + ambient probes; P2 visual grounding (screenshot tool, local VLM) + branch-and-verify planning over CoW forks; P3 remote headless workers via gRPC mesh. Includes `AgentState` deltas and anti-goals.
- **`references/slint-ui-contract.md`** — the Slint-side data contract: every `struct`, `global`, callback, and component property the bridge must implement, plus Catppuccin-Mocha theme tokens.
- **`references/workspace-layout.md`** — Cargo workspace factory layout: six crates, one-way dependency law (`app-desktop → agent-kernel → {context, sandbox, plugin, llm}`), root `Cargo.toml` + release profile, headless-testability rules, `UiCommand`/`UiEvent` channel contract.
- **`references/kernel-architecture.md`** — module-by-module spec of the Rust kernel: `AgentState` machine, actor topology, permission modes, hunk tracking, truncation table, compaction, crate choices, and acceptance checklist.

## Kernel System Prompt (embedded at build time)

The kernel's system prompt lives in `references/kernel-system-prompt.md`. At runtime:

1. Read it once (embed via `include_str!` or load from `assets/`).
2. Substitute `{{WORKSPACE_TREE}}`, `{{GIT_STATUS}}`, `{{PERMISSION_MODE}}`, `{{DATE}}` before sending.
3. Never expose internal file paths of the harness itself in the prompt.

## Non-Goals

- Not a general chat app — the UI is a **developer cockpit** (diff review first-class, tool status visible, token meter shown).
- No third plugin runtime — WASM + MCP only; embedded JS is a deferred v3 option.
- No cloud memory sync, no always-on screen capture, no auto-starting ambient actions — P2/P3 capabilities activate per `capability-roadmap.md` priorities.
- No multi-session orchestration in v1 — one workspace window per repo.

## Quick Sanity Checks

- `cargo build --release` produces **one** binary < ~25 MB, zero runtime deps (`otool -L` / `ldd` shows only system libs).
- Idle memory after 5 min: < 40 MB.
- Typing-indicator latency (SSE delta → visible glyph): ≤ 50 ms p95.
- Deny rules still block in `bypassPermissions` mode.
- Switching `active_provider` in the UI takes effect on the next turn with zero restart.
- `MockProvider` drives the full engine loop in tests — no network.
- A permission-less WASM test plugin physically cannot open a file or socket.
- Plugin tool results render as typed cards (`diff`/`table`/`markdown`) when `ui_type` is present.
- Sandboxed command cannot read `~/.ssh` or see `*_KEY` env vars.
- `rm -rf` audit produces a critical-red approval card; approval still executes inside CoW snapshot.
