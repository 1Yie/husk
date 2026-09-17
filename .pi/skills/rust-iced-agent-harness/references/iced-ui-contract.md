# iced UI Contract

iced is **Elm architecture** — a single state struct + reducer + pure
`view()`, not a retained-widget toolkit. The loop is:

```
App (state)  ──update(&mut self, Message)──►  mutates self, returns Task
        └────view(&self)───────────────────►  derives a widget tree each frame
```

All UI code lives under `crates/app-desktop/src/ui/`. The kernel never
imports iced; `app-desktop` is the only iced-dependent crate.

**Visual language**: `codex-desktop-design.md` — sidebar session list +
execution stream (user text right-aligned plain, agent left) + tool capsules
with **inline** approve/deny at the tool call. Approvals do NOT open a side
panel — the awaiting tool's capsule grows an accept/deny row.

## The App state

```rust
/// Everything `view()` needs — iced owns it on the main thread.
pub struct App {
    pub sessions: Vec<SessionRow>,        // sidebar list
    pub messages: Vec<MessageRow>,        // chat stream
    pub active_steps: Vec<StepRow>,       // tool capsules (flat, mutated in place)
    pub pending: Option<ApprovalRow>,     // Some → inline approve/deny on the capsule
    pub changed_files: Vec<String>,       // hunk tracker feed
    pub stats: StatsRow,                  // tokens / model / mode / degraded
    pub sandbox_unsafe: bool,
    pub is_active: bool,                  // mid-turn → loading dots
    pub suggestions: Vec<SuggestionRow>,
    pub model_options: Vec<ModelRow>,
    // … memory_facts, plugins, branches, slash_commands, context_badges
    pub input: String,                    // composer text
    pub stream_scroll: scrollable::Id,    // pin-to-bottom anchor
    // non-render internals:
    cmd_tx: mpsc::Sender<UiCommand>,      // UI → kernel
    decision: Arc<Mutex<Option<bool>>>,   // approval slot (kernel reads it)
}
```

Row structs (`SessionRow`, `MessageRow`, `StepRow`, `ApprovalRow`, …) are
plain Rust data — `role/text/reasoning/streaming`, `name/state/detail`,
`tool_name/diff`…
Keep them **flat** — `StepRow` holds ids, not nested models; mutation is
`Vec` index inside `update()`.

