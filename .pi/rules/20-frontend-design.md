---
trigger: glob
globs: "crates/app-tauri/frontend/src/**, crates/app-tauri/frontend/scripts/**, crates/app-tauri/frontend/tailwind.config.js"
description: Preserve Husk's existing design system, shared component styling, interaction behavior, and measured performance.
---

# Husk frontend: design and implementation rules

## Authority and design intent

- Extend the existing desktop workspace; do not redesign it while fixing a bug.
  Keep the interface compact, content-first and predominantly neutral. Use accent
  and status colours to communicate state, not to decorate unrelated controls.
- The user's latest design decisions and the current shared components are the
  authority. A different screen's local override is not permission to reproduce it.
  Existing inconsistencies are migration debt, not a new design standard.
- Preserve user-configured themes and fonts. A fix must work beyond the developer's
  default light theme and default panel width.
- This rule governs future edits; it does not authorize an unrelated global restyle.

## Read the actual sources before changing styles

Paths below are relative to `crates/app-tauri/frontend/`:

| Concern | Source of truth |
| --- | --- |
| Light/dark tokens and global typography | `src/index.css` |
| Semantic colours, radius scale and shadows | `tailwind.config.js` |
| Runtime theme/font overrides | `src/lib/appearance.ts` |
| Overlay, bubble, input and focus surfaces | `src/lib/theme.ts` |
| Primitive styling and interaction behavior | `src/components/ui/` |
| Class conflict resolution (`clsx` + `twMerge`) | `src/lib/utils.ts` |
| Settings layout and selection controls | `src/features/settings/components/index.tsx` |
| Shared terminal toggle/shortcut presentation | `src/components/layout-controls.tsx` |
| Browser acceptance checks | `scripts/terminal-reveal/README.md` |

Read the relevant primitive, its theme constants and at least one comparable
consumer. Do not infer a primitive's defaults from shadcn, Radix, or Tailwind docs.

## Shared styling ownership — do not override it accidentally

- Shared primitives own surface colour, border, corner radius, shadow, hover/focus
  fill, open/disabled states, and baseline padding. Business components supply
  content, behavior and necessary layout constraints.
- **Do not add local `rounded-*`, `shadow-*`, border, background, hover or focus
  overrides to "make it look right".** This includes menu items' hover shapes,
  not just the outer popup. Preserve the user's shared-component edits.
- Fix only the reported dimension. A font-size bug does not authorize changing
  popup width, item radius, padding or elevation.
- A contextual density adjustment can set font size, line height, icon size and
  icon/text gap after comparison with the surrounding UI. Do not bundle unrelated
  surface overrides into that change.
- When a genuinely shared variant is needed, propose its scope, add it centrally
  and check affected consumers. Do not silently change a generic component to
  repair one screen, or silently invent a new per-screen visual variant.
- Inspect the resolved classes: `cn()` uses `twMerge`, so a consumer's conflicting
  utility can replace the primitive's style rather than merely append to it.

Example for the compact workspace overflow menu:

```tsx
// Adjust the content's density; inherit the primitive's surface and hover shape.
<DropdownMenuItem className="gap-2 text-xs">...</DropdownMenuItem>

// Wrong for a font-only fix: overrides the shared item's radius and padding.
<DropdownMenuItem className="rounded-md px-2 py-1.5 text-xs">...</DropdownMenuItem>
```

## Theme, typography and icons

- Prefer semantic utilities such as `bg-workspace`, `bg-panel`, `border-hairline`,
  `text-text-primary`, `text-text-secondary` and `text-accent`, or the existing
  `--husk-*` tokens. Centralize any necessary new semantic colour.
- `neutral-*`, `white` and `black` are theme-mapped, not literal Tailwind colours.
  Do not double-invert text with an extra dark variant or assume `text-white`
  always paints white. Overlay and always-dark bubble exceptions belong in
  `src/lib/theme.ts`, not in each consumer.
- Use the configured radius and shadow scale. Names like `rounded-md` do **not**
  have stock Tailwind pixel values in this project. Do not copy numerical defaults
  from another project.
- Inherit the configured UI font for UI text and shortcut hints; preserve the
  configured code font for code/terminal content. Do not force a new font family,
  tracking or weight to compensate for poor contrast.
