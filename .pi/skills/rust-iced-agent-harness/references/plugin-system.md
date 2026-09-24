# Plugin System Spec

Design goal: **the kernel stays minimal and fast; capability growth happens in plugins.** Two extension hooks — tools and context providers — plus optional UI cards. Host stability is non-negotiable: a misbehaving plugin can fail, never crash or hang the app.

## Plugin contract

A plugin = `manifest.json` (self-describing). Two independent halves: `entry` bridges to a server (`.wasm` or MCP server command), `capabilities` extends the kernel in the host. Either may be absent — a manifest that is only `capabilities.hooks` is a pure plugin that never connects anywhere.

```json
{
  "id": "github-workflow-tools",
  "name": "GitHub Actions Assistant",
  "version": "1.0.0",
  "kind": "wasm",                       // optional: which runtime `entry` uses ("wasm" | "mcp", default mcp)
  "entry": "plugin.wasm",               // optional: the bridge — wasm: file path; mcp: { "command": "npx", "args": [...] }
  "permissions": {
    "network": ["api.github.com"],      // host allowlist; empty = no network
    "filesystem": ["read:./.github/workflows"]
  },
  "capabilities": {
    "tools": [
      { "name": "fetch_ci_status",
        "description": "Fetch latest CI run status + failure logs for the current branch",
        "parameters": { "type": "object", "properties": { "branch": { "type": "string" } } } }
    ],
    "context_providers": [
      { "id": "workflow_linter",
        "description": "Inject .github/workflows YAML context into prompts" }
    ],
    "commands": [                          // slash commands → CommandRegistry
      { "name": "ci", "description": "Show latest CI status", "action": "tool:fetch_ci_status" }
    ],
    "hooks": [                             // local commands run by the host — work with or without `entry`
      { "event": "before_tool_execute", "filter": { "tool": "bash" } },
      { "event": "after_tool_execute" }
    ]
  }
}
```

Manifest rules:
- `permissions` are **capability-granting and exhaustive** — anything not declared is denied by the sandbox, not merely filtered at runtime.
- `kind` selects the runtime for `entry` only; the kernel maps both onto one `Plugin` trait. A manifest needs `entry` or at least one capability — a manifest with neither is refused.
- Tool `parameters` must be valid JSON Schema — validated at `register_plugin`, rejected plugins surface as load errors in the settings panel (never silently dropped).

## Plugin trait (`crates/agent-plugin/src/lib.rs`)

```rust
#[async_trait]
pub trait Plugin: Send + Sync {
    fn id(&self) -> &str;
    fn export_tools(&self) -> Vec<serde_json::Value>;                  // JSON Schema array for the LLM
    async fn call_tool(&self, name: &str, args: serde_json::Value) -> anyhow::Result<String>;
    async fn provide_context(&self, _workspace: &str) -> Option<String> { None }   // context hook
}
```

Both runtimes implement this trait, so `PluginManager`, the permission pipeline, and the tool router cannot tell WASM from MCP.

## Runtime selection — dual-track

| Runtime | Mechanism | Strengths | Costs | Use for |
|---------|-----------|-----------|-------|---------|
| **WASM (Wasmtime)** — primary | `.wasm` in-process sandbox, WASI with per-plugin preopens | <1 ms cold start, memory-safe, capability sandbox physically prevents host damage | plugins must compile to WASM (Rust/C/Go/Zig/TS-component) | official + compute-heavy + high-trust extensions |
| **MCP child process** | stdio JSON-RPC (`tools/list`, `tools/call`) | instant access to the entire MCP ecosystem (Node/Python servers) | one OS process each (~tens of MB), spawn latency | third-party/community tools, existing scripts |
| Embedded JS (`deno_core`/`boa`) | in-process script engine | lowest authoring barrier | +10–20 MB binary, weakest isolation | optional v3, not default |

**Rule: WASM + MCP only.** Wasmtime for trusted/high-perf, MCP for ecosystem breadth. No third runtime in v1.

## Wasmtime sandbox (`crates/agent-plugin/src/wasm.rs`)

```rust
pub struct WasmPlugin {
    id: String,
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    manifest: Manifest,           // drives WASI preopens + host-fn gates
}
```

Enforcement mapping (manifest → sandbox):

| Manifest permission | Enforcement |
|--------------------|-------------|
| `filesystem: ["read:./x"]` | WASI `preopen_dir` only that path, read-only; no preopen = no fs at all |
| `network: ["api.x.com"]` | plugins get **no raw sockets**; only a host `fetch(url)` fn that checks the host allowlist per call |
| (absent) | default-deny: no clock write, no env, no stdio beyond captured log pipe |

