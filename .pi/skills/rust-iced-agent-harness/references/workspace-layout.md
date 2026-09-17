# Workspace Layout Spec — factory-grade crate topology

Design goal: **strict one-way dependency flow, headless-testable kernel, UI as a replaceable adapter.** The kernel must never import iced; the GUI is one consumer of the kernel's typed event stream.

## Crate topology

```text
desktop-agent/
├── Cargo.toml                     # workspace root: members, shared deps, release profile
├── Cargo.lock
└── crates/
    ├── app-desktop/               # [presentation/distribution] iced GUI shell — the ONLY iced-dependent crate
    │   └── src/
    │       ├── main.rs            # boot: iced::application(...).run(), single-instance lock
    │       ├── bridge.rs          # UiEvent → UiState mutations on main thread; UiCommand tx
    │       ├── throttler.rs       # 33 ms frame-aligned token coalescer
    │       ├── ui/
    │       │   ├── mod.rs         # RootView — the Render impl, layout skeleton
    │       │   ├── theme.rs       # CodexTheme → iced::Color consts
    │       │   ├── sidebar.rs     # session list
    │       │   ├── stream.rs      # message list + step capsules + input
    │       │   └── components.rs  # capsule / diff rows / loading dots
    │       └── (see iced-ui-contract.md)
    │
    ├── app-cli/                   # [distribution] headless binary — same kernel, `--headless` / remote worker
    │   └── src/main.rs
    │
    ├── agent-kernel/              # [orchestration] NO iced, NO reqwest — pure async engine
    │   └── src/
    │       ├── engine.rs          # AgentState machine, ReAct loop
    │       ├── session.rs         # SessionActor, history, persistence
    │       ├── planner.rs         # lookahead/branch controller (P2)
    │       ├── steering.rs        # mid-turn injection + cancellation
    │       ├── permissions.rs     # modes, rules, consent flow
    │       ├── compaction.rs      # two-pass summarize, prefire
    │       ├── tools/             # built-in tool registry
    │       │   ├── registry.rs    # ToolSpec + dispatch; plugin tools merge here
    │       │   ├── fs_patch.rs    # fuzzy_patch / apply_patch via similar (see native-tools.md)
    │       │   ├── fast_grep.rs   # grep-searcher
    │       │   ├── fs_read.rs     # smart_read (outline/range/search) / list_dir
    │       │   └── shell.rs       # bash tool → delegates to agent-sandbox
    │       └── channels.rs        # mpsc channel setup + ChildRegistry wiring
    │
    ├── agent-context/             # [perception/cognition]
    │   └── src/
    │       ├── workspace.rs       # ignore::WalkBuilder skeleton scan
    │       ├── git.rs             # git2 status/diff sniffing
    │       ├── budget.rs          # TruncationConfig, head/tail fold, fit_conversation_to_budget
    │       ├── hunks.rs           # HunkTracker: file_states, turn_index, undo/rewind
    │       └── memory/            # layered memory (P1)
    │           ├── store.rs       # libsql embedded DB, per-workspace partition
    │           ├── distill.rs     # post-turn fact extraction, confidence scoring
    │           └── embed.rs       # fastembed-rs ONNX vectors, top-k recall
    │
    ├── agent-sandbox/             # [execution isolation]
    │   └── src/
    │       ├── traits.rs          # SandboxBackend trait, SandboxConfig, CommandOutput
    │       ├── audit.rs           # pre-execution command risk classification
    │       ├── cow.rs             # reflink snapshots + merge-back (ioctl_ficlone/clonefile)
    │       └── backends/
    │           ├── linux_bwrap.rs # bubblewrap
    │           ├── linux_landlock.rs
    │           ├── macos_sbpl.rs  # sandbox-exec
    │           └── win_job.rs     # Job Objects
    │
    ├── agent-plugin/              # [extension ecosystem]
    │   └── src/
    │       ├── manager.rs         # PluginManager, manifest validation, tool_router
    │       ├── wasm.rs            # Wasmtime host (WASI capability sandbox)
    │       └── mcp.rs             # stdio JSON-RPC bridge
    │
    ├── agent-ipc/                 # [channel contract] UiCommand/UiEvent/AgentEvent + wire codec
    │   └── src/
    │       ├── events.rs          # the enums BOTH kernel and UI share
    │       └── codec.rs           # serde + length-prefixed framing (future gRPC reuse)
    │
    └── agent-llm/                 # [provider drivers]
        └── src/
            ├── types.rs           # ChatMessage/ToolCall/StreamChunk (normalized)
            ├── provider.rs        # LlmProvider trait, BoxStream
            ├── sse.rs             # eventsource-stream helpers
            ├── sampler.rs         # SamplerActor: retries, doom-loop, idle timeout, fallback
            ├── config.rs          # config.toml schema, env: indirection
            └── adapters/
                ├── openai_compat.rs
                ├── anthropic.rs
                └── mock.rs        # scripted provider for headless tests
```

## Dependency law (enforced by `cargo metadata` lint in CI)

```text
app-desktop ──┐
              ├──▶ agent-kernel ──┬──▶ agent-context
app-cli    ───┘        │          ├──▶ agent-sandbox
                  agent-ipc ◀─────┼──▶ agent-plugin
                       ▲          └──▶ agent-llm
            (events contract shared by kernel + both frontends)
```

