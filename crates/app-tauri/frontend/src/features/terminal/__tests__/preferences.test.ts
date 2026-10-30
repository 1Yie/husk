import { beforeEach, expect, test } from "vitest";
import {
  DEFAULT_TERMINAL_PREFS,
  resolveTerminalFont,
  resolveTerminalPalette,
  sanitizeTerminalPrefs,
  useTerminalPrefs,
} from "../preferences";

beforeEach(() => {
  localStorage.clear();
  useTerminalPrefs.setState(DEFAULT_TERMINAL_PREFS);
});

test("a stored snapshot is clamped and unknown values fall back", () => {
  expect(
    sanitizeTerminalPrefs({
      scheme: "made-up",
      fontSize: 400,
      lineHeight: 0.2,
      cursorStyle: "beam",
      scrollback: -5,
      fontFamily: "x; } body { color: red",
      closeOnExit: "yes",
    }),
  ).toMatchObject({
    scheme: "husk",
    fontSize: 24,
    lineHeight: 1,
    cursorStyle: "block",
    scrollback: 500,
    fontFamily: "",
    closeOnExit: true,
  });
});

test("updates persist; reset keeps the dragged panel height", () => {
  useTerminalPrefs.getState().update({ fontSize: 15, panelHeight: 420 });
  expect(JSON.parse(localStorage.getItem("husk.terminal.v1")!)).toMatchObject({
    fontSize: 15,
    panelHeight: 420,
  });
  useTerminalPrefs.getState().reset();
  expect(useTerminalPrefs.getState()).toMatchObject({
    fontSize: 12,
    panelHeight: 420,
  });
});

test("跟随应用 takes the app's colours and switches with the theme", () => {
  const token = (name: string) =>
    (
      ({ "--husk-bg": "#101010", "--husk-fg": "#eeeeee" }) as Record<
        string,
        string
      >
    )[name];
  expect(resolveTerminalPalette("husk", true, token)).toMatchObject({
    background: "#101010",
    foreground: "#eeeeee",
  });
  expect(resolveTerminalPalette("husk", false).background).toBe("#ffffff");
  // A fixed scheme ignores the app's mode.
  expect(resolveTerminalPalette("dracula", false).background).toBe("#282a36");
});

test.each([
  ["", '"Husk Terminal Mono", monospace'],
  ['"Fira Code"', '"Fira Code", "Husk Terminal Mono", monospace'],
  ['"SF Mono", Menlo', '"SF Mono", "Menlo", "Husk Terminal Mono", monospace'],
  ['"JetBrains Mono"', '"JetBrains Mono", "Husk Terminal Mono", monospace'],
  ['"A, monospace, B"', '"A, monospace, B", "Husk Terminal Mono", monospace'],
  ["monospace", '"Husk Terminal Mono", monospace'],
])("terminal font %s falls back directly to the bundled Nerd Mono font", (chosen, expected) => {
  document.documentElement.style.setProperty("--husk-code-font", "Consolas");
  expect(resolveTerminalFont(chosen)).toBe(expected);
  document.documentElement.style.removeProperty("--husk-code-font");
});

test("legacy unquoted font choices are normalized without changing their families", () => {
  expect(sanitizeTerminalPrefs({ fontFamily: "Menlo" }).fontFamily).toBe('"Menlo"');
  expect(sanitizeTerminalPrefs({ fontFamily: '"SF Mono", Menlo' }).fontFamily).toBe('"SF Mono", "Menlo"');
});
