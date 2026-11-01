# 01 — Framed shell parity on Windows

## Scope

Render the same inset rounded shell on Windows that macOS already draws: one
outer frame around separate rounded work surfaces, with the inner surfaces
staying opaque. The layout gates that read `isMac` become a platform capability
so the shell decision is made in one place. No native material, no geometry
change, no Linux rendering.

## Inputs

- The proposal, `windows-native-backdrop`, and its acceptance criteria 1 and 6.
- `crates/app-tauri/frontend/src/app/layout/main-layout/index.tsx:140-170`
  (frame, sidebar width and resize grip), and `sidebar-glass.ts:58` for the
  hook gate that 02 changes.
- The components that gate layout on `isMac`:
  `components/title-bar/index.tsx:414-424`,
  `features/chat/pages/chat/index.tsx:405-415`,
  `features/chat/components/session-sidebar/index.tsx:455-475,780-790`,
  `features/chat/components/dock/index.tsx:205-250`,
  `features/terminal/terminal-panel.tsx:15-25,200-212`,
  `features/settings/pages/index.tsx:185-290`.
- `crates/app-tauri/frontend/src/index.css:150-166` — the `data-sidebar-glass`
  chain and the 68% coat. Not edited here.
- `crates/app-tauri/frontend/src/lib/platform.ts` — where a `framedShell`
  capability belongs, next to `isMac`.
- Rule `20-frontend-design`: keep the design system, shared component styling,
  interaction behaviour and measured performance.

## Must not touch

- `crates/app-tauri/backend/` — no Rust in this part.
- `tauri.conf.json`, `tauri.linux.conf.json`, `tauri.macos*.conf.json`.
- `window-controls` traffic-light placement, shortcut labels and the
  `terminal-session` keybindings: per-platform behaviour that is not layout.
- The macOS rendered result: the class lists macOS produces stay as they are.

## Done condition

- On a Windows user agent the shell renders the padded `main-workspace-frame`
  with the same inset, radius and gap as macOS, and the sidebar, title bar,
  chat card, Dock, terminal panel and settings page take the framed variants.
- macOS renders unchanged: the existing suites
  (`main-layout/sidebar-glass.test.tsx`, `components/__tests__/layout-controls.test.tsx`
  and the rest of the frontend suite) pass, and a test pins the gate so a
  Windows capability cannot leak into the macOS class list.
- A local render check at 1280×840 shows the frame, the gaps and opaque inner
  surfaces on the Windows path.
