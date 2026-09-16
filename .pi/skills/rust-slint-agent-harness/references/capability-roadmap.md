# Capability Roadmap — the five deep-water gaps

The skeleton (Rust+Slint, state-machine ReAct, pluggable providers, dual-track plugins, layered sandbox) solves **fast, light, safe**. This document specs what makes it **indispensable**: long-lived project cognition (Memory), steerable execution (Steering), system sight (Sensing), exploratory planning (MCTS), and remote muscle (Mesh). Each section: contract → state-machine deltas → UI deltas → testable acceptance.

Ordering legend: **P1** = ship in v1.x (high leverage, contained scope) · **P2** = v2 (new surface area) · **P3** = exploratory (architecture must not preclude).

---

## 1. Memory — hierarchical & evolving (P1)

### Layers

| Layer | Content | Lifetime | Store |
|-------|---------|----------|-------|
| Working memory | current turn's plan, open questions, file pin-set | turn | in `SessionActor` |
| Session memory | compacted history (existing) | session | in-process |
| **Episodic memory** | "what happened": task → outcome → files touched → user corrections | forever | `memory/episodes` table |
| **Semantic memory** | distilled facts: conventions (`errors via AppError, never unwrap`), architecture notes, env quirks | forever, evolving | `memory/facts` + vector index |
| **User persona** | style prefs ("no comments", "functional style"), approval patterns | forever | `memory/persona` |

### Store

- **LibSQL (embedded SQLite)** for structured tables + **FastEmbed-rs** (ONNX, ~30 MB model, in-process) for embeddings. DuckDB acceptable alternative; do NOT run a server DB — single-file, single-binary thesis holds.
- DB at `~/.local/share/<app>/memory.db`; per-repo partitioning by `workspace_id = hash(canonical_root)` so facts stay scoped to the project.

### Write path (the "distiller")

- At `Finished`/`Failed` of a turn, spawn a **low-priority background task** (separate cheap model or the active one, configurable): summarize turn → extract candidate facts → dedupe vs. existing (`cosine > 0.92` → merge, else insert) → write episode + facts. Never blocks the turn.
- **User-correction signal is the highest-value extractor input**: a `deny_tool` with reason, or a steering message, produces a candidate fact tagged `confidence: 0.6`; repeated confirmation raises it; contradiction lowers → decay.

### Read path (prompt-time retrieval)

- Before each turn: embed user prompt → top-k (k=8) semantic facts + last-3 episode summaries for this workspace → inject as `<memory>` block after workspace skeleton, capped 2 KB.
- Persona block always injected (≤500 B).

### Non-negotiables

- All memory is **local, inspectable, deletable**: workspace panel exposes Facts/Persona lists with delete buttons; `--forget <workspace>` wipes a project's rows.
- Secrets never distilled: the sanitizer denylist patterns apply to distiller output.
- Memory retrieval is advisory context — the LLM is told in the system prompt that memory may be stale; `smart_read` ground truth always wins.

## 2. Steering — mixed-initiative interaction (P1)

### The problem

Linear ReAct = only Cancel or wait. Fix: **state hot-patching** — inject a user message mid-turn without resetting the state machine.

### Mechanism

- `SessionActor` checks the input channel **between tool calls and every N streamed chunks** (N≈50 tokens). New `UiCommand::Steer(String)` → inject as a user message *at the current position*: "The user interrupted: {text}. Adjust the current plan accordingly — completed tool calls remain valid."
- `AgentState` gains no new variant for steering itself — `Reasoning` already covers the re-plan; a `steered: bool` flag on the turn marks provenance.
- LLM contract addition (system prompt): on injected steering, briefly acknowledge the new direction and continue; never restart completed work.

### Ambient probes (opt-in, per-repo consent)

- `notify` watcher + language-server diagnostics → **idle detector**: after a user-driven build/test outside the agent produces errors, push a quiet status-bar suggestion: "3 compile errors detected — fix now?" (one click → turns into a normal prompt). Never auto-starts a turn; suggestions expire after 60 s and cap at 1 per 5 min.

### UI deltas

- Input box **never fully disables** during a turn — it morphs to "Steer…" mode (Enter injects steering, Esc+Enter or explicit button cancels).
- `AgentState` display adds `steered` marker on the message.

