# 02 — A subagent can run in the background

## Scope

Let `delegate` start a child and return while it runs, and let `answer_child`
answer a parked child without awaiting its next outcome. Writes are allowed in a
background child, and the session's stop action ends every background child.

## Inputs

- `crates/agent-kernel/src/tools/delegate.rs` — `DelegateArgs` (`:1622`),
  `exec` (`:1720`), `run` (`:1155`), `run_parallel` (`:1023`), `detach` (`:1408`),
  `StartedChild::first` (`:1358`), `answer_child_exec` (`:1920`), `abort_child`
  (`:714`), `route_child_approval` (`:395`) and `install_child_notifier` (`:365`).
- `crates/husk-host/src/session.rs` — the notifier install (`:1436`), the notice
  drain and wake (`:2097`), `begin_turn`'s cancel reset (`:2145`), and
  `UiCommand::Cancel` (`:3300`).
- `crates/app-tauri/frontend/src/features/chat/components/subagents-panel/index.tsx`
  and `tool-chips/agent-card.tsx` — the child card's `running`/`waiting` states.

## Kernel

`DelegateArgs` gains `background: bool` (default false). With it set:

- `exec` spawns through `SubagentSpawner::start`, arms the same detached wait
  `detach` uses (`:1408`), and returns text naming the child id instead of a
  report:
  `[subagent <label> (child_id <id>) is running in the background]` plus a short
  instruction to keep working and expect a notice.
- The child's first outcome — a question or the report — reaches the parent as a
  notice through `ChildNotifier`, exactly as a detached batch sibling does.
- `answer_child` gains the same flag. With it, the call delivers the answer,
  emits `subagent:resumed`, and returns without awaiting the next outcome; the
  next question or the report arrives as a notice.
- `background` and `tasks` together are refused: a batch already runs its
  siblings detached, and one call should not mean two waiting models.
- Writes are allowed (owner's decision). The existing rules still apply: an
  operation needing human approval raises the parent's card
  (`route_child_approval`, `:395`), and the child cannot delegate further.

## Session cancel

A background child outlives the turn that started it, so it cannot keep the
turn's cancel flag — `begin_turn` clears that flag (`:2145`) and the next turn's
`Cancel` would then mean something else. The session gains a second, session-
scoped flag:

- A background child is spawned on the session flag instead of `ctx.cancel`.
- `UiCommand::Cancel` sets the turn flag and the session flag, then ends every
  background child of the session (`abort_child` per id over
  `active_children`, `delegate.rs:436`).
- A normal turn start does not clear the session flag; only a new delegate call
  reads it, and it is cleared when the session has no background children left.

## Frontend

The child card already draws `running` and `waiting` from
`subagent:waiting`/`subagent:resumed`. What is added: a card that is `running`
while its parent turn has already ended stays visible with its own "后台运行中"
line, and the panel's stop action states that it stops background children too.

## Must not touch

- `MAX_PARALLEL_TASKS` and the read-only rule for `tasks`: a parallel batch stays
  what it is.
- `MAX_PARKED_CHILDREN` and `MAX_WAKES_IN_ROW`: the bounds stay, and a chain
  longer than three wakes still waits for the user.
- The `todo` tool and the tool-call event protocol (01 and 03).

## Done condition

- Kernel tests: `background: true` returns before the child finishes; the report
  arrives through the notifier; `answer_child` with `background` returns without
  awaiting; `background` plus `tasks` is refused; a session cancel aborts a
  background child.
- husk tests: the notifier text reaches the model as a notice turn; a session
  cancel ends a background child; the card shows a background child as running
  after its parent turn ended.
- Manual: with `cargo tauri dev` running, a `delegate` in the background lets the
  same turn continue, and the stop button ends it.
