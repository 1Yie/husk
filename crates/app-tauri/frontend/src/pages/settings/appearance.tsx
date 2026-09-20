import { useState } from "react";
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

interface ThemeOption {
  id: string;
  name: string;
}

const THEME_LIST: ThemeOption[] = [
  { id: "codex", name: "Codex" },
  { id: "one-light", name: "One Light" },
  { id: "github-light", name: "GitHub Light" },
  { id: "solarized-light", name: "Solarized Light" },
];

export function AppearanceSettings() {
  const [themeMode, setThemeMode] = useState<"system" | "light" | "dark">("system");
  const [selectedTheme, setSelectedTheme] = useState("codex");

  const [accentColor, setAccentColor] = useState("#339CFF");
  const [bgColor, setBgColor] = useState("#FFFFFF");
  const [fgColor, setFgColor] = useState("#1A1C1F");
  const [uiFont, setUiFont] = useState('-apple-system, BlinkMacSystemFont, "Segoe UI"');
  const [codeFont, setCodeFont] = useState('ui-monospace, "SFMono-Regular", monospace');
  const [contrast, setContrast] = useState(45);

  const [copied, setCopied] = useState(false);

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
        if (json.accent) setAccentColor(json.accent);
        if (json.background) setBgColor(json.background);
        if (json.foreground) setFgColor(json.foreground);
        if (json.contrast !== undefined) setContrast(Number(json.contrast));
        if (json.uiFont) setUiFont(json.uiFont);
        if (json.codeFont) setCodeFont(json.codeFont);
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
                : "border border-neutral-200/90 hover:border-neutral-300"
            )}
          >
            <div className="absolute inset-0 flex">
              <div className="w-1/2 h-full bg-[#627181]" />
              <div className="w-1/2 h-full bg-[#272a2e]" />
            </div>

            <div className="relative w-[86%] h-[82px] rounded-t-xl overflow-hidden flex shadow-md border-t border-x border-black/15">
              <div className="w-1/2 h-full bg-white p-3 flex flex-col gap-2">
                <div className="h-1.5 w-10 bg-neutral-300 rounded-full" />
                <div className="flex flex-col gap-1.5 mt-0.5">
                  <div className="h-1.5 w-12 bg-neutral-200 rounded-full" />
                  <div className="h-1.5 w-16 bg-neutral-200 rounded-full" />
                  <div className="h-1.5 w-10 bg-neutral-200 rounded-full" />
                </div>
              </div>

              <div className="w-1/2 h-full bg-[#2f3338] p-3 flex flex-col gap-2 border-l border-neutral-700/60">
                <div className="h-1.5 w-10 bg-neutral-600 rounded-full" />
                <div className="flex flex-col gap-1.5 mt-0.5">
                  <div className="h-1.5 w-12 bg-neutral-600 rounded-full" />
                  <div className="h-1.5 w-16 bg-neutral-600 rounded-full" />
                  <div className="h-1.5 w-10 bg-neutral-600 rounded-full" />
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
                : "border border-neutral-200/90 hover:border-neutral-300"
            )}
          >
            <div className="h-1.5 w-24 bg-neutral-300/80 rounded-full" />

            <div className="w-[86%] h-[82px] rounded-t-xl bg-white border-t border-x border-neutral-200/80 p-3 flex flex-col gap-1.5 shadow-sm">
              <div className="h-1.5 w-14 bg-neutral-200 rounded-full" />
              <div className="h-1.5 w-20 bg-neutral-200 rounded-full" />
              <div className="h-1.5 w-12 bg-neutral-200 rounded-full" />
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
                : "border border-neutral-200/90 hover:border-neutral-300"
            )}
          >
            <div className="h-1.5 w-24 bg-neutral-500/80 rounded-full" />

            <div className="w-[86%] h-[82px] rounded-t-xl bg-white border-t border-x border-neutral-300/80 p-3 flex flex-col gap-1.5 shadow-sm">
              <div className="h-1.5 w-14 bg-neutral-200 rounded-full" />
              <div className="h-1.5 w-20 bg-neutral-200 rounded-full" />
              <div className="h-1.5 w-12 bg-neutral-200 rounded-full" />
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

      <div className="border border-neutral-200/90 rounded-2xl bg-white shadow-2xs overflow-hidden text-[13px] font-mono leading-relaxed select-text">
        <div className="grid grid-cols-2 divide-x divide-neutral-200/80">
          <div className="py-2.5 flex flex-col">
            <div className="flex items-center px-3 py-0.5">
              <span className="w-6 text-right text-neutral-400 text-xs pr-3 select-none">1</span>
              <div className="whitespace-pre">
                <span className="text-[#7c3aed] font-medium">const </span>
                <span className="text-[#b45309]">themePreview</span>
                <span className="text-neutral-500">: </span>
                <span className="text-[#6366f1]">ThemeConfig </span>
                <span className="text-[#2563eb]">= </span>
                <span className="text-neutral-700">{"{"}</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] text-xs pr-3 select-none font-semibold">2</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c]">  surface: </span>
                <span className="text-[#059669]">"sidebar"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] text-xs pr-3 select-none font-semibold">3</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c]">  accent: </span>
                <span className="text-[#059669]">"#2563eb"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#fef2f2] py-0.5">
              <div
                className="absolute left-0 top-0 bottom-0 w-1"
                style={{
                  backgroundImage:
                    "repeating-linear-gradient(-45deg, #ef4444, #ef4444 2px, #fecaca 2px, #fecaca 4px)",
                }}
              />
              <span className="w-9 text-right text-[#dc2626] text-xs pr-3 select-none font-semibold">4</span>
              <div className="whitespace-pre">
                <span className="text-[#b91c1c]">  contrast: </span>
                <span className="text-[#2563eb]">42</span>
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
                <span className="text-[#7c3aed] font-medium">const </span>
                <span className="text-[#b45309]">themePreview</span>
                <span className="text-neutral-500">: </span>
                <span className="text-[#6366f1]">ThemeConfig </span>
                <span className="text-[#2563eb]">= </span>
                <span className="text-neutral-700">{"{"}</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a]" />
              <span className="w-9 text-right text-[#16a34a] text-xs pr-3 select-none font-semibold">2</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d]">  surface: </span>
                <span className="text-[#16a34a]">"sidebar-elevated"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a]" />
              <span className="w-9 text-right text-[#16a34a] text-xs pr-3 select-none font-semibold">3</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d]">  accent: </span>
                <span className="text-[#16a34a]">"{accentColor.toLowerCase()}"</span>
                <span className="text-neutral-600">,</span>
              </div>
            </div>

            <div className="flex items-center relative bg-[#f0fdf4] py-0.5">
              <div className="absolute left-0 top-0 bottom-0 w-1 bg-[#16a34a]" />
              <span className="w-9 text-right text-[#16a34a] text-xs pr-3 select-none font-semibold">4</span>
              <div className="whitespace-pre">
                <span className="text-[#15803d]">  contrast: </span>
                <span className="text-[#2563eb]">{contrast}</span>
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

        <div className="h-5 bg-neutral-50/70 border-t border-neutral-200/60 grid grid-cols-2 divide-x divide-neutral-200/60 items-center select-none">
          <div className="flex items-center justify-between px-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-neutral-200/60 text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronLeft className="w-3 h-3" />
            </Button>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-neutral-200/60 text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronRight className="w-3 h-3" />
            </Button>
          </div>
          <div className="flex items-center justify-between px-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-neutral-200/60 text-neutral-400 hover:text-neutral-700 rounded"
            >
              <ChevronLeft className="w-3 h-3" />
            </Button>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              className="h-3.5 w-3.5 p-0 hover:bg-neutral-200/60 text-neutral-400 hover:text-neutral-700 rounded"
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
                      <Check className="w-3.5 h-3.5 text-emerald-600" />
                      <span className="text-emerald-600 font-semibold">已复制</span>
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
                    <span className="bg-[#e0edff] text-[#2563eb] text-[10.5px] font-bold px-1.5 py-0.5 rounded leading-none">
                      Aa
                    </span>
                  }
                  options={THEME_LIST.map((t) => ({ value: t.id, label: t.name }))}
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
