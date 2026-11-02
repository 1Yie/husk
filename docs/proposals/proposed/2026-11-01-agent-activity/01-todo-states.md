# 01 — A todo has a state and one level of nesting

## Scope

Change the kernel's `todo` tool and both of husk's renderers so an in-progress
task and a subtask are expressible and visible. Nothing else in the tool changes.

## Inputs

- `crates/agent-kernel/src/tools/todo.rs` in `~/Workspace/korei` — `TodoItem`
  (`:73`), `TodoStore`, `TodoArgs` (`:87`) and the `exec` match over
  `add`/`list`/`done`/`undone`/`remove`/`clear`.
- `crates/husk-host/src/todo_store.rs` — stores the tool's JSON verbatim; no
  change expected, and it must stay a byte-for-byte store.
- `crates/app-tauri/frontend/src/features/chat/components/todo-view/index.tsx` —
  `parseTodos` (`:18`) and the checklist it renders.
- `crates/app-tui/src/events.rs` — `parse_todos` (`:608`) and `TodoItem` in
  `app.rs`, which the TUI strip draws up to four rows of.

## Data

`TodoItem` becomes:

```rust
pub struct TodoItem {
    pub id: usize,                       // stored, monotonic, never shown
    pub text: String,
    pub status: TodoStatus,              // pending | in_progress | completed
    pub parent: Option<usize>,
    pub note: Option<String>,            // one line on how it is being done
}
```

Two additions came from the owner after acceptance, and are part of this part:

- **A progress note** (`note`), written by `start` and dropped by `done`: the
  list shows `[~] #1 extract the parser — reading parser.rs`. Capped at 200
  characters because the list rides in every tool result the model reads.
- **Positional labels**: the block numbers roots by where they sit (`1`, `2`)
  and children by their place under that root (`2-1`), not by the stored id —
  a subtask must not push the next task to `#4`. A call names what the list
  printed; a stored id still resolves when no label reads the same.

Several items may be in progress at once, one per agent working, and the board
is installed _shared_: a delegated child marks the same rows its parent reads
instead of keeping a private list.

- A row written by the previous version carries `done: bool` and no `status`.
  Reading maps `done: true` to `completed` and `done: false` to `pending`, so an
  existing session opens with its list intact. The field is not written back.
- Only one level of nesting: `add` refuses a `parent` that is itself a child of
  another todo.
- `remove` of a parent removes its children.
- `done` of a parent whose children are not all `completed` is refused, and says
  which child is open.
- `start` sets `in_progress`. `undone` returns an item to `pending`.

## Rendering

The block the model reads keeps its header line and gains two states:

```text
1/3 done
[~] #1 extract the parser — reading parser.rs
  [ ] #1-1 move tests
  [x] #1-2 drop the old path
[ ] #2 update the guide
```

`parseTodos` and `parse_todos` accept `[ ]`, `[~]` and `[x]` and read leading
spaces as depth. A line they cannot parse is still ignored, as today, so an
unrelated `Cleared n todos.` header keeps working.

## Must not touch

- Approval, cancellation, delegation and the event protocol — that is 02 and 03.
- `SessionTodoStorage` semantics: it reads and writes one opaque row.
- The tool's `readonly`/`class` registration: it stays a `SessionMutation`.

## Done condition

- Kernel unit tests: legacy-row load, `add` with `parent`, refusal of a
  grandchild, refusal of completing a parent with an open child, `remove` of a
  parent removing children, `start`.
- husk tests: `parseTodos` over the three-state block with indentation, both
  renderers, and the empty/unparseable cases they already cover.
- `cargo test -p agent-kernel` in korei, and husk's host + frontend suites for
  the touched files, both green with the jobs limits `AGENTS.md` sets.
