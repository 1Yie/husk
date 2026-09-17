# Production Hardening Spec — six delivery-grade blind spots

The architecture covers fast/light/safe. This document covers **surviving real user machines**: quit signals mid-build, leaked credentials, DB lockups, flaky wifi, version upgrades, and blurry text on a 4K monitor. Each item: failure mode → mechanism → acceptance test.

---

## 1. Process-tree lifecycle (no orphans, ever)

**Failure mode**: user hits `Cmd+Q` mid `cargo test`, or the GUI crashes. `kill()` on the leader leaves grandchildren orphaned — MCP node servers and `npm install` keep eating CPU forever.

**Mechanism — per-platform parent-death wiring** (inside `agent-sandbox` spawn path AND `agent-plugin` MCP spawn path — both must use it):

| Platform | Mechanism |
|----------|-----------|
| Linux | `prctl(PR_SET_PDEATHSIG, SIGKILL)` pre-exec on every child; bwrap's `--die-with-parent` already covers sandboxed cmds — MCP spawns need the raw prctl |
| macOS/Unix | `setpgid(0,0)` at spawn → on exit send `SIGTERM` → 500 ms grace → `SIGKILL` to the **process group** (`kill(-pgid)`), not the pid |
| Windows | every child assigned to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — OS cascade-kills on host close, incl. crashes |

**Shutdown sequence** (`SessionActor` on `UiCommand::Quit` or last-window-close): cancel in-flight turn → `SIGTERM` all tracked children → drain MCP `pending` map (fail each waiter) → `SIGKILL` stragglers → flush WAL → exit. Tracked via a `ChildRegistry: Vec<Weak<Child>>` — nothing spawns without registering.

**Acceptance**: `kill -9` the app while an MCP server + `sleep 600` run → `pgrep` shows zero survivors within 1 s.

## 2. Secret management (keyring + egress masker)

**Failure mode**: API keys in plaintext `config.toml` get read by any local script; worse — a `bash`/`env` tool call echoes them into the prompt and upstream to the model provider.

**Mechanism**:

- **Storage**: `keyring` crate → macOS Keychain / Windows Credential Manager / Linux Secret Service. `config.toml` stores `api_key = "keyring:<service>/<account>"` alongside the existing `env:` indirection. Plaintext still allowed but warns at load.
- **Egress masker** (`agent-llm/src/masking.rs`): every outbound request body passes a replacer built from all resolved secrets + denylist patterns (`*_KEY`, `*_TOKEN`, `*_SECRET`, `ghp_*`, `sk-*`, `xai-*` shapes). Match → `[REDACTED_<kind>]`. Applied to messages AND tool results — secrets leak most often via `env`/`.env` file reads.
- Sandbox env sanitization (sandbox-model §env) is the first wall; the masker is the second — defense in depth.

**Acceptance**: tool result containing a configured key → LLM request body contains only `[REDACTED_API_KEY]`; secret material never hits `tracing` logs either.

## 3. SQLite concurrency (single-writer + WAL)

**Failure mode**: concurrent writes (session log + token ledger + memory distill) hit `database is locked (5)`; a blocked write stalls history reads → UI white screen.

**Mechanism**:

- `PRAGMA journal_mode=WAL; synchronous=NORMAL; busy_timeout=5000;` at open — mandatory, asserted in code not docs.
- **Single-writer actor**: one task owns the write connection; all writers (`session.rs`, `hunks.rs`, `memory/distill.rs`, telemetry ledger) enqueue `WriteOp` over mpsc. Reads use a small pool (`sqlx`/`r2d2`, max 4 conns).
- Non-blocking rule: **the UI/event loop never awaits a write** — writes are fire-and-forget with the write actor acking via oneshot only when the caller truly needs durability ordering.

**Acceptance**: 200 concurrent write ops + continuous reads → zero `database is locked`, p99 read latency < 20 ms.

## 4. Network resilience (partial-stream preservation)

**Failure mode**: TCP reset at token #800 of a stream → naive impl discards everything, user waits from zero.

**Mechanism**:

