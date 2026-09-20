/**
 * Theme presets — `theme_id` → base accent/background/foreground tokens.
 * A selection writes the preset into `appearance.json` (accent/background/
 * foreground); any later manual tweak to those three fields overrides the
 * preset without touching `theme_id`, so "custom" stays a live state.
 */

import type { AppearanceConfig } from "../invoke/agent";

export interface ThemePreset {
  id: string;
  name: string;
  /** Light-mode surface colors. */
  accent: string;
  background: string;
  foreground: string;
  /** Dark-mode counterparts — applied when the effective mode is dark. */
  darkAccent: string;
  darkBackground: string;
  darkForeground: string;
}

export const THEME_PRESETS: ThemePreset[] = [
  {
    id: "husk",
    name: "Husk",
    accent: "#339CFF",
    background: "#FFFFFF",
    foreground: "#1A1C1F",
    darkAccent: "#4DA3FF",
    darkBackground: "#141416",
    darkForeground: "#E4E4E7",
  },
  {
    id: "one-light",
    name: "One Light",
    accent: "#4078F2",
    background: "#FAFAFA",
    foreground: "#383A42",
    darkAccent: "#61AFEF",
    darkBackground: "#282C34",
    darkForeground: "#ABB2BF",
  },
  {
    id: "github-light",
    name: "GitHub Light",
    accent: "#0969DA",
    background: "#FFFFFF",
    foreground: "#1F2328",
    darkAccent: "#4493F8",
    darkBackground: "#0D1117",
    darkForeground: "#E6EDF3",
  },
  {
    id: "solarized-light",
    name: "Solarized Light",
    accent: "#268BD2",
    background: "#FDF6E3",
    foreground: "#657B83",
    darkAccent: "#268BD2",
    darkBackground: "#002B36",
    darkForeground: "#839496",
  },
];

export function presetFor(id: string | null | undefined): ThemePreset | undefined {
  return THEME_PRESETS.find((t) => t.id === id);
}

/** Merge a preset's colors into `cfg` — `dark` picks the dark side's
 *  tokens, otherwise the light side's. The caller persists + applies the
 *  result. Returns `cfg` unchanged for an unknown id. */
export function withPreset(
  cfg: AppearanceConfig,
  id: string,
  dark: boolean,
): AppearanceConfig {
  const p = presetFor(id);
  if (!p) return cfg;
  return {
    ...cfg,
    theme_id: id,
    accent: dark ? p.darkAccent : p.accent,
    background: dark ? p.darkBackground : p.background,
    foreground: dark ? p.darkForeground : p.foreground,
  };
}