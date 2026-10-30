import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { TerminalSettings } from "@/features/settings/pages/terminal";
import { deferred, mockCommand, tauri } from "@/test/tauri";
import {
  quoteFontFamily, terminalFontOptions, useSystemTerminalFonts,
  type SystemTerminalFont,
} from "../fonts";
import { DEFAULT_TERMINAL_PREFS, resolveTerminalFont, sanitizeTerminalPrefs, useTerminalPrefs } from "../preferences";

beforeEach(() => {
  useTerminalPrefs.setState(DEFAULT_TERMINAL_PREFS);
});

test("font choices use real families, deduplicate faces and put monospace first", () => {
  const options = terminalFontOptions([
    { family: "Arial", monospace: false },
    { family: "Iosevka Nerd Font Mono", monospace: true },
    { family: "Zed Mono", monospace: false },
    { family: "Zed Mono", monospace: true },
    { family: "arial", monospace: false },
    { family: "", monospace: true },
    { family: "bad\nfamily", monospace: true },
  ], "");
  expect(options.map((option) => option.value)).toEqual([
    "", '"Iosevka Nerd Font Mono"', '"Zed Mono"', '"Arial"',
  ]);
  expect(options[0].label).toBe("JetBrains Mono（内置）");
  expect(options[1].label).toBe("Iosevka Nerd Font Mono");
});

test.each(['A, B', 'A "quoted" Font', "A \\ Font", "中文等宽", "A; {} <Font>"])(
  "a real family %s survives storage and remains one CSS family", (family) => {
    const quoted = quoteFontFamily(family);
    expect(sanitizeTerminalPrefs({ fontFamily: quoted }).fontFamily).toBe(quoted);
    expect(resolveTerminalFont(quoted)).toBe(`${quoted}, "Husk Terminal Mono", monospace`);
    expect(terminalFontOptions([{ family, monospace: true }], quoted)[1]).toMatchObject({ value: quoted, label: family });
  },
);

test("an unavailable stored choice remains recognizable without posing as an installed font", () => {
  const options = terminalFontOptions([], '"Removed Mono"');
  expect(options).toHaveLength(2);
  expect(options[1]).toMatchObject({ value: '"Removed Mono"', label: "Removed Mono（已保存）" });
  expect(options[1].description).toContain("内置");
  expect(resolveTerminalFont(options[1].value)).toBe('"Removed Mono", "Husk Terminal Mono", monospace');
});

test("system font enumeration is asynchronous and never changes the user's preferences", async () => {
  const reply = deferred<SystemTerminalFont[]>();
  mockCommand("terminal_fonts", () => reply.promise);
  useTerminalPrefs.getState().update({ fontFamily: '"Old Mono"' });
  const { result } = renderHook(useSystemTerminalFonts);
  expect(result.current.status).toBe("loading");
  act(() => useTerminalPrefs.getState().update({ fontFamily: "" }));
  await act(async () => reply.resolve([{ family: "Old Mono", monospace: true }]));
  expect(result.current.status).toBe("ready");
  expect(result.current.fonts).toEqual([{ family: "Old Mono", monospace: true }]);
  expect(useTerminalPrefs.getState().fontFamily).toBe("");
  expect(tauri.invoke).toHaveBeenCalledWith("terminal_fonts");
});

test("a failed enumeration retains the bundled default", async () => {
  mockCommand("terminal_fonts", () => Promise.reject(new Error("unavailable")));
  const { result } = renderHook(useSystemTerminalFonts);
  await waitFor(() => expect(result.current.status).toBe("error"));
  expect(terminalFontOptions(result.current.fonts, "")).toEqual([
    { value: "", label: "JetBrains Mono（内置）" },
  ]);
});

test("a late font reply is ignored after the settings reader unmounts", async () => {
  const reply = deferred<SystemTerminalFont[]>();
  mockCommand("terminal_fonts", () => reply.promise);
  const { result, unmount } = renderHook(useSystemTerminalFonts);
  unmount();
  await act(async () => reply.resolve([{ family: "Late Mono", monospace: true }]));
  expect(result.current.status).toBe("loading");
});

test("settings use the native family for both stored selection and the live preview", async () => {
  const user = userEvent.setup();
  mockCommand("terminal_fonts", () => [{ family: "Iosevka Nerd Font Mono", monospace: true }]);
  document.documentElement.style.setProperty("--husk-code-font", "Consolas");
  const { container } = render(<TooltipProvider><TerminalSettings /></TooltipProvider>);
  const select = screen.getByRole("button", { name: "终端字体" });
  expect(select).toHaveTextContent("JetBrains Mono（内置）");
  await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith("terminal_fonts"));
  await user.click(select);
  await user.click(await screen.findByRole("menuitemradio", { name: /Iosevka Nerd Font Mono/ }));
  expect(useTerminalPrefs.getState().fontFamily).toBe('"Iosevka Nerd Font Mono"');
  expect(container.querySelector('[aria-hidden="true"][style]')?.getAttribute("style")).toContain('"Iosevka Nerd Font Mono", "Husk Terminal Mono", monospace');
  expect(container.querySelector('[aria-hidden="true"][style]')?.getAttribute("style")).not.toContain("Consolas");
  document.documentElement.style.removeProperty("--husk-code-font");
});
