# Workspace Layout Spec — factory-grade crate topology

Design goal: **strict one-way dependency flow, headless-testable kernel, UI as a replaceable adapter.** The kernel must never import Slint; the GUI is one consumer of the kernel's typed event stream.

## Crate topology

```text
desktop-agent/
├── Cargo.toml                     # workspace root: members, shared deps, release profile
├── Cargo.lock
└── crates/
    ├── app-desktop/               # [presentation/distribution] Slint GUI shell — the ONLY Slint-dependent crate
    │   ├── build.rs               # slint-build compiles ui/
    │   ├── ui/                    # declarative .slint (see slint-ui-contract.md)
    │   └── src/
    │       ├── main.rs            # boot: single-instance lock, tray, global hotkey
    │       ├── bridge.rs          # UiCommand ⇄ UiEvent channel wiring, invoke_from_event_loop
    │       └── throttler.rs       # 33 ms frame-aligned token coalescer
    │
    ├── agent-kernel/              # [orchestration] NO Slint, NO reqwest — pure async engine
    │   └── src/
    │       ├── engine.rs          # AgentState machine, ReAct loop
    │       ├── session.rs         # SessionActor, history, persistence
    │       ├── planner.rs         # lookahead/branch controller (P2)
    │       ├── steering.rs        # mid-turn injection + cancellation
    │       ├── permissions.rs     # modes, rules, consent flow
    │       ├── compaction.rs      # two-pass summarize, prefire
    │       ├── tools/             # built-in tool registry
    │       │   ├── registry.rs    # ToolSpec + dispatch; plugin tools merge here
    │       │   ├── fs_patch.rs    # search_replace / apply_patch via similar
    │       │   ├── fast_grep.rs   # grep-searcher
    │       │   ├── fs_read.rs     # read_file / list_dir
    │       │   └── shell.rs       # bash tool → delegates to agent-sandbox
    │       └── events.rs          # UiCommand / UiEvent / AgentEvent enums (the UI contract in Rust)
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
app-desktop ──▶ agent-kernel ──┬──▶ agent-context
                               ├──▶ agent-sandbox
                               ├──▶ agent-plugin
                               └──▶ agent-llm
```

- **Downstream crates never depend upward.** `agent-llm` doesn't know what a tool is; `agent-sandbox` doesn't know what an LLM is.
- `agent-plugin` depends on `agent-llm` types only for `ToolCall` shape (or duplicate the 3-field struct — prefer dependency).
- The kernel talks to UI exclusively through `events.rs` enums (`UiCommand` in, `UiEvent` out) — a channel boundary, not a call boundary. This is what makes headless mode and the future gRPC mesh free.
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

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"        # no unwinding → smaller, faster; crash = process death + restart, never half-state
strip = true
```

## Rules of thumb (industrial floor)

1. **No C/C++ dynamic linkage** — `rustls-tls` mandatory; `default-features = false` on everything; audit `cargo tree` for `*-sys` crates pulling system libs before each release.
2. **Type-driven edges** — `serde_json::Value` may exist at wire boundaries only; it must deserialize into a concrete struct before crossing a crate boundary.
3. **Headless testability** — `agent-kernel` ships `tests/` driving the full ReAct loop through `MockProvider` + in-memory sandbox; CI runs `cargo test -p agent-kernel` with no display server.
4. **Kernel CLI exists** — `agent-kernel` gets a tiny `examples/headless.rs` (or a `app-cli` crate later) proving the engine runs without Slint; this is also the future remote-worker binary.
5. **UI is an adapter** — everything in `app-desktop` is translation: `UiEvent` → Slint models, Slint callbacks → `UiCommand`. Zero business logic above the channel.
6. **One binary ships** — `cargo build --release -p app-desktop` produces the single static artifact; other crates are libraries (except the future `--headless` worker mode in kernel).

## Migration note

Earlier references assumed a flat `src/` single crate. This layout supersedes it: `src/kernel/*` → `crates/agent-kernel/src/*`, `src/llm/*` → `crates/agent-llm/src/*`, `src/sandbox/*` → `crates/agent-sandbox/src/*`, `src/bridge/*` → `crates/app-desktop/src/{bridge,throttler}.rs`, `ui/` → `crates/app-desktop/ui/`. Build-table stage numbers are unchanged.
