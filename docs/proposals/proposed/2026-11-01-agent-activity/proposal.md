# Show what long-running agent work is doing

Accepted: 2026-11-01

## Problem

Three kinds of long-running work are invisible or blocking, and each shows the
same symptom to the user: the stream stops and there is nothing to read.

- **A todo is only open or done.** `TodoItem` carries `{ id, text, done }`
  (`crates/agent-kernel/src/tools/todo.rs:73`) and the tool offers only
  `add`/`list`/`done`/`undone`/`remove`/`clear`. A task the agent has started
  looks exactly like one it has not reached, and a subtask cannot be expressed
  as part of its parent, so a plan that is really two levels deep is written as
  one flat list.
- **A delegated subagent blocks its parent.** `delegate` awaits the child's
  first outcome (`crates/agent-kernel/src/tools/delegate.rs:1720`), and
  `answer_child` awaits the child's next outcome after answering
  (`delegate.rs:1920`). Until the child finishes or asks a question, the parent
  turn can do nothing else, and the user sees a single card that does not
  change. Parallel `tasks` already detaches its siblings through
  `ChildNotifier` (`delegate.rs:1408`), which the host wires to its notice
  channel and wake (`crates/husk-host/src/session.rs:1436`) — the mechanism for
  a non-blocking child exists, but only inside a read-only batch.
- **A queued tool call is not shown at all.** The engine dispatches a round's
  calls in a `for` loop and emits `ToolCallStarted` only when a call is about to
  run (`crates/agent-kernel/src/engine.rs:1286`, `:1382`), so calls waiting
  their turn have no event and no card. The stream therefore appears to stall
  whenever a round contains several calls. The UI's status set is
  `running`/`done`/`aborted`/`rejected`
  (`crates/app-tauri/frontend/src/features/chat/components/tool-chips/index.tsx:268`),
  and a finished call is matched to its start by tool name, `parent` and
  `childId` (`crates/app-tauri/frontend/src/features/chat/hooks/apply-event.ts:619`),
  so two same-name calls in one round can settle each other's card.

## Proposal

Deliver three changes in the order below. The kernel is the git dependency
`github.com/1Yie/korei` (`Cargo.toml:50`), whose local checkout is
`~/Workspace/korei`; parts 01 and 02 change both repositories, part 03 mainly
the kernel.

### 01 — todo gains an in-progress state and one level of nesting

`TodoItem` becomes `{ id, text, status, parent }`, where `status` is
`pending`/`in_progress`/`completed` and `parent` is an optional parent id. The
tool gains `start` (mark in progress) and an optional `parent` on `add`.
Removing a parent removes its children; completing a parent whose children are
still open is refused. The stored row keeps its shape — husk writes the tool's
own JSON to the session database (`crates/husk-host/src/todo_store.rs`), so an
existing row is read through a serde default that maps `done` onto `status`,
and no migration step runs.

The rendered block the model reads uses `[ ]`, `[~]` and `[x]`, with children
indented two spaces per level. Both renderers follow: `parseTodos`
(`crates/app-tauri/frontend/src/features/chat/components/todo-view/index.tsx:18`)
and the TUI's `parse_todos` (`crates/app-tui/src/events.rs:608`).

### 02 — a subagent can run in the background

