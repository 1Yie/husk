import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { getAppearance, setAppearance, type AppearanceConfig } from "../../invoke/agent/sessions";
import { applyAppearance, broadcastAppearance } from "../../lib/appearance";

/** Appearance settings — just the light/dark/system mode cards. The full
 *  config still round-trips through `cfgRef` (applyAppearance needs the
 *  stored colors/fonts/contrast); only `theme_mode` is user-editable. */
export function AppearanceSettings() {
  const [themeMode, setThemeModeState] = useState<"system" | "light" | "dark">("system");
  const [currency, setCurrency] = useState<"usd" | "cny">("usd");
  // First-load flag: applying the fetched config must not echo back a
  // `set_appearance` write.
  const hydrated = useRef(false);
  const cfgRef = useRef<AppearanceConfig | null>(null);

  // Load the persisted config once, then apply it so the settings window
  // itself renders in the user's theme (it's a separate webview — no
  // shared state with the main window).
  useEffect(() => {
    void getAppearance()
      .then((cfg) => {
        cfgRef.current = cfg;
        setThemeModeState(cfg.theme_mode);
        setCurrency(cfg.currency === "cny" ? "cny" : "usd");
        applyAppearance(cfg);
        hydrated.current = true;
      })
      .catch(() => {});
  }, []);

  const setThemeMode = (v: "system" | "light" | "dark") => {
    setThemeModeState(v);
    if (cfgRef.current) {
      const cfg = { ...cfgRef.current, theme_mode: v };
      cfgRef.current = cfg;
      applyAppearance(cfg);
    }
    if (hydrated.current) {
      void setAppearance({ theme_mode: v })
        .then(() => broadcastAppearance())
        .catch((e) => console.error("save appearance:", e));
    }
  };

  const setCurrencyMode = (v: "usd" | "cny") => {
    setCurrency(v);
    if (cfgRef.current) cfgRef.current = { ...cfgRef.current, currency: v };
    if (hydrated.current) {
      void setAppearance({ currency: v })
        .then(() => broadcastAppearance())
        .catch((e) => console.error("save appearance:", e));
    }
  };

  return (
    <div className="flex flex-col gap-8 w-full">
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-0.5">
          <span className="text-sm font-semibold text-neutral-900">
            外观模式
          </span>
          <span className="text-xs text-neutral-500">
            选择应用的明暗风格，可固定浅色、深色或跟随系统自适应
          </span>
        </div>

        <div className="grid grid-cols-3 gap-3">
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
                  ? "border-2 border-accent ring-2 ring-accent/25 shadow-xs"
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
                themeMode === "system" ? "text-neutral-900" : "text-neutral-600"
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
                  ? "border-2 border-accent ring-2 ring-accent/25 shadow-xs"
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
                themeMode === "light" ? "text-neutral-900" : "text-neutral-600"
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
                  ? "border-2 border-accent ring-2 ring-accent/25 shadow-xs"
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
                themeMode === "dark" ? "text-neutral-900" : "text-neutral-600"
              )}
            >
              深色
            </span>
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-0.5">
          <span className="text-sm font-semibold text-neutral-900">
            货币单位
          </span>
          <span className="text-xs text-neutral-500">
            标题栏会话花费的显示符号，仅切换标识不做汇率换算
          </span>
        </div>

        <div className="flex gap-2">
          {(
            [
              { v: "usd", label: "$ 美元" },
              { v: "cny", label: "¥ 人民币" },
            ] as const
          ).map(({ v, label }) => (
            <button
              key={v}
              type="button"
              onClick={() => setCurrencyMode(v)}
              className={cn(
                "h-8 px-4 rounded-lg text-xs font-medium transition-colors cursor-pointer select-none",
                currency === v
                  ? "border-2 border-accent text-neutral-900 bg-accent/5"
                  : "border border-[color-mix(in_srgb,var(--husk-n200)_90%,transparent)] text-neutral-600 hover:border-neutral-300"
              )}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
