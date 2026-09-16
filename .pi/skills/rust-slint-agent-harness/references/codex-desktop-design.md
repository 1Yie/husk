# Codex Desktop Design Language — Visual Spec

> The kernel↔UI *data* contract lives in `slint-ui-contract.md` (structs,
> callbacks, binding rules — unchanged). This file is the **visual language**:
> layout topology, theme tokens, and the micro-components that make the app
> read as a developer cockpit instead of a chat wrapper.

Design soul: **a compact, immersive task-execution workbench** — not a chatbot.
Feels like Neovim/terminal split given a GUI: left column thinks and dispatches
intent, right column holds truth (code + diffs). Matches the kernel's "single
static binary, millisecond cold start, 15 MB idle" industrial ethos.

---

## 1. Layout topology — three zones, not a chat stream

```
┌────────┬──────────────────┬─────────────────────────────┐
│ Slim   │  Execution       │  Diff / Changes Stage        │
│ sidebar│  stream (prompts │  (review canvas — rises      │
│ ~64px  │  + tool capsules │   whenever files change)     │
│        │  ~420px          │   flex)                      │
└────────┴──────────────────┴─────────────────────────────┘
```

* **Slim sidebar**: workspace switcher, session history, env status
  (sandbox backend badge, git branch). Icon-width, not a nav drawer.
* **Execution stream**: the left main column. User prompts as plain text
  blocks; reasoning collapses to a dimmed one-line fold; tool calls compress
  to single-line **capsules**. No bubbles.
* **Diff / changes stage**: the right canvas. `Bridge.pending` /
  `changed_files` non-empty → it rises with a line-level diff viewer and
  per-hunk Accept/Revert. When nothing is pending it shows a quiet empty
  state ("no pending changes") — it never collapses away (layout stability).

## 2. Theme tokens — `ui/codex_theme.slint`

Industrial cold-grey, near-black. No gradients, no blur, no drop shadows.
1 px hairlines separate zones. Accents are *restrained to the point of
scarcity*: white/silver for focus, green/red for diff only, purple for the
executing state.

```slint
export global CodexTheme {
    // Backgrounds — deep, cold, layered
    out property <color> bg_workspace:      #09090b; // canvas / lowest layer
    out property <color> bg_panel:          #121215; // panels + sidebar
    out property <color> bg_card:           #18181b; // cards / input capsule
    out property <color> bg_hover:          #27272a; // hover
    out property <color> bg_active:         #3f3f46; // selected row

    // Hairline borders — 1px, low contrast
    out property <color> border_hairline:   #27272a;
    out property <color> border_focus:      #52525b;

    // Type — high clarity, zero noise
    out property <color> text_white:        #fafafa; // primary / active titles
    out property <color> text_secondary:    #a1a1aa; // body / descriptions
    out property <color> text_muted:        #52525b; // line numbers, keybinds
    out property <color> text_dim:          #3f3f46; // placeholders

    // Semantics — restrained accents
    out property <color> accent:            #e4e4e7; // cold silver-white = focus
    out property <color> diff_add:          #22c55e;
    out property <color> diff_add_surface:  #052e16;
    out property <color> diff_del:          #ef4444;
    out property <color> diff_del_surface:  #450a0a;
    out property <color> status_running:    #a855f7; // purple = executing
    out property <color> warn:              #f59e0b; // sandbox-unsafe, elevated risk
    out property <color> error:             #ef4444;

    // Fonts
    out property <string> font_mono:        "JetBrains Mono, SF Mono, Menlo, monospace";
    out property <string> font_sans:        "Inter, -apple-system, 'Segoe UI', sans-serif";
}
```

**Supersedes** the Catppuccin `Theme` global in `slint-ui-contract.md` —
keep the name `CodexTheme` so components are explicit; the structs and
`Bridge` callbacks are untouched.

## 3. Signature micro-components

### `CodexStepCapsule` — tool call as a single line

Codex never lets `cargo test` or `smart_read` occupy half a screen. Each
`ActionStepData` renders as a 28 px capsule:

```
[ ●|✓|✗ ] fuzzy_patch   src/engine.rs            ← 28px, hairline border
   running│ok│err
```

