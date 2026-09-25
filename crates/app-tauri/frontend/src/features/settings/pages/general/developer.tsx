// 「开发者」pane — user-level dev flags (`~/.config/husk/settings.toml`). These
// knobs can break the GUI (`renderer`, `gpu_acceleration`) — the recovery
// path is a single obvious file the user edits by hand, then relaunch.

import { useEffect, useState } from "react";
import { Activity, Bug, Monitor, TriangleAlert, Zap } from "@keyline-icons/react";
import { SettingsRenderer, type SettingsSection } from "@/features/settings/components";
import { getDevConfig, saveDevConfig, type DevConfig } from "@/lib/agent-ipc/sessions";
import { isLinux } from "@/lib/platform";

export function DeveloperPane() {
  const [cfg, setCfg] = useState<DevConfig>({});
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void getDevConfig()
      .then((c) => {
        setCfg(c);
        setLoaded(true);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const update = (patch: Partial<DevConfig>) => {
    const next = { ...cfg, ...patch };
    setCfg(next);
    setSaving(true);
    setError(null);
    void saveDevConfig(next)
      .catch((e) => setError(String(e)))
      .finally(() => setSaving(false));
  };

  if (error && !loaded) {
    return (
      <div className="rounded-2xl border border-red-200 bg-red-50 p-4 text-xs text-red-700">
        读取开发者配置失败:{error}
      </div>
    );
  }

  const sections: SettingsSection[] = [
    // The renderer section is Linux-only — X11 vs Wayland is a GTK /
    // WebKitGTK concept; macOS (WKWebView) and Windows (WebView2) have
    // no equivalent. GPU acceleration keys off the same env vars, so it
    // hides with the picker.
    ...(isLinux
      ? [
          {
            kind: "list" as const,
            key: "render",
            title: "渲染",
            description: <b>修改后需要重启应用</b>,
            fields: [
              {
                key: "renderer",
                label: "渲染后端",
                icon: <Monitor className="h-4 w-4 text-neutral-500" />,
                type: "select" as const,
                value: cfg.renderer ?? "x11",
                onChange: (v: string) =>
                  update({ renderer: v === "x11" ? null : (v as "x11" | "wayland") }),
                options: [
                  { value: "x11", label: "X11" },
                  { value: "wayland", label: "Wayland" },
                ],
              },
              {
                key: "gpu",
                label: "GPU 硬件加速",
                icon: <Zap className="h-4 w-4 text-neutral-500" />,
                type: "select" as const,
                value:
                  cfg.gpu_acceleration === false
                    ? "off"
                    : cfg.gpu_acceleration === true
                      ? "on"
                      : "default",
                onChange: (v: string) =>
                  update({
                    gpu_acceleration: v === "off" ? false : v === "on" ? true : null,
                  }),
                options: [
                  { value: "default", label: "默认" },
                  { value: "on", label: "启用" },
                  { value: "off", label: "关闭" },
                ],
              },
            ],
          },
        ]
      : []),
    {
      kind: "list",
      key: "monitor",
      title: "调试",
      fields: [
        {
          key: "monitor_panel",
          label: "监控面板",
          description: "按 Shift+Ctrl+P 切换 FPS/事件速率 HUD",
          icon: <Activity className="h-4 w-4 text-neutral-500" />,
          type: "switch",
          value: cfg.monitor_panel === true,
          onChange: (v) => update({ monitor_panel: v ? true : null }),
        },
        {
          key: "devtools",
          label: "DevTools 面板",
          description: "按 Ctrl+Shift+I 打开",
          icon: <Bug className="h-4 w-4 text-neutral-500" />,
          type: "switch",
          value: cfg.devtools === true,
          onChange: (v) => update({ devtools: v ? true : null }),
        },
      ],
    },
  ];

  return (
    <div className="flex flex-col gap-6">
      {!loaded ? (
        <div className="text-xs text-neutral-500">加载中…</div>
      ) : (
        <SettingsRenderer sections={sections} />
      )}
    </div>
  );
}
