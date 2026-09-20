import { useEffect, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Copy,
  Check,
  Upload,
} from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { SettingsRenderer, SettingSelect } from "@/components/settings";
import { getAppearance, setAppearance, type AppearanceConfig } from "../../invoke/agent/sessions";
import { applyAppearance, broadcastAppearance, isDarkMode } from "../../lib/appearance";
import { THEME_PRESETS, withPreset, presetFor } from "../../lib/themes";

/** Palette defaults shown in the color fields when no dark override is
 *  stored — mirrors the `.dark` tokens in index.css. */
const DARK_FALLBACK = { accent: "#4DA3FF", background: "#16161A", foreground: "#E4E4E7" };

/** The accent/bg/fg this config resolves to under `dark` — dark overrides
 *  win, then the preset's dark side, then the palette fallback. */
function effectiveColors(cfg: AppearanceConfig, dark: boolean) {
  const p = presetFor(cfg.theme_id);
  return {
    accent: (dark && (cfg.dark_accent ?? p?.darkAccent)) || cfg.accent,
    background: (dark && (cfg.dark_background ?? p?.darkBackground)) || (dark ? DARK_FALLBACK.background : cfg.background),
    foreground: (dark && (cfg.dark_foreground ?? p?.darkForeground)) || (dark ? DARK_FALLBACK.foreground : cfg.foreground),
  };
}

