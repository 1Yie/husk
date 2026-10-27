// Terminal preferences — colour scheme, font, cursor and scrollback. A
// window-local UI preference like the dock and layout stores, persisted in
// localStorage. Kept free of xterm so the settings page can import it without
// pulling the renderer into its chunk.

import { create } from "zustand";

const STORAGE_KEY = "husk.terminal.v1";

export interface TerminalPalette {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

type Ansi = Omit<
  TerminalPalette,
  "background" | "foreground" | "cursor" | "selectionBackground"
>;

// xterm's built-in ANSI set is tuned for a dark screen; on a white one its
// yellow and white are unreadable. Every scheme ships all sixteen colours.
const GITHUB_LIGHT_ANSI: Ansi = {
  black: "#24292f",
  red: "#cf222e",
  green: "#116329",
  yellow: "#4d2d00",
  blue: "#0969da",
  magenta: "#8250df",
  cyan: "#1b7c83",
  white: "#6e7781",
  brightBlack: "#57606a",
  brightRed: "#a40e26",
  brightGreen: "#1a7f37",
  brightYellow: "#633c01",
  brightBlue: "#218bff",
  brightMagenta: "#a475f9",
  brightCyan: "#3192aa",
  brightWhite: "#8c959f",
};

const GITHUB_DARK_ANSI: Ansi = {
  black: "#484f58",
  red: "#ff7b72",
  green: "#3fb950",
  yellow: "#d29922",
  blue: "#58a6ff",
  magenta: "#bc8cff",
  cyan: "#39c5cf",
  white: "#b1bac4",
  brightBlack: "#6e7681",
  brightRed: "#ffa198",
  brightGreen: "#56d364",
  brightYellow: "#e3b341",
  brightBlue: "#79c0ff",
  brightMagenta: "#d2a8ff",
  brightCyan: "#56d4dd",
  brightWhite: "#ffffff",
};

export type TerminalSchemeId =
  | "husk"
  | "github-light"
  | "github-dark"
  | "one-dark"
  | "solarized-light"
  | "dracula"
  | "nord";

/** "husk" has no fixed palette: it takes the app's own surface and text
 *  colours and switches with the light/dark theme. */
export const TERMINAL_SCHEMES: {
  id: TerminalSchemeId;
  label: string;
  light?: TerminalPalette;
  dark?: TerminalPalette;
}[] = [
  {
    id: "husk",
    label: "跟随应用",
    light: {
      background: "#ffffff",
      foreground: "#1a1c1f",
      cursor: "#339cff",
      selectionBackground: "#339cff33",
      ...GITHUB_LIGHT_ANSI,
    },
    dark: {
      background: "#16161a",
      foreground: "#e4e4e7",
      cursor: "#4da3ff",
      selectionBackground: "#4da3ff40",
      ...GITHUB_DARK_ANSI,
    },
  },
  {
    id: "github-light",
    label: "GitHub 浅色",
    light: {
      background: "#ffffff",
      foreground: "#1f2328",
      cursor: "#0969da",
      selectionBackground: "#0969da33",
      ...GITHUB_LIGHT_ANSI,
    },
  },
  {
    id: "github-dark",
    label: "GitHub 深色",
    dark: {
      background: "#0d1117",
      foreground: "#e6edf3",
      cursor: "#2f81f7",
      selectionBackground: "#2f81f740",
      ...GITHUB_DARK_ANSI,
    },
  },
  {
    id: "one-dark",
    label: "One Dark",
    dark: {
      background: "#282c34",
      foreground: "#abb2bf",
      cursor: "#528bff",
      selectionBackground: "#3e4451",
      black: "#3f4451",
      red: "#e06c75",
      green: "#98c379",
      yellow: "#e5c07b",
      blue: "#61afef",
      magenta: "#c678dd",
      cyan: "#56b6c2",
      white: "#d7dae0",
      brightBlack: "#5c6370",
      brightRed: "#ea858b",
      brightGreen: "#aad581",
      brightYellow: "#ffd885",
      brightBlue: "#85c0fc",
      brightMagenta: "#d398eb",
      brightCyan: "#6ed5de",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "solarized-light",
    label: "Solarized 浅色",
    light: {
      background: "#fdf6e3",
      foreground: "#657b83",
      cursor: "#586e75",
      selectionBackground: "#eee8d5",
      black: "#073642",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#93a1a1",
      brightBlack: "#002b36",
      brightRed: "#cb4b16",
      brightGreen: "#586e75",
      brightYellow: "#657b83",
      brightBlue: "#839496",
      brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1",
      brightWhite: "#eee8d5",
    },
  },
  {
    id: "dracula",
    label: "Dracula",
    dark: {
      background: "#282a36",
      foreground: "#f8f8f2",
      cursor: "#f8f8f2",
      selectionBackground: "#44475a",
      black: "#21222c",
      red: "#ff5555",
      green: "#50fa7b",
      yellow: "#f1fa8c",
      blue: "#bd93f9",
      magenta: "#ff79c6",
      cyan: "#8be9fd",
      white: "#f8f8f2",
      brightBlack: "#6272a4",
      brightRed: "#ff6e6e",
      brightGreen: "#69ff94",
      brightYellow: "#ffffa5",
      brightBlue: "#d6acff",
      brightMagenta: "#ff92df",
      brightCyan: "#a4ffff",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "nord",
    label: "Nord",
    dark: {
      background: "#2e3440",
      foreground: "#d8dee9",
      cursor: "#d8dee9",
      selectionBackground: "#434c5e",
      black: "#3b4252",
      red: "#bf616a",
      green: "#a3be8c",
      yellow: "#ebcb8b",
      blue: "#81a1c1",
      magenta: "#b48ead",
      cyan: "#88c0d0",
      white: "#e5e9f0",
      brightBlack: "#4c566a",
      brightRed: "#bf616a",
      brightGreen: "#a3be8c",
      brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1",
      brightMagenta: "#b48ead",
      brightCyan: "#8fbcbb",
      brightWhite: "#eceff4",
    },
  },
];

/** `value: ""` follows the app's code font (外观). */
export const TERMINAL_FONTS: { value: string; label: string }[] = [
  { value: "", label: "跟随代码字体" },
  { value: '"JetBrains Mono"', label: "JetBrains Mono" },
  { value: '"Fira Code"', label: "Fira Code" },
  { value: '"Cascadia Code"', label: "Cascadia Code" },
  { value: '"SF Mono", Menlo', label: "SF Mono" },
  { value: "Menlo", label: "Menlo" },
  { value: "Consolas", label: "Consolas" },
  { value: '"Source Code Pro"', label: "Source Code Pro" },
  { value: '"Noto Sans Mono"', label: "Noto Sans Mono" },
];

export type TerminalCursorStyle = "block" | "bar" | "underline";

export interface TerminalPrefs {
  scheme: TerminalSchemeId;
  /** CSS font-family list; "" follows the app's code font. */
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  cursorStyle: TerminalCursorStyle;
  cursorBlink: boolean;
  scrollback: number;
  /** A shell that exits cleanly (`exit`, Ctrl-D) closes its tab. */
  closeOnExit: boolean;
  /** The panel's dragged height — not shown in 设置. */
  panelHeight: number;
}

export const TERMINAL_FONT_SIZE = { min: 10, max: 24 } as const;
export const TERMINAL_LINE_HEIGHTS = [1, 1.1, 1.2, 1.3, 1.4, 1.5] as const;
export const TERMINAL_SCROLLBACK = [1000, 3000, 5000, 10000, 20000] as const;

export const DEFAULT_TERMINAL_PREFS: TerminalPrefs = {
  scheme: "husk",
  fontFamily: "",
  fontSize: 12,
  lineHeight: 1.2,
  cursorStyle: "block",
  cursorBlink: true,
  scrollback: 3000,
  closeOnExit: true,
  panelHeight: 280,
};

const clampNumber = (
  value: unknown,
  min: number,
  max: number,
  fallback: number,
) =>
  typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, value))
    : fallback;

/** Anything read back from storage passes through here: an edited or older
 *  snapshot must never hand xterm an option it rejects. */
export function sanitizeTerminalPrefs(
  raw: Partial<Record<keyof TerminalPrefs, unknown>>,
): TerminalPrefs {
  const d = DEFAULT_TERMINAL_PREFS;
  const scheme = TERMINAL_SCHEMES.some((s) => s.id === raw.scheme)
    ? (raw.scheme as TerminalSchemeId)
    : d.scheme;
  const fontFamily =
    typeof raw.fontFamily === "string" &&
    raw.fontFamily.length <= 200 &&
    !/[;{}<>]/.test(raw.fontFamily)
      ? raw.fontFamily.trim()
      : d.fontFamily;
  const cursorStyle =
    raw.cursorStyle === "bar" || raw.cursorStyle === "underline"
      ? raw.cursorStyle
      : "block";
  const lineHeight = clampNumber(raw.lineHeight, 1, 2, d.lineHeight);
  return {
    scheme,
    fontFamily,
    fontSize: Math.round(
      clampNumber(
        raw.fontSize,
        TERMINAL_FONT_SIZE.min,
        TERMINAL_FONT_SIZE.max,
        d.fontSize,
      ),
    ),
    lineHeight: Math.round(lineHeight * 10) / 10,
    cursorStyle,
    cursorBlink:
      typeof raw.cursorBlink === "boolean" ? raw.cursorBlink : d.cursorBlink,
    scrollback: Math.round(
      clampNumber(raw.scrollback, 500, 50000, d.scrollback),
    ),
    closeOnExit:
      typeof raw.closeOnExit === "boolean" ? raw.closeOnExit : d.closeOnExit,
    panelHeight: Math.round(
      clampNumber(raw.panelHeight, 120, 2000, d.panelHeight),
    ),
  };
}

function load(): TerminalPrefs {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw
      ? sanitizeTerminalPrefs(JSON.parse(raw))
      : DEFAULT_TERMINAL_PREFS;
  } catch {
    return DEFAULT_TERMINAL_PREFS;
  }
}

