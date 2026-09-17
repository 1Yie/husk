# Codex Desktop Design Language — Visual Spec

> The kernel↔UI *data* contract lives in `iced-ui-contract.md` (structs,
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
* **Execution stream**: the left main column. User prompts as plain
  right-aligned text (no bubble); agent replies left-aligned; reasoning
  collapses to a dimmed one-line fold; tool calls compress to single-line
  **capsules**. Approvals are **inline on the capsule** — an `allow`/`deny`
  row grows under the awaiting tool, not a side panel.
* **Diff / changes**: NOT a permanent third column. A capsule's
  `awaiting_confirm` state shows the accept/deny inline; a `changed_files`
  entry opens a **slide-over** with a line-level diff viewer and per-hunk
  Accept/Revert. Nothing reserves space until it's needed.

## 2. Theme tokens — `ui/theme.rs`

Industrial cold-grey, near-black. No gradients, no blur, no drop shadows.
1 px hairlines separate zones. Accents are *restrained to the point of
scarcity*: white/silver for focus, green/red for diff only, purple for the
executing state.

In iced the theme is a `pub mod theme` of `iced::Color` consts + `Font`
stacks — same tokens, Rust form (iced colors are `Color::from_rgb8`):

```rust
pub mod theme {
    use iced::{Color, Font};
    use iced::font::Family;

    pub const BG_WORKSPACE: Color      = Color::from_rgb8(0x09,0x09,0x0b);
    pub const BG_PANEL: Color          = Color::from_rgb8(0x12,0x12,0x15);
    pub const BG_CARD: Color           = Color::from_rgb8(0x18,0x18,0x1b);
    pub const BG_HOVER: Color          = Color::from_rgb8(0x27,0x27,0x2a);
    pub const BG_ACTIVE: Color         = Color::from_rgb8(0x3f,0x3f,0x46);

    pub const BORDER_HAIRLINE: Color   = Color::from_rgb8(0x27,0x27,0x2a);
    pub const BORDER_FOCUS: Color      = Color::from_rgb8(0x52,0x52,0x5b);

    pub const TEXT_WHITE: Color        = Color::from_rgb8(0xfa,0xfa,0xfa);
    pub const TEXT_SECONDARY: Color    = Color::from_rgb8(0xa1,0xa1,0xaa);
    pub const TEXT_MUTED: Color        = Color::from_rgb8(0x52,0x52,0x5b);
    pub const TEXT_DIM: Color          = Color::from_rgb8(0x3f,0x3f,0x46);

    pub const ACCENT: Color            = Color::from_rgb8(0xe4,0xe4,0xe7);
    pub const DIFF_ADD: Color          = Color::from_rgb8(0x22,0xc5,0x5e);
    pub const DIFF_ADD_SURFACE: Color  = Color::from_rgb8(0x05,0x2e,0x16);
    pub const DIFF_DEL: Color          = Color::from_rgb8(0xef,0x44,0x44);
    pub const DIFF_DEL_SURFACE: Color  = Color::from_rgb8(0x45,0x0a,0x0a);
    pub const STATUS_RUNNING: Color    = Color::from_rgb8(0xa8,0x55,0xf7);
    pub const WARN: Color              = Color::from_rgb8(0xf5,0x9e,0x0b);
    pub const ERROR: Color             = Color::from_rgb8(0xef,0x44,0x44);

    pub const MONO: Font = Font { family: Family::Name("JetBrains Mono"), ..Font::MONOSPACE };
    pub const SANS: Font = Font { family: Family::Name("Inter"), ..Font::DEFAULT };
}
```

**Supersedes** any Catppuccin set — keep the `theme` module name so views
are explicit; the `App` state and `Message` enum are untouched.

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
  or routes large diffs to a slide-over — never a scrolling list inside a
  `scrollable`/`column!` row (an inner scrollable inside a scrolled column row) (nested-scroll rule from the contract stands).

### Terminal caret — streaming indicator

A solid blinking **block** caret (`▮`, 500 ms toggle) appended to the
in-flight text — not a web-style breathing dot. Signals "native terminal
writing code directly". Lives at the tail of the last `streaming` message.

### Diff review

Approvals are **inline on the tool capsule** (a 56px `allow`/`deny` row),
not a permanent right panel. A `changed_files` entry can open a slide-over
for full diff review: `path` (mono 600) + `+N -M` (green/red) +
`Revert (Esc)` ghost + `Accept All (⌘Y)` white-solid. Per-hunk
Accept/Revert lives in the diff body gutter.

## 4. Window skeleton — `ui/mod.rs` + `App::view`

iced renders Elm-style — `view()` builds a widget tree from `&self`, one
`row!`/`column!`/`container` composition:

```rust
fn view(&self) -> Element<'_, Message> {
    let sidebar = container(sidebar_view(&self.sessions))
        .width(Length::Fixed(220.0)).height(Length::Fill)
        .style(|_| container::Style {
            background: Some(theme::BG_PANEL.into()),
            border: Border { width: 1.0, color: theme::BORDER_HAIRLINE, .. },
            .. });

    let stream = column![
        scrollable(message_list(&self.messages)),   // ExecutionStream
        steps_strip(&self.active_steps),            // capped capsule list
        input_capsule(&self.input),                 // floating prompt box
    ].spacing(0).height(Length::Fill);

    // Approvals are INLINE on the capsule — no permanent right panel
    row![sidebar, stream].into()
}
```

## 5. Mapping onto the data contract

No struct changes — this is pure presentation:

| Contract feed | Codex rendering |
|---|---|
| `App.active_steps` (`StepRow`) | `CodexStepCapsule` rows in the stream strip |
| `MessageRow.text/reasoning` | left text block / dimmed fold line + caret when `streaming` |
| `App.pending` (`ApprovalRow`) | inline `allow`/`deny` row on the matching capsule; risk tier colors it (critical → `error` banner) |
| `App.changed_files` | sidebar section or slide-over file list (grouped by turn) |
| `running_sandboxes` / `sandbox_unsafe` | sidebar badge + step-capsule footer (`UNSANDBOXED` warn chip) |
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
- **Nested-scroll rule still holds** — no scrolling list inside a
  `scrollable`/`column!` row (an inner scrollable inside a scrolled column row); large diffs go to a slide-over, not an
  expanding inline panel.
