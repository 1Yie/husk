# Terminal regression checks

## Production CSP regression

```sh
npm --prefix crates/app-tauri/frontend run test:terminal:production -- --csp
npm --prefix crates/app-tauri/frontend run test:terminal:production -- --csp --webkit
```

These checks use the production fixture bundle and the application's CSP
configuration. They verify configured terminal fonts, ANSI colours, preserved
columns and glyph-cell layout in light/dark themes. A nonce-blocked control
confirms that the browser enforces the policy. Script nonce/hash hardening
remains enabled.

PTY IPC is simulated. Native installation-package acceptance is a separate
check. Set `CHAT_TERMINAL_EXECUTABLE` to use an installed Chromium executable.

## Animated panel clipping

```sh
NODE_OPTIONS=--no-experimental-webstorage CHAT_TERMINAL_EXECUTABLE='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' PANEL_CLIP_OUTPUT=/absolute/new-directory npm --prefix crates/app-tauri/frontend run test:terminal -- --panel-clip
```

Use a fresh absolute output directory for each run. The real ChatPage and
SessionDock fixture runs light/dark at 1280px and 960px with simulated Mac inset
and non-Mac flat layouts. It pauses the existing width/transform and height CSS
transitions at four intermediate times in each direction, saving actual,
matching-radius reference and alternate-clip PNGs plus `report.json`.
The corner comparison counts pixels whose maximum RGB channel difference
exceeds 2, allowing only minor rasterization rounding.
Corner paint must match the visible card's radius, with transparent clip roots,
unchanged inner width/gutter, zero closed extent and the existing main-column
boundary. The square/rounded control must distinguish painted Dock corners in
both directions, including closing frames with disabled pointer events.
`--panel-clip-baseline` selects the smaller reproduction matrix.

Run `--panels` and `--panels --non-mac` separately for pointer/keyboard resize,
controls, composer and Dock More portal interaction, focus and renderer/process
retention. Set `CHAT_TERMINAL_OUTPUT` to a separate output directory for each
run; it also redirects the default reveal scenario's screenshot. The default
scenario retains the existing reveal/geometry/fit/mock-PTY assertions.
All scenarios use fixture IPC, an ephemeral Vite port and owned browsers.

The panel runs exercise the production `SessionSidebar` header at default,
minimum and wide widths, including collapse/reopen and light/dark/custom themes.
They assert zero icon/name nodes for simulated Mac and explicitly present,
visible branding for non-Mac, plus the native slot, mode hit targets, tooltips
and pointer/keyboard mode callbacks. Screenshots and `panels.json` capture these
checks alongside the existing panel assertions. These are Chrome platform
simulations with mock IPC; native macOS/WebKit and real Windows/Linux need
separate verification.

## Chat workspace file links

```sh
CHAT_TERMINAL_EXECUTABLE='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm --prefix crates/app-tauri/frontend run test:terminal -- --workspace-links
```

This scenario uses the real ChatPage, Markdown renderer, SessionDock and file
preview with fixture-only workspace IPC. It checks click/Enter, repeated and
changed positions, source-mode Markdown, delayed reads, session/root switches,
missing/oversized/truncated files and blocked unsafe links. Programming and
working modes run in light/dark at a narrow supported width. Each run writes
new screenshots and a JSON result under a unique `workspace-file-links`
scratch directory; failures retain their own screenshot and result. The runner
uses its own ephemeral port and closes its browser and server on completion.

## HTTPS Markdown images

```sh
NODE_OPTIONS=--no-experimental-webstorage CHAT_TERMINAL_EXECUTABLE='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm --prefix crates/app-tauri/frontend run test:terminal -- --remote-badges
```

The real file-tree preview intercepts the five exact README Shields URLs and one
explicit failure URL. SVG responses are constructed fixtures with inert script
and event-handler content, rather than live Shields content. Every other
external request is blocked. Assertions cover decoded images, exact query/alt/title,
no-referrer, safe dimensions, a proportional 112px centered logo with narrow-container
scaling, inline rows,
light/dark and programming/working small/expanded previews. Each state receives
16 parent updates over at least 3.2 seconds while DOM identity, local-image IPC,
network requests and following-text position are sampled. File/root switches
and remote load errors are exercised.

Evidence is written to fresh browser subdirectories under
`.husk/scratch/workspace-markdown-remote-images/`; `MARKDOWN_REMOTE_EVIDENCE`
selects an existing run directory. Live availability depends on the host network.
Native Tauri, WebKit and other operating systems require separate verification.

## File-manager Markdown resources

```sh
NODE_OPTIONS=--no-experimental-webstorage CHAT_TERMINAL_EXECUTABLE='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm --prefix crates/app-tauri/frontend run test:terminal -- --markdown-resources
```