function save(prefs: TerminalPrefs) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // A blocked/full quota must not break the terminal.
  }
}

interface TerminalPrefsState extends TerminalPrefs {
  update: (patch: Partial<TerminalPrefs>) => void;
  /** Back to defaults, keeping the panel's dragged height. */
  reset: () => void;
}

const prefsOf = (s: TerminalPrefsState): TerminalPrefs => {
  const { update: _update, reset: _reset, ...prefs } = s;
  return prefs;
};

export const useTerminalPrefs = create<TerminalPrefsState>((set, get) => ({
  ...load(),
  update: (patch) => {
    const next = sanitizeTerminalPrefs({ ...prefsOf(get()), ...patch });
    set(next);
    save(next);
  },
  reset: () => {
    const next = { ...DEFAULT_TERMINAL_PREFS, panelHeight: get().panelHeight };
    set(next);
    save(next);
  },
}));

export function terminalPrefsNow(): TerminalPrefs {
  return prefsOf(useTerminalPrefs.getState());
}

/** The palette for `scheme` under the app's current light/dark mode. Fixed
 *  schemes ignore the mode; "husk" takes the app's surface colours on top. */
export function resolveTerminalPalette(
  scheme: TerminalSchemeId,
  dark: boolean,
  token: (name: string) => string | undefined = () => undefined,
): TerminalPalette {
  const entry =
    TERMINAL_SCHEMES.find((s) => s.id === scheme) ?? TERMINAL_SCHEMES[0];
  const base = (
    dark ? (entry.dark ?? entry.light) : (entry.light ?? entry.dark)
  )!;
  if (entry.id !== "husk") return base;
  return {
    ...base,
    background: token("--husk-bg") ?? base.background,
    foreground: token("--husk-fg") ?? base.foreground,
    cursor: token("--husk-accent") ?? base.cursor,
  };
}

/** A chosen family keeps the app's code font behind it, so CJK and missing
 *  glyphs fall back to something monospaced rather than the browser default. */
export function resolveTerminalFont(
  fontFamily: string,
  codeFont: string | undefined,
): string {
  const fallback = codeFont || "ui-monospace, monospace";
  return fontFamily ? `${fontFamily}, ${fallback}` : fallback;
}

export function isAppDark(): boolean {
  return (
    typeof document !== "undefined" &&
    document.documentElement.classList.contains("dark")
  );
}