* status glyph: `●` running (`status_running` purple), `✓` success
  (`diff_add`), `✗` error (`diff_del`), `⊘` denied (strikethrough),
  `◌` awaiting_confirm (accent border pulses).
* `tool_name` in mono 600, `detail` in muted mono, `overflow: elide`.
* `expandable` → tap opens the detail inline (≤200 lines, max-height 400px)
  or routes large diffs to the right stage — never an inner `ListView`
  inside a `ListView` row (nested-scroll rule from the contract stands).

### Terminal caret — streaming indicator

A solid blinking **block** caret (`▮`, 500 ms toggle) appended to the
in-flight text — not a web-style breathing dot. Signals "native terminal
writing code directly". Lives at the tail of the last `streaming` message.

### Diff stage review bar

Top of the right canvas: `path` (mono 600) + `+N -M` (green/red) +
`Revert (Esc)` ghost button + `Accept All (⌘Y)` white-solid button.
Per-hunk Accept/Revert lives in the diff body gutter.

## 4. Window skeleton — `ui/codex_window.slint`

```slint
import { CodexTheme } from "codex_theme.slint";

export component CodexDesktop inherits Window {
    title: "agent-rs";
    min-width: 1080px; min-height: 720px;
    background: CodexTheme.bg_workspace;

    HorizontalLayout { spacing: 0px;
        // (a) slim sidebar — icon width
        Rectangle { width: 64px; background: CodexTheme.bg_panel;
                    // workspace icons, session dots, sandbox badge, git branch
                    border-right-width: 1px; border-color: CodexTheme.border_hairline; }

        // (b) execution stream — fixed ~420px
        Rectangle { width: 420px; background: CodexTheme.bg_panel;
                    border-right-width: 1px; border-color: CodexTheme.border_hairline;
            VerticalLayout { padding: 16px; spacing: 12px;
                // header: "CURRENT SESSION" + "L2 SANDBOX: ON" chip
                // ListView: prompts as text blocks, reasoning folds,
                //           CodexStepCapsule rows
                // bottom: floating input capsule (bg_card, ⌘⏎ hint, → run)
            } }

        // (c) diff/changes stage — flex fill
        Rectangle { background: CodexTheme.bg_workspace;
            // review bar (path, +N-M, Revert/Accept) when pending != none
            // ListView<DiffLineData>: 20px rows, mono, gutter +/-/space,
            //   add→diff_add_surface, del→diff_del_surface
        } }
}
```

## 5. Mapping onto the data contract

No struct changes — this is pure presentation:

| Contract feed | Codex rendering |
|---|---|
| `Bridge.active_steps` (`ActionStepData`) | `CodexStepCapsule` rows in the stream |
| `SessionMessageData.text/reasoning` | text block / dimmed fold line + caret when `streaming` |
| `Bridge.pending` (`PendingApprovalData`) | right stage rises; risk tier colors the review bar (critical → `error` banner) |
| `Bridge.changed_files` | right stage file list (grouped by turn) |
| `Bridge.running_sandboxes` / `sandbox_unsafe` | sidebar badge + step-capsule footer (`UNSANDBOXED` warn chip) |
| `stats.agent_state` | sidebar status line + input placeholder swap (`Steer the agent…` while active) |
| `suggestions` | quiet status-bar chip in the stream column — never a modal |

## 6. Non-negotiables (visual)

- **No chat bubbles.** Messages are flush-left timeline rows; only prompts
  get a subtle `bg_card` block. Agent output is full-width text.
- **1 px hairlines only.** No shadows, gradients, blur, or rounded corners
  beyond 8 px on the input capsule (4 px elsewhere).
- **Accent scarcity.** Purple = executing only. White = the single primary
  action on screen (Accept All). Green/red are reserved for diff semantics.
- **Density over whitespace.** Stream rows ~28 px, diff rows ~20 px, mono
  type for everything code-adjacent, 10–13 px UI text.
- **Nested-scroll rule still holds** — no `ListView` inside a `ListView`
  row; large diffs go to the right stage, not an expanding inline panel.