## 3. Sensing — eyes & cross-app context (P2)

| Sense | Mechanism | Platform | Gate |
|-------|-----------|----------|------|
| Focused-window screenshot | `CGDisplayStream` / PipeWire / `Graphics Capture` | macOS / Linux-Wayland / Win | per-capture user grant or pre-approved region |
| UI diff verify | capture → compare against expected state after agent UI edits | all | auto when `has_ui_change` flag on turn |
| Local VLM | llama.cpp server or `candle` running Moondream/Qwen2-VL-mini | all | registered as a normal `LlmProvider`-adjacent `VisionProvider` trait |

- `VisionProvider` is a separate trait (not crammed into `LlmProvider`): `describe(image) -> String`, `verify_ui(screenshot, expectation) -> Verdict`.
- New tool: `screenshot { region? }` — readonly, but **consent-gated every time unless a region is pre-authorized**; screenshots enter context as images when the model supports vision, else VLM-described text.
- Privacy: capture path runs through the same audit as `bash` elevated; status bar shows a camera-dot while any capture capability is armed.

## 4. Planning — lookahead & branch pruning (P2)

Greedy ReAct hits walls and burns budget. With CoW snapshots already cheap (~20 ms), the kernel can **fork-and-verify**:

```text
ambiguous/refactor decision point
   ├─ fork snapshot A → candidate plan A (edit + `cargo check`)
   ├─ fork snapshot B → candidate plan B
   └─ score: error count, warnings, test pass → adopt winner, prune loser (drop snapshot)
```

- **Branch points are explicit**: triggered only by (a) a planner tool call `propose_branches`, or (b) dead-end detection — same error signature 3× in a row → auto-branch prompt to the model.
- Implementation reuses `SnapshotMode::Cow` + `SandboxBackend`; branches run sequentially first (CPU-bound), parallel only when resource headroom allows.
- `AgentState::Branching { candidates: u8 }` — UI shows a fork card with per-branch outcome; adopted branch's hunks merge via the standard confirmation path.
- Budget: branching consumes real tokens — cap at 2 branches × 1 verification round unless user raises it; log spend in stats.

## 5. Mesh — remote headless workers (P3)

- Split kernel into **Commander** (local Slint app) ↔ **Worker** (`<app> --headless` on a remote host): same `SandboxBackend`, `ToolRegistry`, `HunkTracker` run remotely; the LLM loop can run on either side (config: `loop_location: local|remote`).
- Transport: mTLS gRPC or QUIC; auth via Tailscale/WireGuard identity — never raw internet exposure.
- Streaming contract is already clean: `StreamChunk` and `UiUpdate` are serializable enums — mesh mode just changes their transport, not their shape. **This is why the actor/mpsc design pays off.**
- v1.x groundwork only: keep `SessionActor` behind a trait boundary so a `RemoteSessionActor` can slot in later without UI changes.

## State-machine additions (cumulative)

```rust
pub enum AgentState {
    // …existing…
    Branching { candidates: u8 },          // MCTS-style fork-and-verify (§4)
    AwaitingConsent { kind: ConsentKind }, // generalizes ToolConfirmation + PluginConsent + ScreenCapture
}
```

`ConsentKind::{Tool{name, summary}, Plugin{id, capability}, Capture{region}}` — one suspend/resume channel, three card renderers.

## Priority & acceptance matrix

| Capability | Pri | Headline test |
|-----------|-----|---------------|
| Memory | P1 | session 2 recalls "no comments" persona + `AppError` convention without re-prompting |
| Steering | P1 | mid-turn "skip file B" → plan adjusts, completed tool calls not re-run |
| Ambient probe | P1 | external `cargo build` failure → suggestion chip ≤2 s |
| Sensing | P2 | UI-edit turn auto-captures window, VLM verdict gates `Finished` |
| Planning | P2 | 3× repeated error → auto-branch; winner merged, loser pruned, stats show spend |
| Mesh | P3 | `UiUpdate`/`StreamChunk` round-trip over gRPC unchanged |

## Anti-goals

- No cloud memory sync in v1 — local DB only.
- Ambient probes never auto-start turns; a suggestion is a chip, not an action.
- No always-on screen recording — capture is event-scoped and consent-gated.
- Mesh does not weaken sandbox guarantees — remote workers run the identical `SandboxBackend`.