`delegate` gains `background: true`. The child spawns, the call returns its
`child_id` immediately, and the child's question or final report arrives as a
notice through the existing `ChildNotifier`, waking the model the same way a
finished background job does. `answer_child` answers a parked child and returns
at once instead of awaiting the next outcome. Background children are not
restricted to read-only tools (owner's decision): the parent's own cancel flag
no longer reaches them, so the session needs a flag that outlives a turn — the
turn-scoped one is cleared at `begin_turn` (`crates/husk-host/src/session.rs:2145`).

Stopping is one action: `UiCommand::Cancel` (`session.rs:3300`) stops the
running turn and every background child of that session. A background child
that needs approval still raises the parent's card (`delegate.rs:395`).

### 03 — a queued tool call has a card before it runs

The engine emits one `ToolCallQueued` per call of the round before the dispatch
loop, and carries a `call_id` on `ToolCallQueued`, `ToolCallStarted` and
`ToolCallFinished` so a card settles against its own call rather than its
name. Dispatch stays sequential (owner's decision): this part makes waiting
visible, it does not make calls concurrent. The transcript keeps the queued row
and adds a `queued` status beside the existing four.

## Alternatives

- **Write the todo tool in husk and shadow the kernel's** (`ToolRegistry::register`
  replaces by name, `crates/agent-kernel/src/tools/registry.rs:111`). Rejected:
  the kernel would keep shipping a `todo` that every other host uses, so the two
  would drift, and the schema the model sees would depend on the host.
- **Give every child a background run and keep no blocking form.** Rejected: a
  parent that must have a report before it can continue (a review gate) has no
  way to wait, and the notice path cannot say "stop here until it lands".
- **Make a round's tools actually run concurrently.** Deferred, not refused:
  cancel, approval order and history ordering all change, and none of it is
  needed to remove the stall the user sees. `ToolCallQueued` does not depend on
  the decision.
- **Emit the queue from the host instead of the kernel.** Rejected: the host
  does not see the round's calls before the engine dispatches them.

## Acceptance criteria

- 01: `todo add` with `parent` creates a child that `list` shows indented under
  its parent; `start` shows it as in progress in the same `list`; a row written
  before this change still loads and reads as open or done. Both the desktop
  todo panel and the TUI strip show the three states and the nesting.
- 02: a `delegate` call with `background: true` returns a `child_id` while the
  child is still running, and the parent turn continues in the same round. The
  child's report arrives as a notice and the model answers it. `answer_child`
  returns without waiting for the child's next outcome. The stop button stops
  the turn and every background child of the session.
- 03: a round with three tool calls shows three cards as soon as the round
  starts, the first one `running` and the rest `queued`; each card settles on
  its own `call_id`, including two calls of the same tool in one round.

## Risks

- **The kernel's event protocol changes**, and its consumers are spread over the
  frontend (`apply-event.ts`), the Tauri forwarder, `app-gpui`, `app-tui`,
  `app-cli` and the host's tests. A missed consumer fails to compile, except on
  the frontend, where a missed `ToolCallQueued` would silently leave a card
  absent.
- **husk pins korei by branch** (`branch = "main"`), so joint work needs a
  `[patch]` to `~/Workspace/korei` and a pinned revision once the kernel change
  ships. A `[patch]` left behind would build husk against an uncommitted tree.
- **Background children may write while the parent writes.** The owner accepted
  this; two agents editing one workspace can overwrite each other, and the
  notice path does not order their writes.
- **A background child's approval card can wait forever.** Nothing wakes a
  session that is idle and has no notice, so an unattended card stays open until
  the user returns — the same as a foreground card, but the parent turn has
  already ended and will not show it as pending.
- **Autonomous follow-ups stop at three wakes** (`MAX_WAKES_IN_ROW`,
  `crates/husk-host/src/session.rs:57`). A chain longer than that waits for the
  user's next message.

## Checklist

- [ ] `01-todo-states.md` — in-progress state and one level of nesting.
- [ ] `02-subagent-background.md` — non-blocking `delegate` (after 01).
- [ ] `03-tool-queue.md` — a queued card and `call_id` correlation (after 02).
- [x] `04-tauri-consumers.md` — independently delivered desktop consumers.

As of 2026-11-03, the SDK producers, shared todo storage, host cancellation and
Tauri invocation-state fixes are committed on local task branches. Automated
coverage includes queued cancellation, same-name result correlation, shared
boards, background notices and session shutdown.

The owner limited this delivery to SDK, husk-host and Tauri. GPUI/TUI/CLI changes
are excluded and their existing local edits are preserved. Husk's local SDK
patch remains in the working configuration; SDK publication and a dependency-pin
update are outside this delivery. Parts 01–03 stay open for their remaining
cross-client and native manual acceptance.
