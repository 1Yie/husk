# 03 — Customise: toggle, reorder, persist

## Scope

Let the reader decide which modules show and in what order, without leaving the
title bar, plus a 「状态栏」 section inside 设置 → 外观 that edits the same stored layout.

## Interaction

- **Entry**: right-click anywhere on the strip opens the customiser; a small
  「自定义」 button fades in at the strip's right edge on hover opens the same
  panel. The settings side lives inside 设置 → 外观 as a 「状态栏」 section — the
  owner's decision, because it styles the chrome the rest of that page already
  configures, and a separate top-level page would split one concern in two.
- **Drag**: pointer drag along the row with an insert line; dropping writes the
  order, Esc cancels without writing. A press cancels the chip's hover timer and
  the popup never opens mid-drag.
- **Keyboard**: a focused chip moves with `Alt+←/→`; the customiser rows move with
  the same keys; every control is reachable by Tab.
- **Toggle**: a hidden module keeps its slot in `items`, so turning it back on
  restores its position.

## Data

`status_strip.items` is written in display order; `status_strip.hidden` holds the
ids switched off. A write is one `setAppearance` call at drop/toggle, never per
pointer move.

## Must not touch

- Module contents (02) and the strip structure (01).
- Other appearance fields.

## Done condition

- Reorder survives a reload and a workspace switch; toggling every module off
  still leaves the customiser reachable and one click restores the defaults.
- Esc during a drag leaves the stored layout untouched.
- Tests: order write/restore, hidden round trip, keyboard move, Esc cancel.

## Result

The owner selected a real window simulation inside appearance settings as the
only configuration surface. The workspace strip displays status; its context
menu and edit button are absent. The preview embeds the real `TitleBar`, with
nonzero sample values in both the window and hidden-item tray.

Pointer dragging uses capture and a ghost, accepts the complete title-bar and
preview-content drop zones, and writes once when released. Escape cancels
without writing. Keyboard ordering, removal, restoration and resetting remain
available in the preview.

This shipped in `f46bad9f`. Component tests and Chromium/WebKit interactions
covered a single drag write, Escape, keyboard movement, hidden-tray restoration
and synchronisation with the mounted workspace strip.
