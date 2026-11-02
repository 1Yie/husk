# Make the title-bar status strip configurable

Accepted: 2026-11-01

## Problem

The title bar's stat cluster is four chips written inline in
`crates/app-tauri/frontend/src/components/title-bar/index.tsx`
(`statsChips`): git branch, generation activity, token usage, context window. The
order is source order, the set is closed, and nothing else the session already
knows — permission mode, the todo list's progress, live subagents, the turn's
cost — can be shown there without editing that file. A reader who wants the
context meter first has no way to say so.

## Proposal

Turn the cluster into an ordered **status strip** driven by configuration:

- A registry of modules, each with an id, a label, a render for the chip and an
  optional hover card (the existing `StatPopover` + `CardRow`).
- `AppearanceSettings.status_strip: { items: [id…], hidden: [id…] }`, global like
  the rest of `appearance` (one layout across workspaces). Absent or empty →
  the built-in order, so no existing install changes.
- A `StatusStrip` that renders the configured order and drops ids the build does
  not know, so removing a module never breaks a stored layout.
- Modules beyond today's four, off by default except the two the owner asked
  for: todo progress (`n/m`, plus the running item while one is in progress) and
  the active-subagent count.

Customisation (P3) is a right-click on the strip plus a 设置 → 状态栏 page: toggle,
drag to reorder, `Alt+←/→` on a focused chip, Esc cancels. The strip keeps its
`<880px` collapse to icon-only.

## Alternatives

- **A settings-only list, no strip interaction.** Rejected: reordering four to
  twelve short items is a drag, not a form; a settings-only path makes the
  reader leave the surface they are composing in.
- **A drag-and-drop library.** Rejected: the repo carries none, and a chip row
  needs insert-position feedback, not a general collision engine. A pointer drag
  over a flex row is ~80 lines and keyboard support has to be written either way.
- **Per-workspace layouts.** Rejected: the strip describes the bar, not the
  project; two workspaces showing different chips reads as a bug.
- **Keep the inline chips and add the new ones.** Rejected: the set would still
  be closed, only larger.

## Acceptance criteria

- P1: with no stored layout, the four existing chips render in the same order,
  size and colour as before (pixel-diffed against the current build).
- P2: each new module shows its real value, hides itself when it has none, and
  opens a hover card with the same interaction as the existing chips.
- P3: a reorder survives a reload and a workspace switch; toggling off every
  module leaves the customise entry reachable; `Alt+←/→` and drag produce the
  same stored order; Esc cancels a drag without writing.
- P4: browser run over 1280/960/880 widths in light, dark and a custom theme:
  no chip clips the window controls, no overlap, screenshots kept.

## Risks

- **The strip sits on the event path.** Every delta re-renders the bar; each
  module must subscribe to its own slice (as `StreamHealth` does) or the shell
  pays for every streamed token.
- **Hover cards and drag compete for the same pointer.** A press on a chip must
  cancel the hover timer, and the popup must not open under the drag.
- **A stored layout can outlive a module.** Unknown ids are skipped rather than
  rendered as gaps; the id list is stable and documented in the registry.
- **Width**: twelve chips overflow a narrow bar. The count is capped and the
  strip collapses to icons before it can push the window controls.

## Checklist

- [x] `01-strip-structure.md` — registry + `StatusStrip` + stored layout.
- [ ] `02-new-modules.md` — permission mode, todo progress, subagents, cost
      (after 01). Todo progress, subagents and cost shipped; the standalone
      permission module remains deferred.
- [x] `03-customise.md` — toggle, drag and keyboard editing in the appearance
      preview. The owner's settings-only entry supersedes the right-click path.
- [x] `04-browser-acceptance.md` — the width × theme run.