- Match the current local density. The workspace's compact menu uses `text-xs`
  (12px/16px at the default root size), 14px icons and an 8px icon/text gap.
  Generic menu defaults are not automatically suitable for compact controls.
  These values do not mandate shrinking every menu or every settings page.
- Keep decorative icons hidden from accessibility APIs; label icon-only buttons.
  Use the existing icon libraries and the same icon/spacing treatment as adjacent
  actions. Counts and shortcuts remain subordinate without becoming unreadable.
- Check small active text against its actual composited background; target 4.5:1.
  Consider ancestor opacity and hover/focus states, not just nominal token values.

## Layout, overlays and interaction

- Reuse existing Radix wrappers instead of custom popup positioning or native
  replacements. Preserve their placement gate, scroll handling, focus management
  and portal conventions. Keep existing menus non-modal; do not generalize that
  requirement to modal dialogs.
- Settings use `SettingSelect`, `SettingRow`, `SettingsSection`, `SettingsCards`
  and `KvList` where applicable. Do not replace `SettingSelect` with a scroll-locking
  select just to reproduce its appearance.
- Menus must be checked **open and hovered/focused**. Verify icon spacing, counts,
  Escape dismissal, keyboard selection, focus restoration and outside clicks.
  Do not steal focus back after the user intentionally focuses another control.
- Protect fixed actions before constraining content. Use `min-w-0` and deliberate
  overflow handling; do not hide an unreachable action with `overflow-hidden`.
  Workspace tabs put excess items in the existing More menu. Terminal tabs scroll,
  retain fixed controls and reveal the active tab. Do not substitute text clipping
  or a different overflow model without a requirement.
- Reuse the same component for the same action's tooltip and shortcut presentation.
  Obtain platform shortcuts from `src/lib/platform.ts`; do not hardcode a second
  spelling or colour treatment at another entry point.
- Loading states use existing skeleton primitives with stable geometry and an
  accessible status. Do not replace errors, empty states or meaningful progress
  with an indefinite skeleton.

## Animation and performance

- Reuse the existing layout/reveal mechanisms; preserve synchronized panel and
  neighboring content geometry. A transform-only overlay is not a fix if it leaves
  hit targets, menus or surrounding layout at the wrong size.
- Respect reduced motion. Clean up animation frames, observers and pointer state
  on release, cancellation, lost capture, hide and unmount.
- Keep subscriptions near their consumers. Do not rerender the application shell
  for terminal visibility, or commit expensive application state on every pointer
  event. Batch drag previews and commit final state through the existing mechanism.
- Apply only changed renderer options. Colour/cursor changes must not invalidate
  font geometry. Hidden terminal renderers catch up when shown; keep PTYs, output,
  stable keys and visible split panes intact.
- For a performance complaint, measure the actual interaction before editing:
  frame gaps, work count, fit/reflow count and lifecycle events as applicable.
  Compare before/after under the same conditions. Report simulated CPU throttling
  and mocked IPC; do not present those timings as native PTY performance.

## Required development and acceptance workflow

1. Identify the exact operation and affected states; clarify an ambiguous symptom.
   Inspect the user's screenshot and current diff before editing.
2. Read the owning primitive/tokens and a comparable consumer. State which layer
   should change. Preserve unrelated user edits.
3. Create a failing regression at the relevant boundary before fixing a bug.
   For a visual issue, use browser-computed styles/geometry and screenshots;
   a jsdom class assertion alone does not establish the visual result.
4. Make the smallest correction. Do not mix a density fix with radius, colour,
   spacing-scale or architecture changes that were not requested.
5. Check light/dark themes, narrow supported widths, long labels/counts, user
   fonts, and relevant hover/focus/disabled/loading states. Inspect the actual
   popup screenshot, not only the closed trigger. Exercise keyboard behavior.
6. Run targeted tests and `npm --prefix crates/app-tauri/frontend run typecheck`.
   Use the documented browser checks when applicable; check LSP before a build.
   If a required browser/native check cannot run, name the gap rather than claiming
   the interface was verified.
7. Review the final diff for local overrides and unrelated changes. Report the
   fix and the checks actually performed. Do not bump a version or commit unless
   the user requested it.