- **Stream salvage**: `SamplerActor` persists received chunks as they arrive (in-memory + write actor). On transport `Err` mid-stream: keep partial text visible, mark the message `interrupted`, retry with a **continuation prompt** ("Your previous response was cut off after: <tail 200 chars>. Continue from exactly that point.") — cheaper than full resample and preserves UX.
- **Offline degrade**: `ProviderFactory` marks providers `available: false` on connect-refused; if `fallback_chain` ends in a local provider (`ollama`/`llama.cpp`), degrade kicks in automatically with the `stats.degraded` UI state already defined. Fully offline + no local model → workspace tools (`grep`, `smart_read`, `list_dir`, AST index) still function — the agent can browse/index, just not reason.
- Retry policy from llm-provider-layer stays: backoff on transport/429/5xx, doom-loop guard, never retry 4xx.

**Acceptance**: `kill` the proxy mid-stream → partial text stays, retry banner appears, continuation resumes within backoff ≤ 3 attempts before surfacing `Failed`.

## 5. Versioning: schema migrations + self-update

**Failure mode**: v0.2.0 ships with a new `facts` column → old `memory.db` → panic on startup. Or: users never learn updates exist.

**Mechanism**:

- **Migrations**: `refinery` (or `sqlx::migrate`) embedded migrations run at boot inside the write actor before serving. Schema version in `PRAGMA user_version`. Rule: **migrations are additive-or-transform, never destructive**; a failed migration backs up `memory.db` → `memory.db.bak-<ver>` and starts fresh rather than crashing.
- **Self-update**: `self_update` crate checks GitHub Releases on a 24 h timer (config `updates.check = true`). Download → verify signature/checksum → **atomic replace** (write `.new`, `rename` over self — never patch in place). UI: quiet "Update ready — restart to apply" chip; never force-restart mid-turn.
- Config schema gets a `version` field too; unknown-version configs load with defaults + warning, never panic.

**Acceptance**: boot v0.2 binary against a v0.1 DB → migrates cleanly, zero data loss; update flow replaces binary while preserving `config.toml` + `memory.db`.

## 6. Font & HiDPI correctness

**Failure mode**: CJK glyphs render as tofu boxes on a machine without the right fonts; dragging the window from 4K/200% to 1080p/100% tears layout.

**Mechanism**:

- **Embedded fallback fonts**: `include_bytes!` a permissively-licensed CJK-capable font subset (e.g. MiSans/JetBrains Mono w/ fallback slice) → iced `Font` + `load_font` at startup as last-resort fallback. Keep it subsetted — full CJK fonts are 5–15 MB; use a coverage-reduced slice or accept a larger binary consciously (the one binary-size exception worth making).
- **DPI changes**: iced/wgpu handles scale-factor changes on Wayland/X11 correctly — GPU-rendered, no software-path env to pin. Test drag-across-monitors manually before each release.
- **Text metrics**: iced `px` units scale with DPI automatically (wgpu + cosmic-text shaping) — fixed 20px diff rows stay correct; do not hardcode pixel assumptions in measurement code.

**Acceptance**: fresh VM with no dev fonts → UI renders CJK + mono correctly; 4K↔1080p drag → no tearing, no offset clicks.

## Where these land in the crates

| Concern | Crate | Module |
|---------|-------|--------|
| Process-tree kill, pdeathsig, Job Object close | `agent-sandbox` | `traits.rs` spawn helpers + `ChildRegistry` in kernel `session.rs` |
| MCP child registration | `agent-plugin` | `mcp.rs` spawn path calls the same registry |
| Keyring + masker | `agent-llm` | `config.rs` (resolve `keyring:`), `masking.rs` (egress filter) |
| WAL + write actor | `agent-context` | `memory/store.rs` |
| Salvage + offline degrade | `agent-llm` | `sampler.rs` |
| Migrations + self-update | `app-desktop` | `main.rs` boot sequence (before window shows) |
| Fonts + DPI | `app-desktop` | `main.rs` font registration, backend pin |

## Boot order (main.rs)

```text
1. single-instance lock
2. config load (+ keyring resolution, version check)
3. DB open → WAL pragmas → migrations → write actor spawn
4. backend detect (sandbox) + provider build (env:/keyring: resolved)
5. font registration → window create
6. (background) update check, ambient probes
```