This scenario opens a nested Markdown document from the real file tree and
keeps the Preview tab selected. It checks decoded PNG/JPEG/GIF/WebP images,
alt text, sibling/parent and encoded/absolute/file references, numeric positions,
ordinary heading fragments and explicit query limitations. Light/dark and
programming/working modes cover small and expanded previews. Missing,
unsupported and oversized images remain visible as errors; file/root switches
discard pending images. The exact HTTPS failure fixture is intercepted and bypasses
the local reader; every other external request is blocked. Recorded fixture IPC
also verifies that unsafe paths cause no reads. IPC and external image responses
are simulated; the real ChatPage, Dock and Markdown components run in Chrome.

After images decode, each small/expanded preview receives 16 unrelated parent
updates over at least 3.2 seconds. The probe asserts retained image DOM identity,
zero extra image IPC, zero missing-image samples and at most 1 CSS px variation
in the text below the images. Mutation observations and 16 ms samples capture
transient replacements as well as settled geometry.

Each run keeps screenshots and results in a new
`.husk/scratch/workspace-markdown-image-stability/browser-*` directory, including its
first failure. `MARKDOWN_REMOTE_EVIDENCE` can place this regression inside a remote
image acceptance run. The runner owns an ephemeral port and closes its browser/server.
The current resource contract is maintained in
[the frontend blueprint](../../../../../.blueprint/tauri-frontend.md).
The workspace-files and chat-stream unit suites also cover processor context
changes with explicit root/document plugin options.

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
It also loads the bundled terminal font from production assets and checks actual
Canvas rasterization through xterm's computed font stack: default ASCII and Nerd
Mono icons, switching to an installed Linux monospace font, and an unavailable
family falling back to bundled JB without inheriting the app's code font. The
Linux font probe requires DejaVu Sans Mono, Liberation Mono or Noto Sans Mono.
This is native-WebKit production-resource coverage with simulated PTY/font-list
IPC, not a claim that an installed Tauri package or other OSes were exercised.
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

For the full settings overlay shell, use a fresh absolute output directory per
run (the directory must not exist; its parent must exist):

```sh
SETTINGS_SHELL_OUTPUT=/absolute/path/settings-mac-run \
  npm --prefix crates/app-tauri/frontend run test:terminal -- --settings-shell
SETTINGS_SHELL_OUTPUT=/absolute/path/settings-nonmac-run \
  npm --prefix crates/app-tauri/frontend run test:terminal -- --settings-shell --non-mac
```

This scenario keeps the real MainLayout, ChatPage and xterm mounted beneath the
production-shaped fixed settings wrapper. It checks light/dark/custom-font
geometry, transparent frame and opaque card surfaces, the retained private
sidebar width and DOM/renderer identities, navigation, scrolling, shared menu
hover/keyboard/Escape, terminal input and appearance-save calls. At 840, 600 and
480px heights it also measures the navigation rail: the rail must scroll on its
own, keep the title strip and return button in place, keep the return button as
wide as the navigation rows, leave no horizontal overflow and stay independent
of the content pane's scroll. The actual
sidebar-glass hook runs against recorded mock IPC; native material, real PTYs
and HMR are outside this browser scenario. `--settings-shell-baseline` records
pre-change screenshots and geometry only; acceptance runs omit that flag.

`node crates/app-tauri/frontend/scripts/terminal-reveal/sidebar-glass-check.mjs`
checks the shared shell surface in light, dark and custom themes, with native
material active or unavailable and at a narrow window width. It loads the
production boot styles, compares boot and workspace-loading backgrounds with
the frame, and verifies that loading conceals old content while retaining its
DOM and input value. Native IPC is recorded by the fixture.

Install Playwright's Chromium, or select an existing executable with
`CHAT_TERMINAL_EXECUTABLE`. `CHAT_TERMINAL_CPU=4` enables Chromium CPU throttling.
The optional `--native-webkit` development check uses the same GTK assertions
but loads through Vite's HTTP server; use the production checks above for
packaged-resource acceptance.

For the real ChatPage welcome activity card, use a fresh absolute output
directory whose parent exists:

```sh
WELCOME_RESPONSIVE_OUTPUT=/absolute/path/welcome-run \
  CHAT_TERMINAL_EXECUTABLE='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  NODE_OPTIONS=--no-experimental-webstorage \
  npm --prefix crates/app-tauri/frontend run test:terminal -- --welcome-responsive
```

This Chrome scenario uses fixture-only usage and recent-artifact IPC. It checks
both welcome callers with loaded and zero records, 513/514/600px actual welcome
content widths, light/dark themes and simulated Mac/non-Mac platforms. It verifies
greeting, primary CTA and recents clicks, header/month-label geometry, a wide
window squeezed by the real sidebar and Dock, width restoration, office
artifacts, the existing-session empty greeting and the settings heatmap's
unchanged 480px horizontal-scroll behavior. Screenshots and computed geometry
are saved beside `report.json`; browser/server resources close on failure too.
`--welcome-responsive-baseline` runs the first narrow regression assertion
against a pre-fix implementation and preserves its screenshot and geometry.

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
