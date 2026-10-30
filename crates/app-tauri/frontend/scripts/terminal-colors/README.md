# Shell color negotiation acceptance

```sh
TERMINAL_BROWSER=/usr/bin/google-chrome \
  node crates/app-tauri/frontend/scripts/terminal-colors/check.mjs
TERMINAL_BROWSER=/usr/bin/google-chrome \
  node crates/app-tauri/frontend/scripts/terminal-colors/check.mjs --light
```

Runs the actual xterm parser and DOM renderer against **simulated PTY IPC**.
The fixture sets an OSC palette color, requests foreground/background/cursor
colors before `terminal_open` returns, and emits a themed prompt only after
the background-color reply arrives.

Checks:

- Startup and hidden-terminal OSC 4/10/11/12 replies reach the PTY, including
  both BEL and ST query terminators.
- Disabling human input does not break color negotiation.
- Shell-defined palette entries, ANSI 256 colors, and RGB foreground/background
  values survive parsing and rendering exactly.
- OSC default foreground/background changes paint the actual xterm scrolling
  surface; checks do not mistake its legacy viewport placeholder for that layer.
- Hiding/showing retains the prompt and does not restart the PTY.
- Color queries do not refit or resize terminal geometry.
- Changing fonts/cursor style does not reset shell-provided palette/default colors.

Screenshots are saved to `/tmp/husk-terminal-colors/`. This is not a native
PTY performance measurement or a test of the user's installed shell profiles.
The script never edits shell startup files or reads native-terminal themes.