Operational guards:
- `Config::async_support(true)` + `consume_fuel(true)` — fuel budget per call kills infinite loops; per-call wall timeout (default 30 s) kills hangs.
- One `Store` per call (stateless plugin model); reuse `Engine`/`Module` across calls.
- Prefer **WIT/component-model** typed interfaces over raw `(i32,i32)->i32` pointer passing once the ABI stabilizes; until then, exchange length-prefixed JSON in linear memory.
- Trap → `Err` → tool result "plugin crashed" → session continues. Host never `unwrap`s plugin output.

## MCP bridge (`crates/agent-plugin/src/mcp.rs`)

MCP is **full JSON-RPC 2.0 over stdio**, not just "spawn and call". Spec target: protocol version `2024-11-05`. The client must implement the complete lifecycle — anything less fails to load real community servers.

### Lifecycle (mandatory sequence)

```text
spawn(command, args, env)          # Stdio piped stdin/stdout, stderr → log ring buffer
   │
   ▼
initialize {protocolVersion, capabilities, clientInfo}
   │◀── server returns ServerCapabilities {tools?, resources?, prompts?}
   ▼
notifications/initialized         # handshake complete
   │
   ▼
tools/list → cache schemas → export_tools()   # gated on server caps
resources/list, prompts/list                   # when caps declare them
   │
   ▼ (steady state)
tools/call, resources/read, prompts/get
notifications/tools/list_changed  → re-list, hot-reload without restart
notifications/cancelled, progress → forward to step card
   │
   ▼ (shutdown)
graceful: drain pending → kill child → reap
```

### Client skeleton

```rust
pub struct McpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    req_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    server_caps: ServerCapabilities,          // what the server declared at initialize
    _process: Child,
}
```

- **Background reader task**: `BufReader::new(stdout).lines()` loop demultiplexes — `id` present → resolve `pending[id]` oneshot; `method` present → notification → internal handler (`tools/list_changed` → re-list + refresh router). Per-call timeout on the oneshot (default 30 s).
- `call_rpc(method, params)`: allocate id → insert oneshot → `serde_json` line + `\n` → await.
- `call_tool`: `tools/call` → concatenate `content[]` items where `type == "text"`; non-text content (image/blob) → placeholder marker `[mcp: <type> content omitted]`.
- Env for the child comes **only** from manifest `env` + the sandbox env allowlist — never inherit host env.

### Three primitives, not one

| Primitive | Methods | Kernel mapping |
|-----------|---------|----------------|
| **Tools** | `tools/list`, `tools/call` | `export_tools()` / `call_tool()` — same as today |
| **Resources** | `resources/list`, `resources/read`, `resources/subscribe` | surface as a `context_provider`: readable URIs (logs, DB schema, files) injected as `<plugin_context>` or exposed via a `read_resource` tool shim |
| **Prompts** | `prompts/list`, `prompts/get` | surfaced to the UI slash-command menu — user-invoked templates, rendered client-side then submitted as a normal prompt |

Server capabilities gate everything: a server that didn't declare `resources` never gets `resources/list` called.

### Standard config format (community-compatible)

Support the de-facto `mcpServers` shape so users paste configs straight from Claude Desktop / community docs:

```toml
# config.toml — also accept mcpServers JSON blocks
[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "env:GITHUB_TOKEN" }

[mcp_servers.sqlite]
command = "uvx"
args = ["mcp-server-sqlite", "--db-path", "./test.db"]
```

Each `mcp_servers.*` entry synthesizes a `kind: "mcp"` manifest internally — no separate manifest file required.

### Safety posture

- Sandbox is weaker (full process) → MCP plugins get the **same permission-mode gates as `bash`** plus explicit install-time consent ("This plugin runs `npx foo-server` — trust?"); UI badge `kind: "mcp"`.
- Optionally runnable under `SandboxBackend` (manifest `sandboxed: true`) when the server is local-binary-safe.
- Stderr → log ring buffer, surfaced on plugin error cards.
- Child death = plugin `status: "error"` in settings; auto-restart with backoff (max 3) on transport failure, never mid-request.

## PluginManager (`crates/agent-plugin/src/manager.rs`)

```rust
#[derive(Default)]
pub struct PluginManager {
    plugins: HashMap<String, Arc<dyn Plugin>>,
    tool_router: HashMap<String, Arc<dyn Plugin>>,   // tool name -> owning plugin
}
```

