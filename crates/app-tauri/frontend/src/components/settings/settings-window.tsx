import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Sliders,
  Minus,
  X,
  Settings as SettingsIcon,
  ShieldAlert,
  Zap,
  FileEdit,
  ShieldCheck,
  Brain,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { isMac } from "@/lib/platform";
import { Label } from "@/components/ui/label";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { getDefaultPrefs, setDefaultPrefs } from "../../invoke/agent/sessions";

const win = getCurrentWindow();

interface PermissionOption {
  value: string;
  label: string;
  badge?: string;
  desc: string;
  icon: React.ReactNode;
}

const PERMISSION_OPTIONS: PermissionOption[] = [
  {
    value: "auto",
    label: "自动",
    badge: "推荐",
    desc: "自动执行文件修改与常规命令，仅在遇到高危指令（如删除、外网访问）时弹窗确认。",
    icon: <Zap className="h-4 w-4 text-amber-500 shrink-0" />,
  },
  {
    value: "bypassPermissions",
    label: "全部权限",
    badge: "完全信任",
    desc: "全自动自主执行，跳过所有修改和命令的确认弹窗，实现最高执行效率。",
    icon: <ShieldAlert className="h-4 w-4 text-emerald-500 shrink-0" />,
  },
  {
    value: "acceptEdits",
    label: "仅接受编辑",
    desc: "自动允许代码与文件修改，但在执行终端命令前仍需手动批准。",
    icon: <FileEdit className="h-4 w-4 text-blue-500 shrink-0" />,
  },
  {
    value: "default",
    label: "每次确认",
    desc: "最严格的安全模式，凡是涉及修改文件或运行终端命令前均需手动批准。",
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

  const drag = (e: React.MouseEvent) => {
    const el = e.target as HTMLElement;
    if (e.button === 0 && el.closest("[data-drag]") && !el.closest("[data-nodrag]")) {
      void win.startDragging();
    }
  };

  return (
    <div className="flex flex-col h-screen w-screen bg-neutral-100/50 select-none overflow-hidden font-sans">
      {/* Frameless Draggable TitleBar */}
      <div
        onMouseDown={drag}
        data-drag
        data-tauri-drag-region
        className="h-9 flex-none flex items-center justify-between px-3 border-b border-neutral-200/80 bg-white select-none cursor-default"
      >
        {/* macOS: leave room for native traffic-light buttons */}
        {isMac && <div data-drag data-tauri-drag-region className="w-[78px] shrink-0" />}

        <div data-drag data-tauri-drag-region className="flex items-center gap-2">
          <SettingsIcon className="h-4 w-4 text-neutral-600 shrink-0" />
          <span className="text-[13px] font-semibold text-neutral-800 tracking-tight">
            设置
          </span>
        </div>

        <div data-drag data-tauri-drag-region className="flex-1 h-full" />

        {/* Window controls — hidden on macOS (native traffic lights) */}
        {!isMac && (
          <div data-nodrag className="flex items-center gap-0.5 shrink-0">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => void win.minimize()}
              className="text-neutral-500 hover:bg-neutral-200/70 hover:text-neutral-900"
            >
              <Minus className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => void win.close()}
              className="text-neutral-500 hover:bg-red-500 hover:text-white"
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        )}
      </div>

      {/* Main Dual-Column Body: Left Options, Right Settings */}
      <div className="flex flex-1 overflow-hidden h-[calc(100vh-36px)]">
        {/* Left Column: Focused "个性化" */}
        <aside className="w-56 flex-none bg-neutral-100/60 border-r border-neutral-200/80 flex flex-col p-3 select-none justify-between">
          <div className="flex flex-col gap-1.5">
            <Button
              variant="secondary"
              className="w-full justify-start gap-2.5 h-10 px-3 bg-white text-neutral-900 font-semibold shadow-xs border border-neutral-200/80 text-[13px]"
            >
              <Sliders className="h-4 w-4 text-neutral-900 shrink-0" />
              <span>个性化</span>
            </Button>
          </div>

          <div className="p-1">
            <span className="text-[11px] text-neutral-400 font-mono block text-center">
              agent-rs v0.1.0
            </span>
          </div>
        </aside>

        {/* Right Column: Personalized Settings Content with shadcn ScrollArea */}
        <ScrollArea className="flex-1 h-full bg-white">
          <div className="p-8 flex flex-col justify-between min-h-[calc(100vh-36px)]">
            <div className="flex flex-col gap-6 max-w-3xl">
              <div>
                <h2 className="text-base font-semibold text-neutral-900">
                  个性化偏好
                </h2>
                <p className="text-xs text-neutral-500 mt-1">
                  配置新建会话的默认权限与思考深度。这些偏好仅在创建新会话时生效，
                  不会更改已开始会话的运行状态；如需调整当前会话，请使用输入框旁的控制项。
                </p>
              </div>

              {/* Permission Mode Cards */}
              <div className="flex flex-col gap-3">
                <Label className="text-sm font-medium text-neutral-800">
                  默认权限模式
                </Label>

                <div className="grid gap-2.5">
                  {PERMISSION_OPTIONS.map((opt) => {
                    const isSelected = permission === opt.value;
                    return (
                      <Card
                        key={opt.value}
                        onClick={() => void handlePermissionChange(opt.value)}
                        className={cn(
                          "p-3.5 cursor-pointer transition-all duration-150 shadow-none",
                          isSelected
                            ? "border-neutral-900 bg-neutral-50/80 ring-1 ring-neutral-900"
                            : "border-neutral-200/90 bg-white hover:border-neutral-300 hover:bg-neutral-50/40"
                        )}
                      >
                        <div className="flex items-start gap-3.5">
                          <div className="mt-0.5">{opt.icon}</div>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="text-sm font-semibold text-neutral-900">
                                {opt.label}
                              </span>
                              {opt.badge && (
                                <Badge
                                  variant={opt.value === "bypassPermissions" ? "default" : "secondary"}
                                  className={cn(
                                    "text-[10px] px-1.5 py-0 font-medium h-4",
                                    opt.value === "bypassPermissions" && "bg-emerald-600 hover:bg-emerald-600 text-white"
                                  )}
                                >
                                  {opt.badge}
                                </Badge>
                              )}
                            </div>
                            <p className="text-xs text-neutral-500 mt-1 leading-relaxed">
                              {opt.desc}
                            </p>
                          </div>
                          <div className="pt-0.5 shrink-0">
                            <div
                              className={cn(
                                "w-4 h-4 rounded-full border flex items-center justify-center transition-colors",
                                isSelected
                                  ? "border-neutral-900 bg-neutral-900 text-white"
                                  : "border-neutral-300 bg-white"
                              )}
                            >
                              {isSelected && <div className="w-1.5 h-1.5 rounded-full bg-white" />}
                            </div>
                          </div>
                        </div>
                      </Card>
                    );
                  })}
                </div>
              </div>

              <Separator className="my-1" />

              {/* Thinking Level Selection using shadcn Select */}
              <div className="flex flex-col gap-2.5">
                <div className="flex items-center gap-2">
                  <Brain className="h-4 w-4 text-neutral-500" />
                  <Label htmlFor="thinking-level" className="text-sm font-medium text-neutral-800">
                    默认思考深度
                  </Label>
                </div>

                <Select value={thinking} onValueChange={(val) => void handleThinkingChange(val)}>
                  <SelectTrigger
                    id="thinking-level"
                    className="h-10 w-full bg-white border-neutral-200 text-sm focus:ring-1 focus:ring-neutral-900"
                  >
                    <SelectValue placeholder="选择思考深度" />
                  </SelectTrigger>
                  <SelectContent className="bg-white">
                    {THINKING_OPTIONS.map((t) => (
                      <SelectItem key={t.value} value={t.value} className="text-sm py-2 cursor-pointer">
                        <span className="font-medium text-neutral-900">{t.label}</span>
                        <span className="text-neutral-500 text-xs ml-2">— {t.desc}</span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
