# Wait for a delegated child without repeating the parent's work

Accepted: 2026-11-03
Implemented: 2026-11-04

## Problem

The reported parent delegates a parallel review, needs its result, and keeps
reading files while saying that it is waiting.

The current kernel supports background delegation and sends the child's next
question or final report through its parent notifier. Husk installs that
notifier in `crates/husk-host/src/session.rs`; the engine drains host notices
between model rounds and can wake an idle parent. The existing
`a_background_child_returns_at_once_and_reports_as_a_notice` regression passes.

The model-facing parent tools provide `delegate` and `answer_child`, with no
explicit child-wait action. A background `delegate` result says “Keep working”.
In goal mode, `CodingPolicy::on_quiet_round`
(`crates/agent-coding/src/policy.rs`) nudges an undeclared running goal to
continue. These contracts offer no explicit way to suspend a parent tool round
until the child has an update. The exact reported conversation has not been
captured; the notification baseline proves delivery, not the parent's choice
of tools.

## Proposed contract

Add one parent-only tool:

```text
wait_child { child_id: u64 }
```

- Wait asynchronously for that child's next question or final report.
- Keep the same parent turn open, including its goal contract, while waiting.
  The engine makes no further parent model request during the wait.
- Preserve the existing host-notice delivery. The update is available to the
  parent before the waiter resumes, with one report in model context.
- A question wakes the parent so it can use `answer_child`.
- A completed-before-wait race returns promptly and preserves the delivered
  update. An unknown or foreign child never exposes another session's data.
- Parent/session cancellation releases the wait through the existing
  cancellation contract. Waiting does not spawn, retry or redo the child.
- Only child-lifetime coordination state is added; no persisted result cache,
  timer-driven model polling or new dependency is introduced.

## Model-facing guidance

Replace the background result's unconditional instruction to keep working with
this guidance:

> Continue independent work only. When this child's next question or report is
> needed and no independent work remains, call `wait_child` with its `child_id`.
> Do not repeat repository reads or repeat the delegated review to fill the
> wait. Answer a child's question with `answer_child`.

The wait tool's description states that it blocks the current tool round until
the next child handoff or cancellation. The child registry excludes it through
the same parent-only boundary as `delegate` and `answer_child`.

## Implementation boundaries

Korei's `agent-kernel` owns child handoffs, wait coordination and the tool
contract. Husk exposes the tool in its parent registries, retains host-notice
privilege fencing, and gives the existing tool card a readable wait label.
Ordinary, plan and goal parents use the same wait mechanism.

This change preserves `v0.4.1`. SDK publication, updating Husk's pinned SDK and
any subsequent application release are separate delivery actions.

## Acceptance

- A gated background review plus an explicit parent wait produces no additional
  parent model calls or parent file reads until the gate is released.
- Both a final report and an `ask_parent` question release the wait. The latter
  continues through `answer_child`.
- Completion-before-wait and completion-during-wait each preserve the update
  once in model context.
- Cancellation releases a pending wait and keeps session stop effective.
- A foreign/unknown child id is handled without waiting on or disclosing
  another session's child.
- The wait tool is absent from child registries and available in supported
  parent modes.
- A goal-mode parent remains in the same turn during the wait and can complete
  its goal after consuming the child's report.
- Existing background notification, parallel delegation, approval and
  cancellation regressions remain green.

## Result

Korei commit `24d1ba428cc7ee4e3b88aa76f0ffae07035d8b72` implements
`wait_child` and is published on its `main`. The SDK uses child-lifetime
handoff signals, the existing parent notifier, and the dispatcher's
cancellation guard. Its package README owns the host-wiring contract.

Husk pins all five SDK crates to that Git commit. The parent registry exposes
the wait while child registries exclude it. The system prompt directs the
parent to wait after independent work ends, and the tool card displays
“等待子代理” with its target id.

Verification:

- The SDK workspace passed 222 tests; its remote CI passed.
- Husk's related Rust packages passed 503 tests with two existing ignored
  tests and the existing platform-bound `sandbox_disabled_routes_to_none`
  exclusion. The production `custom-protocol` check passed.
- The five host waiting regressions covered build, plan and goal turns,
  cross-turn child lifetime, and question-answer-report handoffs. Gated waits
  produced no additional parent model calls or file-reading tools.
- Frontend directory formatting, type checks, 1085 tests and the production
  build passed.
- Chromium rendered the real `ChatStream` in light/dark modes at widths 1280
  and 720, including a long id, a configured UI font, keyboard expansion,
  running state and completed report details.

Provider traffic in the backend regressions was scripted. The browser probe
used the project's tooltip provider and mocked native IPC.
