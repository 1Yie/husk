import { useEffect, useState } from "react";
import {
  SlidersHorizontal,
  Settings as SettingsIcon,
  TriangleAlert,
  Zap,
  SquarePen,
  ShieldCheck,
  Sparkles,
  Palette,
} from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { PopupWindow } from "@/layout/popup-window";
import { cn } from "@/lib/utils";
import { getDefaultPrefs, setDefaultPrefs } from "../../invoke/agent/sessions";
import { AppearanceSettings } from "./appearance";
import {
  SettingsRenderer,
  type SettingsCardOption,
} from "@/components/settings";

const PERMISSION_OPTIONS: SettingsCardOption[] = [
  {
    value: "auto",
    label: "自动",
    badge: (
      <Badge
        variant="secondary"
        className="text-[10px] px-1.5 py-0 font-medium h-4"
      >
        推荐
      </Badge>
    ),
    description: "自动执行文件修改与常规命令，仅在遇到高危指令（如删除、外网访问）时弹窗确认。",
    icon: <Zap className="h-4 w-4 text-amber-500 shrink-0" />,
  },
  {
    value: "bypassPermissions",
    label: "全部权限",
    badge: (
      <Badge
        variant="default"
        className="text-[10px] px-1.5 py-0 font-medium h-4 bg-emerald-600 hover:bg-emerald-600 text-white"
      >
        完全信任
      </Badge>
    ),
    description: "全自动自主执行，跳过所有修改和命令的确认弹窗，实现最高执行效率。",
    icon: <TriangleAlert className="h-4 w-4 text-emerald-500 shrink-0" />,
  },
  {
    value: "acceptEdits",
    label: "仅接受编辑",
    description: "自动允许代码与文件修改，但在执行终端命令前仍需手动批准。",
    icon: <SquarePen className="h-4 w-4 text-blue-500 shrink-0" />,
  },
  {
    value: "default",
    label: "每次确认",
    description: "最严格的安全模式，凡是涉及修改文件或运行终端命令前均需手动批准。",
    icon: <ShieldCheck className="h-4 w-4 text-neutral-500 shrink-0" />,
  },
];

const THINKING_OPTIONS = [
  { value: "off", label: "关闭思考", desc: "不使用额外推理过程，响应最快" },
  { value: "low", label: "轻度思考", desc: "进行快速简短的分析" },
  { value: "medium", label: "中等思考", desc: "平衡速度与分析质量" },
  { value: "high", label: "深度思考", desc: "深层推演架构与复杂逻辑" },
  { value: "max", label: "最大推演", desc: "全速深度推理，解决高难任务" },
];

export function SettingsWindow() {
  const [activeTab, setActiveTab] = useState<"appearance" | "preferences">("appearance");
  const [permission, setPermission] = useState("auto");
  const [thinking, setThinking] = useState("medium");

  const loadData = async () => {
    try {
      const prefs = await getDefaultPrefs();
      if (prefs.permission_mode) {
        setPermission(prefs.permission_mode);
      }
      if (prefs.thinking_level) {
        setThinking(prefs.thinking_level);
      }
    } catch (e) {
      console.error("load settings error:", e);
    }
  };

  useEffect(() => {
    void loadData();
  }, []);

  const handlePermissionChange = async (val: string) => {
    setPermission(val);
    try {
      await setDefaultPrefs({ permission_mode: val });
    } catch (e) {
      console.error("Failed to update permission mode:", e);
    }
  };

  const handleThinkingChange = async (val: string) => {
    setThinking(val);
    try {
      await setDefaultPrefs({ thinking_level: val });
    } catch (e) {
      console.error("Failed to update thinking level:", e);
    }
  };

  return (
    <PopupWindow
      maximize={false}
      title={
        <div data-drag data-tauri-drag-region className="flex items-center gap-2">
          <SettingsIcon className="h-4 w-4 text-neutral-600 shrink-0" />
          <span className="text-[13px] font-semibold text-neutral-800 tracking-tight">
            设置
          </span>
        </div>
      }
    >
      <div className="flex flex-1 overflow-hidden h-[calc(100vh-36px)]">
        <aside
          data-tauri-drag-region="deep"
          className="w-56 flex-none bg-neutral-100/60 border-r border-neutral-200/80 flex flex-col p-3 select-none justify-between"
        >
          {/* Nav items are h-10, but the Button base radius (rounded-lg = 14px)
              is sized for h-9 controls: at h-10 it lands at 70% of half-height
              and reads visibly tighter than the h-8 session rows in the main
              sidebar (88%), despite the identical white active-card treatment.
              rounded-xl (18px → 90%) puts the two sidebars on the same footing. */}
          <div data-nodrag data-tauri-drag-region="false" className="flex flex-col gap-1.5">
            <Button
              variant="ghost"
              onClick={() => setActiveTab("appearance")}
              className={cn(
                "w-full justify-start gap-2.5 h-10 rounded-xl px-3 text-[13px] transition-all",
                activeTab === "appearance"
                  ? "bg-white text-neutral-900 font-semibold shadow-xs border border-neutral-200/80"
                  : "text-neutral-600 hover:text-neutral-900 hover:bg-neutral-200/50 font-medium"
              )}
            >
              <Palette className="h-4 w-4 text-neutral-900 shrink-0" />
              <span>外观</span>
            </Button>

            <Button
              variant="ghost"
              onClick={() => setActiveTab("preferences")}
              className={cn(
                "w-full justify-start gap-2.5 h-10 rounded-xl px-3 text-[13px] transition-all",
                activeTab === "preferences"
                  ? "bg-white text-neutral-900 font-semibold shadow-xs border border-neutral-200/80"
                  : "text-neutral-600 hover:text-neutral-900 hover:bg-neutral-200/50 font-medium"
              )}
            >
              <SlidersHorizontal className="h-4 w-4 text-neutral-900 shrink-0" />
              <span>个性化</span>
            </Button>
          </div>

          <div className="p-1">
            <span className="text-[11px] text-neutral-400 font-mono block text-center">
              Husk v0.1.0
            </span>
          </div>
        </aside>

        <div className="flex-1 min-h-0 overflow-y-auto bg-white">
          <div className="p-8 flex flex-col justify-between min-h-[calc(100vh-36px)]">
            {activeTab === "appearance" ? (
              <AppearanceSettings />
            ) : (
              <div className="flex flex-col gap-6 max-w-3xl">
                <div>
                  <h2 className="text-base font-semibold text-neutral-900">
                    个性化偏好
                  </h2>
                </div>

                <SettingsRenderer
                  sections={[
                    {
                      kind: "cards",
                      key: "permission",
                      title: "默认权限模式",
                      value: permission,
                      onChange: (v) => void handlePermissionChange(v),
                      options: PERMISSION_OPTIONS,
                    },
                    {
                      kind: "list",
                      key: "model",
                      title: "模型偏好",
                      fields: [
                        {
                          key: "thinking",
                          type: "select",
                          label: "默认思考深度",
                          description: "配置新会话启动时的默认推理思考深度",
                          icon: <Sparkles className="h-4 w-4 text-neutral-500" />,
                          value: thinking,
                          onChange: (v) => void handleThinkingChange(v),
                          placeholder: "选择思考深度",
                          options: THINKING_OPTIONS.map((t) => ({
                            value: t.value,
                            label: t.label,
                            description: t.desc,
                          })),
                        },
                      ],
                    },
                  ]}
                />
              </div>
            )}
          </div>
        </div>
      </div>
    </PopupWindow>
  );
}