- `register_plugin`: validate manifest → build runtime → `export_tools` → populate `tool_router`. Name collisions: namespaced as `plugin_id:tool_name` when two plugins claim the same bare name.
- `dispatch_tool_call(&ToolCall)`: route → `plugin.call_tool` → truncate output through the **same `TruncationConfig` as built-ins** (plugin output is not exempt from budgets).
- `collect_dynamic_contexts(workspace)`: fan out `provide_context` with per-plugin 2 s timeout; each result tagged `<plugin_context id="...">` and appended after the workspace skeleton in the prompt.
- Discovery: scan `~/.config/<app>/plugins/*/manifest.json` + `<repo>/.husk/plugins/*/` at startup; repo-local plugins require per-repo trust consent.
- Lifecycle: enable/disable persisted in config; disable = drop `Arc`, purge `tool_router` entries, kill MCP child. Hot-reload on manifest file change (debounced 500 ms).

## Extension points beyond tools (Hooks / Commands / Providers)

Tools alone make plugins **passive** — they only fire when the model decides to call them. Three more dimensions turn the plugin layer into an event-driven lifecycle system:

```
Providers  → data/context ingestion        (prompt-time, automatic)
Commands   → deterministic user control    (slash, bypasses the LLM loop)
Hooks      → lifecycle interception        (AOP over the state machine)
Tools      → action execution              (model-invoked)
```

### Hooks — middleware over the state machine (`agent-kernel/src/hooks.rs`)

```rust
pub enum HookAction {
    Continue,
    MutateMessages(Vec<ChatMessage>),   // rewrite the outbound context
    BlockTurn(String),                  // stop this turn with a reason
    InjectSystemNote(String),           // silent system-prompt addition
}

#[async_trait]
pub trait AgentHook: Send + Sync {
    fn id(&self) -> &str;
    async fn on_user_input(&self, _input: &str) -> anyhow::Result<HookAction> { Ok(HookAction::Continue) }
    async fn before_tool_execute(&self, _call: &mut ToolCall) -> anyhow::Result<bool> { Ok(true) } // false = veto
    async fn after_tool_execute(&self, _call: &ToolCall, _output: &mut String) -> anyhow::Result<()> { Ok(()) }
    async fn on_state_transition(&self, _old: &AgentState, _new: &AgentState) {}
}
```

Pipeline rules:
- Hooks run as an **ordered chain** (registration order); each gets a bounded timeout (default 2 s) — a slow hook degrades to `Continue`, never stalls the turn.
- `before_tool_execute` runs **before** the permission/audit gate — a hook can veto or rewrite args, but cannot *approve* (safety stays with permissions).
- `after_tool_execute` output mutation is capped by the same TruncationConfig; hook output is marked in UI (`⚡ hook <id>` line under the step card).
- A plugin hook is a **local command** (`manifest.capabilities.hooks[].run`): the kernel spawns it per intercepted event with a JSON payload on stdin and reads a JSON verdict from stdout, bounded by the hook's own timeout. Interception happens in the host, so an MCP plugin can declare hooks too (its MCP connection is irrelevant to them) — WASM plugins export `agent_*` fns instead once that runtime lands, mapping onto the same `AgentHook` trait.
- A hook is arbitrary local code, so it inherits the plugin's trust gate and the MCP child-process posture: `env_clear` + declared env only, secret-shaped `env:VAR` refused, `kill_on_drop`, Linux `PR_SET_PDEATHSIG`.

#### Hook wire contract (`crates/agent-plugin/src/hooks.rs`)

```jsonc
// manifest.json
"capabilities": {
  "hooks": [{
    "event": "before_tool_execute",  // on_user_input | before_tool_execute
                                     // | after_tool_execute | on_state_transition
    "filter": { "tool": "bash" },    // optional: tool events match `tool`,
                                     // transitions match `state`
    "run": { "command": "./guard.sh", "args": [], "env": {} },  // relative → plugin dir
    "timeout_ms": 2000               // optional, default 2000, capped 10000
  }]
}
```

| event | stdin | stdout verdict |
|---|---|---|
| `on_user_input` | `{event, input}` | `{"action":"continue"}` / `{"action":"block","reason"}` / `{"action":"inject","note"}` |
| `before_tool_execute` | `{event, tool, args}` | `{"action":"continue"}` / `{"action":"veto","reason"}` / `{"action":"rewrite","args"}` |
| `after_tool_execute` | `{event, tool, args, output}` | `{"action":"continue"}` / `{"action":"rewrite_output","output"}` |
| `on_state_transition` | `{event, old, new}` | ignored — fire-and-forget |
| `on_response` | `{event, text}` | `{"action":"rewrite","text":"..."}` — the assistant's final answer, mutable before it lands |