## The Message enum (the only way state changes)

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // UI → kernel intents
    Submit,                                   // send self.input
    InputChanged(String),
    Approve(usize /*step_id*/),
    Deny(usize /*step_id*/),
    Steer,                                    // same input box, mid-turn
    SelectSession(i64),
    NewSession,
    // kernel → UI (from the event subscription)
    KernelEvent(UiEvent),                     // wrapped UiEvent from agent-ipc
    Tick,                                     // 60Hz frame for dots/caret
}
```

`update()` matches on `Message`, mutates `self`, returns `Task::none()`
(usually) — e.g. `Submit` → `self.cmd_tx.send(UiCommand::Prompt{…})` +
push a `MessageRow`. `KernelEvent(ev)` → the bridge's `apply_event(ev)`
logic inlined as a method on `App` (mutate `messages`/`active_steps`).

## The kernel → UI subscription (the iced equivalent of the mpsc drain)

The kernel emits `UiEvent`s on a tokio mpsc (kernel-rt thread). iced needs
them on the main thread as `Message::KernelEvent`. Two clean patterns:

- **`Subscription::run`** — a stream that polls a `std::sync::mpsc::Receiver`
  and yields `Message::KernelEvent` per item; a forwarder thread moves
  `tokio` events into the std-mpsc.
- **`Task::perform`** batches — a `Subscription` emits one `Tick` at ~60 Hz;
  `update(Tick)` drains the whole std-mpsc in one pass (batch-friendly).

Either way: the tokio thread NEVER touches `App`. `update()` on the main
thread is the only place state mutates.

## The streaming lifecycle (same 3-step as before)

1. `KernelEvent(UserPrompt)` → push `MessageRow{role:user}` + a
   `MessageRow{role:agent, streaming:true}` to reserve the reply row.
2. `KernelEvent(TextDelta/ReasoningDelta)` → mutate that row's
   `text`/`reasoning` in place. `StreamThrottler` already batched to ~30fps
   before the event crosses — never emit a `Message` per SSE chunk.
3. `KernelEvent(AssistantMessage/Finished)` → `streaming:false` on the last
   agent row. Tool capsules mutate `state` by `step_id` index.
4. Scroll pin: after mutating, return `scrollable::snap_to(stream_scroll,
   RelativeOffset::END)` so the stream stays pinned unless the user scrolled.

## Components → iced widgets

| Component | iced shape | Notes |
|-----------|-----------|-------|
| `SessionSidebar` | `column!` of `button`/`text` rows, `container` styled `BG_PANEL` | session rows + New button; `SelectSession` fires `UiCommand` |
| `ExecutionStream` | `scrollable(column![…])` of message widgets | user row = `container(text(..)).align_right(Fill)`; agent = left `text` + `toggler`-collapsed reasoning + `▮` caret while `streaming` |
| `CodexStepCapsule` | fn `-> Element` per `StepRow` | `row!` 28px; `awaiting_confirm` appends a `row![allow_btn, deny_btn]` (56px) — no side panel |
| `DiffStage`/diff rows | `scrollable(column![…])` over `Vec<DiffLineData>` | inline-expanded on the capsule or a slide-over `pane_grid`/`stack` for >50 lines — never a permanent third column |
| `LoadingDots` | 3 `text("●")`/`canvas` dots, opacity animated by `Tick` | driven by `is_active` + `Message::Tick` |
| `InputCapsule` | `text_input` + `button` in a `container` styled `BG_CARD`, radius 8 | `InputChanged`/`Submit`; while `is_active` placeholder = "Steer the agent…" and `Submit`→`Steer` |

## What iced gives you (use it)

- **`row!`/`column!`/`container`/`scrollable`** — the whole layout vocabulary.
- **`iced::widget::text_input`/`button`/`toggler`/`combo_box`** — real
  inputs, no hand-rolled text boxes.
- **`Subscription`** (`keyboard`, `time::every`, `events`, custom stream) —
  the frame pump + hotkeys + the kernel event feed.
- **`Task`** (`perform` async, `batch`, `done`) — fire a kernel command, do
  async work, animate.
- **Styling**: `container::Style`/`button::Style`/`text::Style` closures —
  theme via the `theme` module, no stylesheet.
- **Canvas** for the loading dots / custom capsules if text isn't enough.
- **wgpu under the hood** — GPU-rendered, but pure Rust end to end.

## Binding rules for the iced side

- `App` owns everything; `update()` is the only mutator; `view()` is pure
  `&self → Element`.
- `active_steps` mutate by index — never rebuild the `Vec` per tick.
- `pending: Option<ApprovalRow>` — `Some` → that `StepRow` renders its
  approve/deny row; `Approve`/`Deny` write the kernel `decision` slot (the
  `Arc<Mutex<Option<bool>>>` the SessionActor polls — it can't queue behind
  the `await`ed turn on the cmd channel).
- `stats.tokens_used` updates on `StreamChunk::Done` usage, not per token.
- `is_active` mirrors `AgentState::is_active()` — drives the loading dots
  and the input's Steer-vs-Submit mode.
- `Snap_to` the stream `scrollable::Id` on new content; stop pinning when
  the user scrolls up (track scroll offset in `App`).

## Implementation phasing

1. **Mock data first** — `App::demo()` populating two messages + one
   `awaiting_confirm` step so layout/theme is verified before the kernel
   attaches.
2. **Core callbacks first** — `Submit`, `Approve`/`Deny`, `Steer`
   end-to-end before secondary panels.
3. **The subscription pump** — tokio → std-mpsc → `Subscription` →
   `update(KernelEvent)` is the load-bearing piece; get `TextDelta` flowing
   before styling.
