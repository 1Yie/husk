# 03 — A queued tool call has a card before it runs

## Scope

Make a round's not-yet-dispatched tool calls visible, and settle every card
against its own call rather than its tool name. Dispatch stays sequential.

## Inputs

- `crates/agent-kernel/src/engine.rs` — the dispatch loop (`:1286`) and the
  `ToolCallStarted` send (`:1382`).
- `crates/agent-ipc/src/events.rs` — `UiEvent::ToolCallStarted` and
  `ToolCallFinished`, and the `Custom` payloads that already carry `child_id`.
- `crates/agent-llm-core/src/types.rs` — `ToolCall.id` (`:567`), the provider
  invocation id that already exists and is never empty once a call reaches the
  engine.
- Consumers to update: `crates/app-tauri/frontend/src/features/chat/hooks/apply-event.ts`
  (`:595`, `:619`), `crates/app-tauri/backend/ipc/forwarder.rs`,
  `crates/app-gpui/src/features/chat/{model,transcript,turns}.rs`,
  `crates/app-gpui/src/{bridge/owner,app/shell}.rs`,
  `crates/app-gpui/src/features/notifications.rs`, `crates/app-tui/src/{events,app}.rs`,
  `crates/app-cli/src/main.rs`, and the host's tests that assert on the events
  (`crates/husk-host/src/checkpoint.rs`, `checkpoint_tests.rs`,
  `tests/{stage4,hooks,delegate_children,delegate_dispatch,cancel_during_tool,approval_timeout}.rs`).

## Protocol

Two additions, both optional on the wire so an older reader keeps working:

- `UiEvent::ToolCallQueued { name, args_preview, call_id, parent, child_id }`,
  emitted for every call of the round immediately after the round's calls are
  known and before the first dispatch.
- `call_id: String` on `ToolCallStarted` and `ToolCallFinished`, carrying
  `ToolCall.id`.

The engine emits the queue in the same order the loop dispatches, so the cards
appear in call order and each turns from `queued` to `running` in place rather
than being replaced.

## Frontend

- `ToolChipRow.status` gains `queued`; its text is 排队中.
- `apply-event.ts` matches `ToolCallFinished` to `ToolCallStarted` by
  `call_id` when both carry one, and keeps the current name/`parent`/`childId`
  match only for an event without an id (a replayed older session).
- A round's activity preview (`toolActivityPreview`) names the running call and
  counts the queued ones, so the collapsed status row shows that work remains.

## Other consumers

`app-gpui`, `app-tui` and `app-cli` each already match start to finish by name;
they take the new `call_id` the same way so two same-name calls stop settling
each other's row. A `ToolCallQueued` they do not draw is still an event they must
accept without a match, so each keeps a no-op arm rather than dropping the event.

## Must not touch

- Dispatch order, cancel boundaries and history ordering: the loop stays
  sequential and the pre-dispatch cancel check (`:1289`) stays where it is.
- The `delegate` and `todo` tool schemas (01 and 02).

## Done condition

- Kernel test: a round of three calls emits three `ToolCallQueued` in order,
  each followed by exactly one `ToolCallStarted` with the same `call_id`.
- Host test: two calls of the same tool in one round each settle on their own
  `call_id`.
- Frontend test: three cards appear before the first finishes, the first is
  进行中 and the rest 排队中, and a same-name pair settles correctly.
- `cargo check` for each husk crate that consumes the events, plus the frontend
  typecheck and the chat suite.
