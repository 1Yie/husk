# app-gpui

Native desktop leaf package `husk-gpui`. GPUI-Kit owns the rendering/input substrate;
Husk owns composition, behavior, state projection, and lifecycle.

## Boundaries

`app -> features -> bridge -> husk-host`. Features also use `ui` resources.
GUI objects stay out of shared host and PTY crates.

`bridge::HostBridge` owns asynchronous access to one background `SessionManager`.
One receiver consumes its ordered events. Cancellation, approval and question
answers do not wait behind configuration or workspace I/O.

Every event and session cache entry uses `SessionKey { root, id }`. Snapshot history
is a baseline loaded once; subsequent metadata snapshots retain live projection.
`ChatModel` owns display projection, not persisted business state.
Session-specific model, thinking, permission, agent-mode and Fast commands carry
the same key and reject a stale target instead of mutating whichever session is
active when the owner queue executes.

Completion state and final assistant text may arrive separately. A completed
draft retains its identity until the final text replaces it or a new prompt begins.
Submitted text is retained by request until the host accepts it; a failed request
restores the text without discarding a newer draft or targeting another session.

`husk-terminal` is the owner-scoped PTY registry. `TerminalDeck` owns pane entities,
tabs and split trees. `TerminalPanel` owns input and native painting;
`TerminalScreen` owns VT state and palette. Window-level state owns visibility.
Hiding a deck never destroys its panes.

Workspace reads use the scoped `agent_context::git` API on the background
executor. Results carry a request generation. On Linux/Android/macOS, content reads are
directory-handle-relative, no-follow, nonblocking and bounded; unsupported
platforms and exhausted scan budgets report errors instead of incomplete success.

## Presentation

Use existing Husk fonts and Keyline SVG assets. `ui::layout` contains measured
logical sizes; `ui::theme` owns app token mapping. Prefer direct Kit components
over generic wrapper layers.

Streamed content updates its chat entity and changed rows; it does not rebuild
the entire shell per delta. Avoid I/O, blocking receivers and long-held host locks
in render/update callbacks.

Desktop notifications use fixed public copy, are suppressed for the visible
session, and fail closed when the persisted preference cannot be read. Their
bounded worker never receives prompts, session titles or tool output.

## Platform and verification

The native target accepts macOS 15+. Linux requires Vulkan and native development
libraries. Portal futures use the shared upstream async-io configuration rather
than mutually exclusive runtime features.

Run the dependency law, package-scoped formatting, host/projection/PTY tests and
the isolated native smoke tool documented in `crates/app-gpui/README.md`.
Headless tests do not verify native IME, display-server integration or packaging.

The default release entry remains unchanged. Computer control stays denied until
an independent visible indicator and stop path are implemented and verified.
See the package README for remaining product-level parity work.
