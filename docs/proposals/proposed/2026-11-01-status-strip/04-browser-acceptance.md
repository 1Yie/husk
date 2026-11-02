# 04 — Browser acceptance

## Scope

Prove the strip where it actually renders, at the widths and themes the design
rule names.

## Method

The existing Playwright harness (`crates/app-tauri/frontend/scripts/terminal-reveal/`)
with the real `TitleBar`, extended to a `--status-strip` scenario, or the
measurement probe promoted to a script if the scenario proves too heavy.

## Matrix

- Widths: 1280, 960, 880 (the collapse threshold) and 720 (below it).
- Themes: light, dark, one user theme with a custom accent and font.
- States: empty session; a session with todos and two live subagents; a narrow
  bar with every module enabled (the overflow case).

## Assertions

- No chip overlaps the window controls or the right-side toggle; no clipped text.
- Chip heights identical across modules and across themes.
- `Alt+←/→` and drag produce the same stored order.
- Screenshots kept per cell of the matrix; failures named by cell.

## Done condition

Every cell captured, assertions green, and any cell that could not run (a native
WebKit/GTK pass, say) named in the report rather than implied.

## Result

Chromium and WebKit each passed 36 captured cells: widths 1280, 960, 880 and 720;
light, dark and a custom accent/font; empty, todo/subagent and all-module states.
The real title bar had no horizontal overflow, clipped controls, overlapping
controls or uncaught page errors.

Separate browser checks covered the four defaults when pricing is available,
preview editing, persistence round trips, theme switching and mutually exclusive
detail cards. The overlay colour matrix passed 77 scenarios per engine above
4.5:1 after the thinking-tier correction in `9e1625b6`. The combined desktop
frontend run passed type-checking, 1,081 tests and a production build.

These runs use the existing terminal-reveal harness with mocked native IPC.
Chromium's frosted painting was visually checked. WebKit exposes the blur style
and geometry, while native blur painting remains a desktop verification item.
