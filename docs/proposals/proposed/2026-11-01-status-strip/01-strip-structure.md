# 01 — Registry, strip and stored layout

## Scope

Move the four existing chips behind a registry and an ordered `StatusStrip`, and
persist the order. No new module, no visual change.

## Inputs

- `crates/app-tauri/frontend/src/components/title-bar/index.tsx` — `statsChips`,
  the chip wrapper (`flex items-center gap-0.5 … translate-y-[1px]`),
  `CHIP_ICON`, `ContextRing`, `StatPopover`.
- `crates/husk-host/src/prefs.rs` — `AppearanceSettings` (global row `appearance`).
- `crates/app-tauri/frontend/src/lib/agent-ipc/sessions.ts` — `AppearanceConfig`,
  `getAppearance` / `setAppearance` (partial merge).

## Storage

```rust
#[serde(default)]
pub status_strip: Option<StatusStripSettings>,

pub struct StatusStripSettings {
    /// Module ids in display order; unknown ids are skipped on render.
    pub items: Vec<String>,
    /// Ids the reader switched off — kept so a reorder survives a toggle.
    pub hidden: Vec<String>,
}
```

Absent, or an `items` list that resolves to nothing, means the built-in order:
a strip with no modules has no way back to the customiser.

## Frontend

- `components/title-bar/strip/registry.tsx`: one entry per module
  (`id`, `label`, `hideWhenEmpty`, `render(props)`), default order
  `stream-health, git, activity, tokens, context`.
- `components/title-bar/strip/index.tsx`: `StatusStrip` resolves the stored order
  against the registry, renders the chips inside the existing wrapper, and keeps
  the `compact` (icon-only) rule.
- `components/title-bar/strip/use-status-strip.ts`: read the stored layout once
  at mount (`getAppearance`), expose the resolved list, and write changes with
  `setAppearance({ status_strip })`.

## Must not touch

- Chip styling, the hover-card interaction, the `<880px` collapse, the window
  controls.
- Any other appearance field.

## Done condition

- With no stored layout the rendered DOM is the same four chips in the same order
  with the same classes; the measurement probe's geometry and a screenshot before
  and after are identical.
- A stored `items` order is honoured; an unknown id is skipped; an empty or
  all-unknown list falls back to the built-in order.
- Kernel/host tests for the new `AppearanceSettings` field (absent, present,
  unknown ids survive a round trip) and the frontend tests for order resolution.

## Result

The declarative registry and stored layout shipped in `f46bad9f`. Unknown ids
are skipped, persisted custom layouts retain their order, and the built-in
selection follows the owner's four defaults: git, tokens, context and turn cost.
The registry is the source of those defaults.

Appearance persistence is carried by the host DTO and the Tauri merge path.
Its focused Rust tests passed, together with frontend layout and rendering tests.
Current behavior is described in `.blueprint/tauri-frontend.md`,
`.blueprint/husk-host.md` and `.blueprint/tauri-backend.md`.
