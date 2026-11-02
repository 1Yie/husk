# Prepare the Tauri consumers for agent activity

## Scope

Deliver the frontend portion of parts 01–03 against the repository's committed
SDK pin. The kernel producers, host cancellation integration, persisted todo
schema and other clients remain in those parts; their delivery is independent
of this consumer preparation.

## Inputs

- `crates/app-tauri/frontend/src/types/index.ts`
- `crates/app-tauri/frontend/src/features/chat/hooks/`
- `crates/app-tauri/frontend/src/features/chat/components/todo-view/`
- `crates/app-tauri/frontend/src/features/chat/components/tool-chips/`
- `crates/app-tauri/frontend/src/features/chat/components/subagents-panel/`

## Outcome

- Render pending, in-progress and completed todo lines, including indented
  children and their progress notes.
- Associate queued, started and finished tool rows by `call_id` when supplied.
- Keep background-child cards live when their parent turn settles, while
  retaining the foreground behavior of events without background metadata.
- Expose these projected values to the status-strip modules.

## Done

- Frontend type-checking and the component/projection tests pass.
- The desktop build succeeds against the committed SDK dependency.
- Deferred producer and other-client changes are absent from the delivery.

## Result

The Tauri consumers shipped in `8ebdb524` and `f46bad9f`. The frontend suite
passed, and the build used the committed dependency rather than the local korei
patch. The later combined run passed 1,081 tests.

This validates the consumers with synthetic events and persisted-view fixtures.
End-to-end emission and cancellation remain acceptance work for parts 01–03.
Current module ownership and behavior are documented in
`.blueprint/tauri-frontend.md`.