- Degrade rule is the in-process contract: non-zero exit, timeout, unparsable JSON, or an unknown `action` → `Continue`. A hook can never approve; `before_tool_execute` still runs ahead of the permission gate.
- `MutateMessages` has no wire spelling in v1 — serializing the whole outbound context across a process boundary is a bigger contract than the interception is worth today. Built-in `AgentHook`s still have it.
- Chain order = plugin discovery order, then declaration order within one manifest. Timeout is per hook, not per event.
- A repo-local manifest (`<repo>/.husk/plugins/*`) stays inert — hooks included — until trusted (`trust_mcp` → `plugin-trust.json`), because a hook is local code and `git clone` + open must not run it.
- Command resolution: a bare name (`python3`) goes to `PATH`; a path-shaped one (`./guard.sh`, `scripts/guard.sh`) resolves against the plugin's own directory, which is also the child's cwd. There is no shell layer — a `\|`, `&&` or `$VAR` in `command` is an argument, not syntax (wrap it in `sh -c "…"` yourself if you want one).
- **Not yet enforced:** `sandboxed: true` is declared, surfaced in the settings card, and otherwise ignored — hook commands (like MCP children) run as plain children today. Wiring them through `SandboxBackend` is the next hardening step.

#### Writing one

`~/.config/husk/plugins/git-guard/manifest.json` + `guard.sh` next to it (chmod `+x`):

```jsonc
{
  "id": "git-guard",
  "name": "Git Guard",
  "capabilities": {
    "hooks": [{
      "event": "before_tool_execute",
      "filter": { "tool": "bash" },
      "run": { "command": "./guard.sh" },
      "timeout_ms": 1000
    }]
  }
}
```

```sh
#!/bin/sh
# stdin: {"event":"before_tool_execute","tool":"bash","args":{"command":"git push --force"}}
payload=$(cat)
case "$payload" in
  *"push --force"*) echo '{"action":"veto","reason":"force-push needs a human"}';;
  *)                echo '{"action":"continue"}';;
esac
```

The same script can rewrite in place instead: `{"action":"rewrite","args":{"command":"git push"}}` — the permission gate and the tool both see the rewritten arguments (the step card still previews what the model asked for).

#### SDKs (`sdks/`)

`run.command` is language-agnostic — anything the kernel can spawn works. `sdks/python/husk_hooks.py` and `sdks/ts/husk-hooks.ts` are zero-dep single files to vendor next to the manifest; they wrap stdin → handler → verdict:

```python
from husk_hooks import on, run, veto

@on("before_tool_execute")
def guard(p):
    if "push --force" in p["args"].get("command", ""):
        return veto("force-push needs a human")

run()
```

```jsonc
"run": { "command": "python3", "args": ["guard.py"] }   // or "bun", ["guard.ts"]
```

### Commands — deterministic slash control (`agent-kernel/src/commands.rs`)

Input starting with `/` is intercepted by `CommandRegistry` **before** the ReAct loop — zero tokens spent:

```rust
#[async_trait]
pub trait SlashCommand: Send + Sync {
    fn name(&self) -> &'static str;            // "diff", "model", "clear"
    fn description(&self) -> &'static str;
    async fn execute(&self, ctx: &CommandContext) -> anyhow::Result<CommandResult>;
}

pub enum CommandResult {
    Reply(String),                  // local echo, no LLM call
    ControlAction(ControlOp),       // reset session, switch model, undo turn…
    FeedToAgent(String),            // wraps result into a normal prompt
}
```