- **Downstream crates never depend upward.** `agent-llm` doesn't know what a tool is; `agent-sandbox` doesn't know what an LLM is.
- `agent-plugin` depends on `agent-llm` types only for `ToolCall` shape (or duplicate the 3-field struct — prefer dependency).
- **`agent-ipc` owns the UI contract** (`UiCommand`/`UiEvent`/`AgentEvent`). Both `agent-kernel` and `app-desktop` depend on it — putting these enums inside `agent-kernel` would force the UI crate to reach into orchestration internals just to translate types, and would couple `app-cli`/future mesh binaries to kernel internals too. Kernel↔UI is a **channel boundary, not a call boundary**: kernel emits `UiEvent`, any consumer renders it. This is what makes headless mode and the future gRPC mesh free.
- **Tools are kernel-side** (`agent-kernel/src/tools/`): they orchestrate sandbox+context but the registry lives next to the engine that dispatches them.

## Root Cargo.toml

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
tracing = "0.1"

# zero C/C++ dynamic deps: rustls only, never openssl-sys
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
eventsource-stream = "0.2"

[profile.dev]
panic = "unwind"       # tests need unwinding; debug UX needs backtraces

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"        # no unwinding → smaller, faster; crash = process death + restart, never half-state
strip = true
debug = "line-tables-only"   # keep symbols for crash reports without full debuginfo
```

**Panic-profile hazard**: `panic = "abort"` in release silently breaks `#[should_panic]` tests *when run under `--release`* and turns recoverable `catch_unwind` paths into process death. Dev profile stays `unwind` so `cargo test` and debugging behave normally; only the shipped binary aborts. If a library ever needs `catch_unwind` in release (e.g. around plugin/WASM traps — Wasmtime handles its own, but audit third-party calls), that crate is wrong for `panic=abort` — document any exception explicitly.

## Rules of thumb (industrial floor)

1. **No C/C++ dynamic linkage** — `rustls-tls` mandatory; `default-features = false` on everything; audit `cargo tree` for `*-sys` crates pulling system libs before each release.
2. **Type-driven edges** — `serde_json::Value` may exist at wire boundaries only; it must deserialize into a concrete struct before crossing a crate boundary.
3. **Headless testability** — `agent-kernel` ships `tests/` driving the full ReAct loop through `MockProvider` + in-memory sandbox; CI runs `cargo test -p agent-kernel` with no display server.
4. **Kernel CLI exists** — `app-cli` is a real binary crate from day one (`--headless` mode, future remote worker); it shares `agent-ipc` events with `app-desktop` so both frontends are thin adapters over the same kernel.
5. **UI is an adapter** — everything in `app-desktop` is translation: `UiEvent` → `UiState` mutations, view callbacks → `UiCommand`. Zero business logic above the channel.
6. **One binary ships** — `cargo build --release -p app-desktop` produces the single static artifact; other crates are libraries (except `app-cli`, the future `--headless` worker binary).
7. **Feature hygiene** — optional heavy deps gate behind cargo features: `memory` (libsql+fastembed), `vision` (xcap+candle), `mesh` (tonic). `app-desktop` enables `memory` by default; `app-cli` can ship `default-features = false` for a minimal remote worker. Keeps the core lean when capability flags are off.
8. **Circular-dep guard** — `cargo metadata`-based CI lint asserts the DAG above; a PR that adds `agent-llm → agent-kernel` or `agent-context → agent-kernel` fails the lint, not the reviewer.
9. **Events live in `agent-ipc`, not kernel** — `UiCommand`/`UiEvent`/`AgentEvent` are the wire contract; kernel publishes/consumes, UI translates, `app-cli` reuses. No `app-desktop → agent-kernel::types` imports for event shapes.

## Build-order alignment with SKILL.md stages

| Stage | Crate(s) touched | Proves |
|-------|------------------|--------|
| 1 | `agent-context` | workspace scan + git sniff standalone-testable |
| 2 | `agent-llm` | provider abstraction + `MockProvider` headless driver |
| 3 | `agent-kernel` (tools) | registry dispatch, truncation budgets |
| 4 | `agent-kernel` (engine) + `agent-ipc` | ReAct loop emits `UiEvent` — headless `cargo test -p agent-kernel` |
| 5 | `app-desktop` | first iced render of a real stream |
| 6 | `agent-kernel` (permissions) + `agent-context` (hunks) | consent flow + undo |
| 7 | `agent-kernel` (compaction) | two-pass summarize |
| 8 | `agent-sandbox` | L2 isolation on one platform first |
| 9 | `agent-plugin` | WASM + MCP runtimes behind `Plugin` trait |
| 10 | `agent-context` (memory) + `agent-kernel` (steering) | recall injection + mid-turn steer |
| 11 | `agent-llm` (masking) + `agent-context` (store) + `app-desktop` (boot) | hardening |
| 12 | `app-desktop` | polish |

`app-cli` gets built alongside stage 4 — it is the headless proof vehicle, not an afterthought.

## Migration note

Earlier references assumed a flat `src/` single crate. This layout supersedes it: `src/kernel/*` → `crates/agent-kernel/src/*`, `src/llm/*` → `crates/agent-llm/src/*`, `src/sandbox/*` → `crates/agent-sandbox/src/*`, `src/bridge/*` → `crates/app-desktop/src/{bridge,throttler}.rs`, `ui/` → `crates/app-desktop/src/ui/` (iced views are Rust, not a markup DSL). Build-table stage numbers are unchanged; `events.rs` moved to `crates/agent-ipc/src/events.rs` (shared UI contract).