export function AppearanceSettings() {
  const [themeMode, setThemeModeState] = useState<"system" | "light" | "dark">("system");
  const [selectedTheme, setSelectedThemeState] = useState("husk");

  const [accentColor, setAccentColorState] = useState("#339CFF");
  const [bgColor, setBgColorState] = useState("#FFFFFF");
  const [fgColor, setFgColorState] = useState("#1A1C1F");
  const [uiFont, setUiFontState] = useState('-apple-system, BlinkMacSystemFont, "Segoe UI"');
  const [codeFont, setCodeFontState] = useState('ui-monospace, "SFMono-Regular", monospace');
  const [contrast, setContrastState] = useState(45);

  const [copied, setCopied] = useState(false);
  // First-load flag: applying the fetched config must not echo back a
  // `set_appearance` write (harmless but wasteful).
  const hydrated = useRef(false);
  // The full last-known config — local color states only mirror the
  // *current mode's* fields, so update() merges patches over this ref to
  // avoid dropping the other mode's stored values.
  const cfgRef = useRef<AppearanceConfig | null>(null);

  // Load the persisted config once, then apply it so the settings window
  // itself renders in the user's theme (it's a separate webview — no
  // shared state with the main window).
  useEffect(() => {
    void getAppearance()
      .then((cfg) => {
        cfgRef.current = cfg;
        const dark = isDarkMode(cfg);
        const eff = effectiveColors(cfg, dark);
        setThemeModeState(cfg.theme_mode);
        if (cfg.theme_id) setSelectedThemeState(cfg.theme_id);
        setAccentColorState(eff.accent);
        setBgColorState(eff.background);
        setFgColorState(eff.foreground);
        setUiFontState(cfg.ui_font);
        setCodeFontState(cfg.code_font);
        setContrastState(cfg.contrast);
        applyAppearance(cfg);
        hydrated.current = true;
      })
      .catch(() => {});
  }, []);

  /** One setter per field — updates local state, re-applies the theme for
   *  live preview, and persists the merged config. */
  const update = (patch: Partial<AppearanceConfig>) => {
    const base = cfgRef.current ?? {
      theme_mode: themeMode,
      theme_id: selectedTheme,
      accent: accentColor,
      background: bgColor,
      foreground: fgColor,
      ui_font: uiFont,
      code_font: codeFont,
      contrast,
    };
    const cfg: AppearanceConfig = { ...base, ...patch };
    cfgRef.current = cfg;
    applyAppearance(cfg);
    if (hydrated.current) {
      void setAppearance(patch)
        .then(() => broadcastAppearance())
        .catch((e) => console.error("save appearance:", e));
    }
  };

  const setThemeMode = (v: "system" | "light" | "dark") => {
    setThemeModeState(v);
    update({ theme_mode: v });
    // Re-show the color fields for the new mode's stored values.
    const c = cfgRef.current;
    if (c) {
      const eff = effectiveColors(c, isDarkMode(c));
      setAccentColorState(eff.accent);
      setBgColorState(eff.background);
      setFgColorState(eff.foreground);
    }
  };
  // Picking a preset theme writes its colors into accent/background/
  // foreground — a later manual tweak keeps the id but overrides the token,
  // so "custom" is just the live state of the three color fields.
  const setSelectedTheme = (v: string) => {
    setSelectedThemeState(v);
    const cur = cfgRef.current ?? {
      theme_mode: themeMode, theme_id: selectedTheme, accent: accentColor,
      background: bgColor, foreground: fgColor, ui_font: uiFont,
      code_font: codeFont, contrast,
    };
    const next = withPreset(cur, v, isDarkMode(cur));
    const eff = effectiveColors(next, isDarkMode(next));
    setAccentColorState(eff.accent);
    setBgColorState(eff.background);
    setFgColorState(eff.foreground);
    update({
      theme_id: v,
      accent: next.accent,
      background: next.background,
      foreground: next.foreground,
      dark_accent: next.dark_accent,
      dark_background: next.dark_background,
      dark_foreground: next.dark_foreground,
    });
  };
  const dark = themeMode === "dark" || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const setAccentColor = (v: string) => { setAccentColorState(v); update(dark ? { dark_accent: v } : { accent: v }); };
  const setBgColor = (v: string) => { setBgColorState(v); update(dark ? { dark_background: v } : { background: v }); };
  const setFgColor = (v: string) => { setFgColorState(v); update(dark ? { dark_foreground: v } : { foreground: v }); };
  const setUiFont = (v: string) => { setUiFontState(v); update({ ui_font: v }); };
  const setCodeFont = (v: string) => { setCodeFontState(v); update({ code_font: v }); };
  const setContrast = (v: number) => { setContrastState(v); update({ contrast: v }); };

  const handleCopyTheme = () => {
    const config = {
      name: selectedTheme,
      mode: themeMode,
      accent: accentColor,
      background: bgColor,
      foreground: fgColor,
      uiFont,
      codeFont,
      contrast,
    };
    void navigator.clipboard.writeText(JSON.stringify(config, null, 2));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleImportTheme = () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json";
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        const json = JSON.parse(text);
        const patch: Partial<AppearanceConfig> = {};
        // Colors land on the current mode's side — a theme imported in
        // dark mode customizes the dark surface, not the light one.
        if (json.accent) (dark ? (patch.dark_accent = json.accent) : (patch.accent = json.accent));
        if (json.background) (dark ? (patch.dark_background = json.background) : (patch.background = json.background));
        if (json.foreground) (dark ? (patch.dark_foreground = json.foreground) : (patch.foreground = json.foreground));
        if (json.contrast !== undefined) patch.contrast = Number(json.contrast);
        if (json.uiFont) patch.ui_font = json.uiFont;
        if (json.codeFont) patch.code_font = json.codeFont;
        if (json.mode) patch.theme_mode = json.mode;
        if (json.name) patch.theme_id = json.name;
        // Update local state to match, then persist the merged patch.
        if (patch.accent ?? patch.dark_accent) setAccentColorState((patch.accent ?? patch.dark_accent)!);
        if (patch.background ?? patch.dark_background) setBgColorState((patch.background ?? patch.dark_background)!);
        if (patch.foreground ?? patch.dark_foreground) setFgColorState((patch.foreground ?? patch.dark_foreground)!);
        if (patch.contrast !== undefined) setContrastState(patch.contrast);
        if (patch.ui_font) setUiFontState(patch.ui_font);
        if (patch.code_font) setCodeFontState(patch.code_font);
        if (patch.theme_mode) setThemeModeState(patch.theme_mode);
        if (patch.theme_id) setSelectedThemeState(patch.theme_id);
        update(patch);
      } catch (err) {
        console.error("Failed to import theme JSON:", err);
      }
    };
    input.click();
  };

  return (
    <div className="flex flex-col gap-6 max-w-3xl">
      <div>
        <h2 className="text-base font-semibold text-neutral-900 tracking-tight">
          外观
        </h2>
      </div>

      <div className="grid grid-cols-3 gap-4">
        <Button
          type="button"
          variant="ghost"
          onClick={() => setThemeMode("system")}
          className="flex flex-col items-center cursor-pointer group select-none text-left p-0 h-auto w-full border-0 bg-transparent hover:bg-transparent focus-visible:outline-none focus-visible:ring-0"
        >
          <div
            className={cn(
              "w-full h-[126px] rounded-2xl overflow-hidden transition-colors relative flex items-end justify-center",
              themeMode === "system"
                ? "border-2 border-[#339CFF] ring-2 ring-[#339CFF]/25 shadow-xs"
                : "border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] hover:border-neutral-300"
            )}
          >
            <div className="absolute inset-0 flex">
              <div className="w-1/2 h-full bg-[#627181]" />
              <div className="w-1/2 h-full bg-[#272a2e]" />
            </div>

            <div className="relative w-[86%] h-[82px] rounded-t-xl overflow-hidden flex shadow-md border-t border-x border-[#09090b]/15">
              <div className="w-1/2 h-full bg-[#ffffff] p-3 flex flex-col gap-2">
                <div className="h-1.5 w-10 bg-[#d4d4d8] rounded-full" />
                <div className="flex flex-col gap-1.5 mt-0.5">
                  <div className="h-1.5 w-12 bg-[#e4e4e7] rounded-full" />
                  <div className="h-1.5 w-16 bg-[#e4e4e7] rounded-full" />
                  <div className="h-1.5 w-10 bg-[#e4e4e7] rounded-full" />
                </div>
              </div>

              <div className="w-1/2 h-full bg-[#2f3338] p-3 flex flex-col gap-2 border-l border-[#3f3f46]/60">
                <div className="h-1.5 w-10 bg-[#52525b] rounded-full" />
                <div className="flex flex-col gap-1.5 mt-0.5">
                  <div className="h-1.5 w-12 bg-[#52525b] rounded-full" />
                  <div className="h-1.5 w-16 bg-[#52525b] rounded-full" />
                  <div className="h-1.5 w-10 bg-[#52525b] rounded-full" />
                </div>
              </div>
            </div>
          </div>
          <span
            className={cn(
              "text-xs mt-2.5 transition-colors",
              themeMode === "system" ? "font-semibold text-neutral-900" : "font-normal text-neutral-600"
            )}
          >
            系统
          </span>
        </Button>

        <Button
          type="button"
          variant="ghost"
          onClick={() => setThemeMode("light")}
          className="flex flex-col items-center cursor-pointer group select-none text-left p-0 h-auto w-full border-0 bg-transparent hover:bg-transparent focus-visible:outline-none focus-visible:ring-0"
        >
          <div
            className={cn(
              "w-full h-[126px] rounded-2xl overflow-hidden transition-colors relative flex flex-col justify-between items-center pt-3.5 bg-[#f2f4f6]",
              themeMode === "light"
                ? "border-2 border-[#339CFF] ring-2 ring-[#339CFF]/25 shadow-xs"
                : "border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] hover:border-neutral-300"
            )}
          >
            <div className="h-1.5 w-24 bg-[#d4d4d8]/80 rounded-full" />

            <div className="w-[86%] h-[82px] rounded-t-xl bg-[#ffffff] border-t border-x border-[#e4e4e7]/80 p-3 flex flex-col gap-1.5 shadow-sm">
              <div className="h-1.5 w-14 bg-[#e4e4e7] rounded-full" />
              <div className="h-1.5 w-20 bg-[#e4e4e7] rounded-full" />
              <div className="h-1.5 w-12 bg-[#e4e4e7] rounded-full" />
            </div>
          </div>
          <span
            className={cn(
              "text-xs mt-2.5 transition-colors",
              themeMode === "light" ? "font-semibold text-neutral-900" : "font-normal text-neutral-600"
            )}
          >
            浅色
          </span>
        </Button>

        <Button
          type="button"
          variant="ghost"
          onClick={() => setThemeMode("dark")}
          className="flex flex-col items-center cursor-pointer group select-none text-left p-0 h-auto w-full border-0 bg-transparent hover:bg-transparent focus-visible:outline-none focus-visible:ring-0"
        >
          <div
            className={cn(
              "w-full h-[126px] rounded-2xl overflow-hidden transition-colors relative flex flex-col justify-between items-center pt-3.5 bg-[#43474d]",
              themeMode === "dark"
                ? "border-2 border-[#339CFF] ring-2 ring-[#339CFF]/25 shadow-xs"
                : "border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] hover:border-neutral-300"
            )}
          >
            <div className="h-1.5 w-24 bg-[#71717a]/80 rounded-full" />

            <div className="w-[86%] h-[82px] rounded-t-xl bg-[#1c1c1f] border-t border-x border-[#3f3f46]/80 p-3 flex flex-col gap-1.5 shadow-sm">
              <div className="h-1.5 w-14 bg-[#3f3f46] rounded-full" />
              <div className="h-1.5 w-20 bg-[#3f3f46] rounded-full" />
              <div className="h-1.5 w-12 bg-[#3f3f46] rounded-full" />
            </div>
          </div>
          <span
            className={cn(
              "text-xs mt-2.5 transition-colors",
              themeMode === "dark" ? "font-semibold text-neutral-900" : "font-normal text-neutral-600"
            )}
          >
            深色
          </span>
        </Button>
      </div>

      <div className="border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] rounded-2xl bg-white shadow-2xs overflow-hidden text-[13px] font-mono leading-relaxed select-text">
        <div className="grid grid-cols-2 divide-x divide-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)]">
          <div className="py-2.5 flex flex-col">
            <div className="flex items-center px-3 py-0.5">
              <span className="w-6 text-right text-neutral-400 text-xs pr-3 select-none">1</span>
              <div className="whitespace-pre">
                <span className="text-[#7c3aed] dark:text-[#a78bfa] font-medium">const </span>
                <span className="text-[#b45309] dark:text-[#fbbf24]">themePreview</span>
                <span className="text-neutral-500">: </span>
                <span className="text-[#6366f1] dark:text-[#818cf8]">ThemeConfig </span>
                <span className="text-[#2563eb] dark:text-[#60a5fa]">= </span>
                <span className="text-neutral-700">{"{"}</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] dark:bg-[#3b2226] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] dark:text-[#f87171] text-xs pr-3 select-none font-semibold">2</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c] dark:text-[#f87171]">  surface: </span>
                <span className="text-[#059669] dark:text-[#4ade80]">"sidebar"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] dark:bg-[#3b2226] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] dark:text-[#f87171] text-xs pr-3 select-none font-semibold">3</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c] dark:text-[#f87171]">  accent: </span>
                <span className="text-[#059669] dark:text-[#4ade80]">"#2563eb"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] dark:bg-[#3b2226] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] dark:text-[#f87171] text-xs pr-3 select-none font-semibold">4</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c] dark:text-[#f87171]">  contrast: </span>
                <span className="text-[#2563eb] dark:text-[#60a5fa]">42</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center px-3 py-0.5">
              <span className="w-6 text-right text-neutral-400 text-xs pr-3 select-none">5</span>
              <div className="whitespace-pre">
                <span className="text-neutral-700">{"};"}</span>
              </div>
            </div>
          </div>

          <div className="py-2.5 flex flex-col">
            <div className="flex items-center px-3 py-0.5">
              <span className="w-6 text-right text-neutral-400 text-xs pr-3 select-none">1</span>
              <div className="whitespace-pre">
                <span className="text-[#7c3aed] dark:text-[#a78bfa] font-medium">const </span>
                <span className="text-[#b45309] dark:text-[#fbbf24]">themePreview</span>
                <span className="text-neutral-500">: </span>
                <span className="text-[#6366f1] dark:text-[#818cf8]">ThemeConfig </span>
                <span className="text-[#2563eb] dark:text-[#60a5fa]">= </span>
                <span className="text-neutral-700">{"{"}</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] dark:bg-[#1c3328] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a] dark:bg-[#22c55e]" />
              <span className="w-9 text-right text-[#16a34a] dark:text-[#4ade80] text-xs pr-3 select-none font-semibold">2</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d] dark:text-[#4ade80]">  surface: </span>
                <span className="text-[#16a34a] dark:text-[#4ade80]">"sidebar-elevated"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] dark:bg-[#1c3328] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a] dark:bg-[#22c55e]" />
              <span className="w-9 text-right text-[#16a34a] dark:text-[#4ade80] text-xs pr-3 select-none font-semibold">3</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d] dark:text-[#4ade80]">  accent: </span>
                <span className="text-[#16a34a] dark:text-[#4ade80]">"{accentColor.toLowerCase()}"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] dark:bg-[#1c3328] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a] dark:bg-[#22c55e]" />
              <span className="w-9 text-right text-[#16a34a] dark:text-[#4ade80] text-xs pr-3 select-none font-semibold">4</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d] dark:text-[#4ade80]">  contrast: </span>
                <span className="text-[#2563eb] dark:text-[#60a5fa]">{contrast}</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center px-3 py-0.5">
              <span className="w-6 text-right text-neutral-400 text-xs pr-3 select-none">5</span>
              <div className="whitespace-pre">
                <span className="text-neutral-700">{"};"}</span>
              </div>
            </div>
          </div>
        </div>

        <div className="h-5 bg-[color-mix(in_srgb,var(--husk-n50)_70%,transparent)] border-t border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] grid grid-cols-2 divide-x divide-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] items-center select-none">
          <div className="flex items-center justify-between px-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronLeft className="w-3 h-3" />
            </Button>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronRight className="w-3 h-3" />
            </Button>
          </div>
          <div className="flex items-center justify-between px-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronLeft className="w-3 h-3" />
            </Button>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronRight className="w-3 h-3" />
            </Button>
          </div>
        </div>
      </div>

      <SettingsRenderer
        sections={[
          {
            kind: "list",
            key: "theme",
            title: themeMode === "dark" ? "深色主题" : "浅色主题",
            actions: (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleImportTheme}
                  className="h-8 px-2.5 text-xs font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 gap-1.5 rounded-lg"
                >
                  <Upload className="w-3.5 h-3.5 text-neutral-500" />
                  <span>导入</span>
                </Button>

                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleCopyTheme}
                  className="h-8 px-2.5 text-xs font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 gap-1.5 rounded-lg"
                >
                  {copied ? (
                    <>
                      <Check className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                      <span className="text-emerald-600 dark:text-emerald-400 font-semibold">已复制</span>
                    </>
                  ) : (
                    <>
                      <Copy className="w-3.5 h-3.5 text-neutral-500" />
                      <span>复制主题</span>
                    </>
                  )}
                </Button>

                <SettingSelect
                  value={selectedTheme}
                  onChange={setSelectedTheme}
                  placeholder="选择主题"
                  prefix={
                    <span className="bg-[#e0edff] text-[#2563eb] dark:text-[#60a5fa] text-[10.5px] font-bold px-1.5 py-0.5 rounded leading-none">
                      Aa
                    </span>
                  }
                  options={THEME_PRESETS.map((t) => ({ value: t.id, label: t.name }))}
                />
              </>
            ),
            fields: [
              {
                key: "accent",
                type: "color",
                label: "强调色",
                value: accentColor,
                onChange: setAccentColor,
              },
              {
                key: "bg",
                type: "color",
                label: "背景",
                value: bgColor,
                onChange: setBgColor,
              },
              {
                key: "fg",
                type: "color",
                label: "前景",
                value: fgColor,
                onChange: setFgColor,
              },
              {
                key: "uiFont",
                type: "input",
                label: "UI 字体",
                value: uiFont,
                onChange: setUiFont,
              },
              {
                key: "codeFont",
                type: "input",
                label: "代码字体",
                value: codeFont,
                onChange: setCodeFont,
              },
              {
                key: "contrast",
                type: "slider",
                label: "对比度",
                value: contrast,
                onChange: setContrast,
                min: 0,
                max: 100,
                step: 1,
              },
            ],
          },
        ]}
      />
    </div>
  );
}
