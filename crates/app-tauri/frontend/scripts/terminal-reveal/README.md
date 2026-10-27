# Terminal regression checks

Run from the repository root:

```sh
npm --prefix crates/app-tauri/frontend run test:terminal:production -- --repeat=3
npm --prefix crates/app-tauri/frontend run test:terminal:production -- --reduced-motion
```

These commands require Python 3 with PyGObject, GTK 3, WebKit2 4.1 and a graphical
display. Each repetition must pass; the runner stops on the first failure.

The production check bundles the real `ChatPage`, terminal UI and xterm with a
160-step conversation. Only Tauri IPC is simulated: process lifecycle counts are
boundary assertions, not a benchmark of real shell startup or PTY throughput.
The build uses a temporary directory and never replaces the application's
`dist/`; temporary files are removed on success and failure.

Assets use a restricted `husk-terminal-test://localhost` handler, matching the
custom-protocol loading shape of a packaged Tauri application. It only serves
files inside the temporary build root, with normal origin security enabled.
This avoids WebKitGTK's intermittent local-HTTP resource failures being
misclassified as application animation failures. Resource-load errors still
fail the check and include their URI and error code.

Coverage includes repeated open/close, layout and composer alignment, split
geometry and retained renderers, drag-time fit suppression, warm switching,
compact tab menus, rename, and closing each requested process exactly once.
Reduced motion is requested through GTK's local animation setting and must be
observed by the page's actual media query.

`p95TimerMs` spans the full one-second measurement window, including cold
terminal initialization after reveal; it is not an animation-only measurement.
React's `Profiler` callbacks are disabled in production, so `chatCommits: 0`
there must not be interpreted as proof of zero React commits.

For development-mode Chromium checks:

```sh
npm --prefix crates/app-tauri/frontend run test:terminal
npm --prefix crates/app-tauri/frontend run test:terminal -- --tabs
npm --prefix crates/app-tauri/frontend run test:terminal -- --split
npm --prefix crates/app-tauri/frontend run test:terminal -- --resize
npm --prefix crates/app-tauri/frontend run test:terminal -- --menu
npm --prefix crates/app-tauri/frontend run test:terminal -- --settings
```

The settings check renders the real terminal settings pane at 320–1080px widths
and with enlarged root text. It verifies that scheme names, colour swatches and
selection markers stay on one row, cards remain equal-sized, and Solarized can
still be selected.

Install Playwright's Chromium, or select an existing executable with
`CHAT_TERMINAL_EXECUTABLE`. `CHAT_TERMINAL_CPU=4` enables Chromium CPU throttling.
The optional `--native-webkit` development check uses the same GTK assertions
but loads through Vite's HTTP server; use the production checks above for
packaged-resource acceptance.

For the workspace welcome page and responsive tab overflow menu:

```sh
node crates/app-tauri/frontend/scripts/terminal-reveal/workspace-check.mjs
```

This checks light/dark themes at 260–640px widths, keeps the Home control
reachable, verifies overflow selection and keyboard focus restoration, and
saves welcome-page screenshots to `/tmp/workspace-welcome-{light,dark}.png`.
It accepts the same `CHAT_TERMINAL_EXECUTABLE` browser override.

For live terminal-settings performance:

```sh
node crates/app-tauri/frontend/scripts/terminal-reveal/settings-performance-check.mjs
```

This uses eight real xterm renderers and the real settings controls, with mocked
PTY IPC and 4× Chromium CPU throttling. It reports palette application timings
and frame gaps, asserts that palette/cursor changes do not refit geometry, and
checks that only visible terminals apply changes immediately. A hidden tab must
catch up with the latest font when selected without restarting its process.
Timings are diagnostic, not cross-machine thresholds; redundant work is asserted
deterministically. `CHAT_TERMINAL_EXECUTABLE` selects an installed Chromium.
