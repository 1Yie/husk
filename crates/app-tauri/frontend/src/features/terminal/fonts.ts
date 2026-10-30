import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** A private alias keeps the bundled fallback independent of installed fonts. */
export const BUNDLED_TERMINAL_FONT = "Husk Terminal Mono";
export const BUNDLED_TERMINAL_FONT_STACK = '"Husk Terminal Mono", monospace';

export interface SystemTerminalFont {
  family: string;
  monospace: boolean;
}

const validFamily = (family: string) =>
  family.length > 0 && family.length <= 400 && !/[\x00-\x1f\x7f-\x9f]/.test(family);

/** Family names are data, never a CSS list (a real name may contain commas). */
export function quoteFontFamily(family: string): string {
  return `"${family.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

const GENERIC_FAMILIES = new Set([
  "monospace", "ui-monospace", "serif", "sans-serif", "cursive", "fantasy",
  "system-ui", "ui-serif", "ui-sans-serif", "ui-rounded", "emoji", "math",
  "fangsong", "inherit", "initial", "unset", "revert", "revert-layer",
]);

/** Accept the old quoted/unquoted CSS lists, but serialize only literal names.
 * Invalid storage is discarded as a whole instead of passing CSS to xterm. */
export function terminalFontFamilies(value: string): string[] {
  let rest = value.trim();
  if (rest.length > 1200 || /[\x00-\x1f\x7f-\x9f]/.test(rest)) return [];
  const families = new Map<string, string>();
  while (rest) {
    const match = /^(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)'|([\p{L}\p{M}\p{N}_ -]+))\s*(,|$)/u.exec(rest);
    if (!match) return [];
    const quoted = match[1] ?? match[2];
    const family = quoted === undefined ? match[3].trim().replace(/ +/g, " ")
      : quoted.replace(/\\(?:([\da-f]{1,6}) ?|(.))/gi, (_, hex: string | undefined, char: string) => {
        const point = hex ? parseInt(hex, 16) : 0;
        return hex ? String.fromCodePoint(point > 0 && point <= 0x10ffff ? point : 0xfffd) : char;
      });
    if (!validFamily(family)) return [];
    const key = family.toLowerCase();
    if (key !== BUNDLED_TERMINAL_FONT.toLowerCase()
      && (quoted !== undefined || !GENERIC_FAMILIES.has(key))) families.set(key, family);
    rest = rest.slice(match[0].length).trimStart();
    if (match[4] && !rest) return [];
  }
  return [...families.values()];
}

export function normalizeTerminalFont(value: unknown): string {
  return typeof value === "string" ? terminalFontFamilies(value).map(quoteFontFamily).join(", ") : "";
}

function normalizeSystemFonts(raw: unknown): SystemTerminalFont[] {
  const families = new Map<string, SystemTerminalFont>();
  if (!Array.isArray(raw)) return [];
  for (const font of raw) {
    if (!font || typeof font.family !== "string" || typeof font.monospace !== "boolean") continue;
    const family = font.family.trim();
    if (!validFamily(family)) continue;
    const key = family.toLowerCase();
    const previous = families.get(key);
    if (previous) previous.monospace ||= font.monospace;
    else families.set(key, { family, monospace: font.monospace });
  }
  return [...families.values()].sort((a, b) => Number(b.monospace) - Number(a.monospace)
    || a.family.toLowerCase().localeCompare(b.family.toLowerCase()));
}

export function terminalFontOptions(fonts: SystemTerminalFont[], selected: string) {
  const options: { value: string; label: string; description?: string }[] = [
    { value: "", label: "JetBrains Mono（内置）" },
    ...normalizeSystemFonts(fonts).map((font) => ({
      value: quoteFontFamily(font.family), label: font.family,
      ...(font.monospace ? { description: "等宽" } : {}),
    })),
  ];
  if (selected && !options.some((option) => option.value === selected)) {
    options.splice(1, 0, {
      value: selected, label: `${terminalFontFamilies(selected).join(", ")}（已保存）`,
      description: "不可用时使用内置字体",
    });
  }
  return options;
}

/** Re-read on each settings visit so newly installed fonts are discoverable. */
export function useSystemTerminalFonts() {
  const [state, setState] = useState<{
    fonts: SystemTerminalFont[]; status: "loading" | "ready" | "error";
  }>({ fonts: [], status: "loading" });
  useEffect(() => {
    let cancelled = false;
    void invoke<SystemTerminalFont[]>("terminal_fonts").then(
      (fonts) => { if (!cancelled) setState({ fonts: normalizeSystemFonts(fonts), status: "ready" }); },
      () => { if (!cancelled) setState({ fonts: [], status: "error" }); },
    );
    return () => { cancelled = true; };
  }, []);
  return state;
}