- Built-ins ship first: `/clear`, `/diff`, `/model`, `/compact`, `/undo`.
- MCP `prompts/list`+`prompts/get` maps here: server prompt templates surface as `/plugin_id:name` commands → `FeedToAgent`.
- WASM plugins export `command_execute(name, args_json)` → registered under `plugin_id:name`.
- Unknown `/x` falls through to the LLM as normal text (with a hint the command wasn't found).

### Context providers — modular prompt assembly (`agent-context/src/providers.rs`)

Refactors the hardcoded workspace/git/memory injection into a trait so plugins join the same pipeline:

```rust
#[async_trait]
pub trait ContextProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn provide_context(&self, workspace: &Path) -> Option<ContextChunk>;
}

pub struct ContextChunk {
    pub priority: u8,          // higher survives compaction/budget pressure
    pub title: String,         // "Git Diff", "Memory: 3 facts"
    pub content: String,
    pub estimated_tokens: usize,
}
```

- Built-in providers (workspace skeleton, git status, memory recall) become ordinary registrations — **the kernel treats its own context the same as plugin context**.
- Assembly order: sort by `priority` desc → fill the context budget → remainder dropped with a UI-visible note (context badges show what was injected vs. dropped).
- MCP `resources/list`+`resources/read` maps here: readable URIs become provider chunks; the plugin's `provide_context` in the base trait is the WASM equivalent.

### Runtime → extension-point matrix

| Point | WASM plugin | MCP plugin | Built-in |
|-------|------------|------------|----------|
| Tools | `export_tools` | `tools/call` | registry |
| Providers | `provide_context` | `resources/*` | workspace/git/memory |
| Commands | `command_execute` | `prompts/*` | `/clear` `/diff` `/model`… |
| Hooks | `agent_*` fns (with the WASM runtime) | `run` command ✅ | masking, audit, distill |

With this matrix the harness consumes **all three MCP primitives** (tools + resources + prompts), not just tools.

### Phased rollout

1. **Commands first** — cheapest, immediate UX win; built-ins only, then open to plugins.
2. **Providers second** — refactor existing injection into the trait; plugin `provide_context` already exists.
3. **Hooks** — landed as the local-command transport (above). WASM `agent_*` exports stay pending the WASM runtime; every hook is still a per-turn latency + failure surface, so keep the chain short.

## Kernel integration points

- **Tool dispatch order**: built-in registry first, then `plugin_router` — built-in names are reserved and can never be shadowed.
- **Permission pipeline**: plugin tools pass through the same modes as built-ins; `readonly` defaults to `false` for plugins (conservative). A plugin requesting a capability beyond its manifest → `AgentState::AwaitingPluginConsent { plugin_id, capability }` → native dialog → allow once / allow always (writes back to manifest trust store) / deny.
- **New state**: `AwaitingPluginConsent { plugin_id: String, capability: String }` — parallel to `AwaitingToolConfirmation`, same suspend/resume plumbing.
- **Hunk tracker**: plugin-triggered file writes still record via `RecordAgentWrite` with `origin: plugin_id` for attribution and undo.

## UI contract additions (see iced-ui-contract.md)

- `PluginInfo` struct + `Bridge.plugins` list → settings panel (enable toggle, permission list, kind badge, reload).
- `Bridge.approve_plugin_capability(plugin_id, capability, always)` callback.
- **Typed result cards**: plugin tool results may carry `{"ui_type": "diff"|"table"|"markdown", "data": ...}` — the UI dispatcher renders the matching card instead of plain text. Unknown `ui_type` falls back to markdown. Keep the v1 vocabulary to those three.
- Consent dialog text pattern: `Plugin "{name}" requests {capability} — allow once / always / deny?`

## Security non-negotiables

- Manifest permissions are the **maximum** a plugin can ever get; runtime escalation always asks the user.
- No plugin API exposes the host's tokio runtime, file descriptors, or other plugins' state.
- Plugin tool calls appear in the UI exactly like built-in ones (status card, timing, truncated output) — no silent background execution.
- Repo-local plugins are inert until explicitly trusted for that repo (path-keyed consent store).

## Acceptance checklist

- [ ] WASM plugin with `network: []` physically cannot resolve DNS (verified by hostile test plugin)
- [ ] WASM plugin with no fs permission gets trap on any path open
- [ ] MCP plugin crash → error card, session alive
- [ ] Plugin tool output obeys 40 KB truncation
- [ ] `plugin_id:tool` namespacing resolves collisions deterministically
- [ ] Consent dialog deny → tool result "denied by user", agent loop continues
- [ ] Disable plugin → its tools vanish from next LLM request's schema list
- [ ] Full MCP handshake: `initialize` → `notifications/initialized` → `tools/list` against a real server (e.g. `@modelcontextprotocol/server-filesystem`)
- [ ] `notifications/tools/list_changed` → tools hot-reload without restart
- [ ] Server-declared caps gate: server without `resources` never receives `resources/list`
- [ ] `mcpServers` TOML entry boots a community server with only `command`/`args`/`env`
- [ ] MCP child killed → error card + session alive; backoff-restart ≤3
