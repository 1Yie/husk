import { useEffect, useRef, useState } from "react";
import { Coins, Monitor, Moon, Sun } from "@keyline-icons/react";
import { Skeleton } from "@/components/ui/skeleton";
import { KvList, KvListContent } from "@/components/ui/kv-list";
import { Button } from "@/components/ui/button";
import {
  EmptyState,
  SettingRow,
  SettingSelect,
  SettingsCards,
  SettingsSection,
} from "@/features/settings/components";
import { getAppearance, setAppearance, type AppearanceConfig } from "@/lib/agent-ipc/sessions";
import { applyAppearance, broadcastAppearance } from "@/lib/appearance";
import { cn } from "@/lib/utils";
import { toast } from "sonner";

type ThemeMode = "system" | "light" | "dark";

const THEMES = [
  { value: "system", label: "跟随系统", Icon: Monitor },
  { value: "light", label: "浅色", Icon: Sun },
  { value: "dark", label: "深色", Icon: Moon },
] as const;

/** Literal zinc colours depict each theme independently of the current one. */
function PreviewWindow({ dark }: { dark: boolean }) {
  return (
    <div className={cn("flex h-full min-w-0 flex-1 p-2.5 pb-0", dark ? "bg-zinc-800" : "bg-zinc-100")}>
      <div className={cn(
        "flex w-full overflow-hidden rounded-t-lg border border-b-0 shadow-sm",
        dark ? "border-zinc-600/50 bg-zinc-900" : "border-zinc-300/70 bg-[#ffffff]",
      )}>
        <div className={cn("flex w-[28%] flex-col gap-1.5 border-r p-2", dark ? "border-zinc-700 bg-zinc-800" : "border-zinc-200 bg-zinc-50")}>
          <span className={cn("mb-1 h-1 w-3/4 rounded-full", dark ? "bg-zinc-500" : "bg-zinc-300")} />
          <span className={cn("h-1 w-full rounded-full", dark ? "bg-zinc-600" : "bg-zinc-200")} />
          <span className={cn("h-1 w-2/3 rounded-full", dark ? "bg-zinc-600" : "bg-zinc-200")} />
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-2 p-2.5">
          <span className={cn("h-1 w-1/2 rounded-full", dark ? "bg-zinc-500" : "bg-zinc-300")} />
          <span className={cn("h-3 w-3/4 self-end rounded-md", dark ? "bg-zinc-700" : "bg-zinc-100")} />
          <div className="flex flex-col gap-1">
            <span className={cn("h-1 w-full rounded-full", dark ? "bg-zinc-700" : "bg-zinc-200")} />
            <span className={cn("h-1 w-2/3 rounded-full", dark ? "bg-zinc-700" : "bg-zinc-200")} />
          </div>
          <span className={cn("mt-auto h-4 w-full rounded-md border", dark ? "border-zinc-700 bg-zinc-800" : "border-zinc-200 bg-zinc-50")} />
        </div>
      </div>
    </div>
  );
}

function ThemePreview({ mode }: { mode: ThemeMode }) {
  return (
    <div aria-hidden="true" className="flex h-28 w-full overflow-hidden rounded-xl border border-hairline">
      {mode === "system" ? (
        <>
          <PreviewWindow dark={false} />
          <PreviewWindow dark />
        </>
      ) : <PreviewWindow dark={mode === "dark"} />}
    </div>
  );
}

export function AppearanceSettings() {
  const [themeMode, setThemeModeState] = useState<ThemeMode>("system");
  const [currency, setCurrency] = useState<"usd" | "cny">("usd");
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState("");
  const [loadAttempt, setLoadAttempt] = useState(0);
  // Preserve the complete config when applying a theme preview.
  const cfgRef = useRef<AppearanceConfig | null>(null);

  useEffect(() => {
    let alive = true;
    setLoadError("");
    void getAppearance()
      .then((cfg) => {
        if (!alive) return;
        cfgRef.current = cfg;
        setThemeModeState(cfg.theme_mode);
        setCurrency(cfg.currency === "cny" ? "cny" : "usd");
        applyAppearance(cfg);
        setLoaded(true);
      })
      .catch((e) => { if (alive) setLoadError(String(e)); });
    return () => { alive = false; };
  }, [loadAttempt]);

  const setThemeMode = (v: ThemeMode) => {
    if (!cfgRef.current) return;
    setThemeModeState(v);
    const cfg = { ...cfgRef.current, theme_mode: v };
    cfgRef.current = cfg;
    applyAppearance(cfg);
    void setAppearance({ theme_mode: v })
      .then(() => broadcastAppearance())
      .catch((e) => toast.error("保存外观失败", { description: String(e) }));
  };

  const setCurrencyMode = (v: "usd" | "cny") => {
    if (!cfgRef.current) return;
    setCurrency(v);
    cfgRef.current = { ...cfgRef.current, currency: v };
    void setAppearance({ currency: v })
      .then(() => broadcastAppearance())
      .catch((e) => toast.error("保存货币单位失败", { description: String(e) }));
  };

  if (loadError) {
    return (
      <KvList>
        <EmptyState
          icon={<Monitor className="h-4 w-4" />}
          title="无法读取外观设置"
          description={<span className="break-all">{loadError}</span>}
          action={<Button size="sm" variant="outline" onClick={() => setLoadAttempt((n) => n + 1)}>重试</Button>}
        />
      </KvList>
    );
  }

  return (
    <div className="flex w-full min-w-0 flex-col gap-8">
      <SettingsSection title="主题">
        {!loaded ? (
          <div className="grid animate-pulse grid-cols-[repeat(auto-fit,minmax(min(100%,160px),1fr))] gap-3" aria-busy="true">
            {THEMES.map(({ value }) => <Skeleton key={value} className="h-[178px] rounded-2xl" />)}
          </div>
        ) : (
          <SettingsCards
            value={themeMode}
            onChange={(v) => setThemeMode(v as ThemeMode)}
            className="grid-cols-[repeat(auto-fit,minmax(min(100%,160px),1fr))]"
            options={THEMES.map(({ value, label, Icon }) => ({
              value,
              label,
              icon: <Icon className="h-4 w-4" />,
              preview: <ThemePreview mode={value} />,
            }))}
          />
        )}
      </SettingsSection>

      <SettingsSection title="显示">
        <KvList>
          <KvListContent>
            <SettingRow
              title="货币单位"
              description="仅更改金额符号，不进行汇率换算"
              icon={<Coins className="h-4 w-4" />}
              wideControl
            >
              {!loaded ? <Skeleton className="h-8 w-full rounded-xl" /> : (
                <SettingSelect
                  value={currency}
                  onChange={(v) => setCurrencyMode(v as "usd" | "cny")}
                  ariaLabel="货币单位"
                  options={[
                    { value: "usd", label: "$ 美元" },
                    { value: "cny", label: "¥ 人民币" },
                  ]}
                />
              )}
            </SettingRow>
          </KvListContent>
        </KvList>
      </SettingsSection>
    </div>
  );
}
