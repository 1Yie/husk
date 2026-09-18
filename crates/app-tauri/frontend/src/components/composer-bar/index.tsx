// Composer — floating input bar with config.toml-based model selection & Orb-driven send button.

import { useEffect, useState } from "react";
import {
  Plus,
  ChevronDown,
  ArrowUp,
  Square,
  FileText,
  FileTerminal,
  FileDiff,
  FolderOpen,
  Search,
  ListTodo,
  Wrench,
  ShieldCheck,
  Check,
  Brain,
} from "lucide-react";
import { Orb } from "../agent-orb";
import * as agent from "../../invoke/agent";
import type { SessionView } from "../../hooks/useAgent";
import { pendingApprovalOf } from "../../hooks/useAgent";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
  DropdownMenuGroup,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

const PERMISSION_MODES = [
  { value: "default",            label: "默认" },
  { value: "acceptEdits",        label: "接受编辑" },
  { value: "auto",               label: "自动" },
  { value: "dontAsk",            label: "不询问" },
  { value: "bypassPermissions",  label: "跳过权限" },
] as const;

const THINKING_LEVELS = [
  { value: "off",     label: "关闭思考", desc: "不使用推理计算" },
  { value: "minimal", label: "极低强度", desc: "最小推理深度" },
  { value: "low",     label: "低强度",   desc: "轻度思考分析" },
  { value: "medium",  label: "中等思考", desc: "标准平衡思考" },
  { value: "high",    label: "高强度",   desc: "深度推演与设计" },
  { value: "xhigh",   label: "超高强度", desc: "超深层推演" },
  { value: "max",     label: "最大思考", desc: "最大算力深度推演" },
] as const;

/** Tool name → icon for the approval strip. */
function toolIcon(name: string) {
  const cls = "h-3.5 w-3.5";
  if (name === "bash" || name === "pty" || name === "shell")
    return <FileTerminal className={cls} />;
  if (name === "fuzzy_patch" || name === "apply_patch")
    return <FileDiff className={cls} />;
  if (name === "write_file" || name === "fs_patch")
    return <FileText className={cls} />;
  if (name === "list_dir" || name === "read_dir")
    return <FolderOpen className={cls} />;
  if (name === "grep" || name === "smart_read" || name === "fs_read")
    return <Search className={cls} />;
  if (name === "todo")
    return <ListTodo className={cls} />;
  return <Wrench className={cls} />;
}

export function ComposerBar({ view }: { view: SessionView }) {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<string>("default");

  const [activeModel, setActiveModel] = useState<string>("");
  const [activeProvider, setActiveProvider] = useState<string>("");
  const [models, setModels] = useState<agent.ModelItem[]>([]);
  const [activeThinkingLevel, setActiveThinkingLevel] = useState<string>("off");

  // Load models directly from config.toml via kernel IPC
  const fetchModels = async () => {
    try {
      const info = await agent.getModelInfo();
      if (info) {
        if (info.active_model) setActiveModel(info.active_model);
        if (info.active_provider) setActiveProvider(info.active_provider);
        if (info.active_thinking_level) setActiveThinkingLevel(info.active_thinking_level);
        if (info.models && Array.isArray(info.models)) {
          setModels(info.models);
        }
      }
    } catch (err) {
      console.warn("Could not load model_info from config:", err);
    }
  };

  useEffect(() => {
    void fetchModels();
  }, []);

  const currentModelObj =
    models.find((m) => m.model === activeModel && m.provider === activeProvider) ||
    models.find((m) => m.model === activeModel);

  const hasReasoning = Boolean(
    currentModelObj?.reasoning ||
      (currentModelObj?.available_levels && currentModelObj.available_levels.length > 0)
  );

  const availableLevelValues =
    currentModelObj?.available_levels && currentModelObj.available_levels.length > 0
      ? currentModelObj.available_levels
      : hasReasoning
      ? ["off", "low", "medium", "high", "max"]
      : [];

  const effectiveThinkingLevels = THINKING_LEVELS.filter((lvl) =>
    availableLevelValues.includes(lvl.value)
  );

  const currentThinkingObj = THINKING_LEVELS.find((l) => l.value === activeThinkingLevel);
  const currentThinkingLabel = currentThinkingObj?.label || activeThinkingLevel;

  const handleSelectModel = async (provider: string, model: string) => {
    setActiveModel(model);
    setActiveProvider(provider);
    try {
      await agent.setModel(provider, model);
      const target = models.find((m) => m.model === model && m.provider === provider);
      if (target?.available_levels && target.available_levels.length > 0) {
        if (!target.available_levels.includes(activeThinkingLevel)) {
          const nextLevel = target.available_levels.includes("medium")
            ? "medium"
            : target.available_levels.find((l) => l !== "off") || "off";
          setActiveThinkingLevel(nextLevel);
          await agent.setThinkingLevel(nextLevel);
        }
      }
    } catch (e) {
      console.error("setModel failed:", e);
    }
  };

  const handleSelectThinkingLevel = async (level: string) => {
    setActiveThinkingLevel(level);
    try {
      await agent.setThinkingLevel(level);
    } catch (e) {
      console.error("setThinkingLevel failed:", e);
    }
  };

  const streaming = view.streaming;
  const pendingRaw = pendingApprovalOf(view);
  const [decidedId, setDecidedId] = useState<number | null>(null);

  useEffect(() => {
    if (pendingRaw && decidedId !== null && decidedId !== pendingRaw.requestId) {
      setDecidedId(null);
    }
  }, [pendingRaw?.requestId, decidedId]);

  const pending = pendingRaw && pendingRaw.requestId !== decidedId ? pendingRaw : null;

  const submit = async () => {
    const t = text.trim();
    if (!t) return;
    try {
      if (streaming) await agent.steer(t);
      else await agent.sendPrompt(t);
      setText("");
    } catch (e) {
      console.error("agent_cmd failed:", e);
    }
  };

  const cancel = async () => {
    try {
      await agent.cancelTurn();
    } catch {
      /* no active turn */
    }
  };

  const decide = async (approved: boolean) => {
    if (!pending) return;
    const reqId = pending.requestId;
    setDecidedId(reqId);
    try {
      await agent.decideTool(reqId, approved);
    } catch (e) {
      console.error("decideTool failed:", e);
      setDecidedId(null);
    }
  };

  const switchMode = async (m: string) => {
    setMode(m);
    try {
      await agent.setPermissionMode(m);
    } catch (e) {
      console.error("setPermissionMode failed:", e);
    }
  };

  const modeLabel =
    PERMISSION_MODES.find((m) => m.value === mode)?.label ?? "默认";

  return (
    <TooltipProvider delayDuration={300}>
      <div className="w-full flex justify-center px-4 pb-6 select-none">
        {pending ? (
          /* Approval pending — grey outer card + top strip. */
          <div className="max-w-3xl w-full bg-[#f4f4f6] dark:bg-neutral-900 rounded-[24px] pt-2.5 flex flex-col gap-2 transition-all">
            <div className="flex items-center gap-2 px-3 pt-0.5 text-xs text-neutral-600 dark:text-neutral-400 font-medium select-none">
              <span className="text-neutral-500 shrink-0 flex items-center">
                {toolIcon(pending.toolName)}
              </span>
              <span className="shrink-0 text-neutral-800 dark:text-neutral-200 font-medium">
                允许执行
              </span>
              <span className="font-mono text-[11px] text-neutral-600 dark:text-neutral-300 truncate min-w-0 flex-1 bg-black/[0.04] dark:bg-white/[0.06] border border-black/[0.06] dark:border-white/[0.08] px-2 py-0.5 rounded">
                {pending.args || pending.toolName}
              </span>
              <Button
                size="sm"
                className="shrink-0 h-6 px-2.5 text-[11px] bg-emerald-600 hover:bg-emerald-700 text-white cursor-pointer"
                onClick={() => void decide(true)}
              >
                允许
              </Button>
              <Button
                size="sm"
                variant="outline"
                className="shrink-0 h-6 px-2.5 text-[11px] border-neutral-300 text-neutral-600 hover:bg-neutral-200 cursor-pointer"
                onClick={() => void decide(false)}
              >
                拒绝
              </Button>
            </div>

            {/* Inner white input card */}
            <div className="bg-white dark:bg-neutral-950 border border-[#e4e4e7] dark:border-neutral-800 rounded-[18px] p-3 flex flex-col gap-2 shadow-[0_2px_8px_rgba(0,0,0,0.02)]">
              <Textarea
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void submit();
                  }
                }}
                placeholder={streaming ? "插入指示引导生成 (Steer)…" : "输入消息或要求后续变更…"}
                rows={1}
                className="min-h-[42px] max-h-[180px] resize-none border-0 shadow-none focus-visible:ring-0 px-2 pt-1 text-[14px] leading-relaxed"
              />

              <ComposerToolbar
                modeLabel={modeLabel}
                mode={mode}
                switchMode={switchMode}
                activeModel={activeModel}
                activeProvider={activeProvider}
                models={models}
                onSelectModel={handleSelectModel}
                hasReasoning={hasReasoning}
                activeThinkingLevel={activeThinkingLevel}
                currentThinkingLabel={currentThinkingLabel}
                effectiveThinkingLevels={effectiveThinkingLevels}
                onSelectThinkingLevel={handleSelectThinkingLevel}
                streaming={streaming}
                text={text}
                submit={submit}
                cancel={cancel}
              />
            </div>
          </div>
        ) : (
          /* Normal — single white card. */
          <div className="max-w-3xl w-full bg-white dark:bg-neutral-950 border border-[#e4e4e7] dark:border-neutral-800 rounded-[22px] shadow-[0_4px_20px_rgba(0,0,0,0.04)] hover:border-neutral-300 dark:hover:border-neutral-700 transition-all p-3 flex flex-col gap-2">
            <Textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void submit();
                }
              }}
              placeholder={streaming ? "插入指示引导生成 (Steer)…" : "输入消息或要求后续变更…"}
              rows={1}
              className="min-h-[42px] max-h-[180px] resize-none border-0 shadow-none focus-visible:ring-0 px-2 pt-1 text-[14px] leading-relaxed"
            />

            <ComposerToolbar
              modeLabel={modeLabel}
              mode={mode}
              switchMode={switchMode}
              activeModel={activeModel}
              activeProvider={activeProvider}
              models={models}
              onSelectModel={handleSelectModel}
              hasReasoning={hasReasoning}
              activeThinkingLevel={activeThinkingLevel}
              currentThinkingLabel={currentThinkingLabel}
              effectiveThinkingLevels={effectiveThinkingLevels}
              onSelectThinkingLevel={handleSelectThinkingLevel}
              streaming={streaming}
              text={text}
              submit={submit}
              cancel={cancel}
            />
          </div>
        )}
      </div>
    </TooltipProvider>
  );
}

function ComposerToolbar({
  modeLabel,
  mode,
  switchMode,
  activeModel,
  activeProvider,
  models,
  onSelectModel,
  hasReasoning,
  activeThinkingLevel,
  currentThinkingLabel,
  effectiveThinkingLevels,
  onSelectThinkingLevel,
  streaming,
  text,
  submit,
  cancel,
}: {
  modeLabel: string;
  mode: string;
  switchMode: (m: string) => Promise<void>;
  activeModel: string;
  activeProvider: string;
  models: agent.ModelItem[];
  onSelectModel: (provider: string, model: string) => Promise<void>;
  hasReasoning: boolean;
  activeThinkingLevel: string;
  currentThinkingLabel: string;
  effectiveThinkingLevels: readonly { value: string; label: string; desc: string }[];
  onSelectThinkingLevel: (level: string) => Promise<void>;
  streaming: boolean;
  text: string;
  submit: () => Promise<void>;
  cancel: () => Promise<void>;
}) {
  const canSubmit = text.trim().length > 0;

  return (
    <div className="flex items-center justify-between pt-1">
      {/* Left toolbar: attachment + permission-mode dropdown */}
      <div className="flex items-center gap-1.5">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              className="w-7 h-7 rounded-full text-neutral-500 hover:text-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-800"
            >
              <Plus className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="top">添加附件</TooltipContent>
        </Tooltip>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              title="权限"
              className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 dark:text-neutral-300 bg-neutral-100/80 dark:bg-neutral-800/80 hover:bg-neutral-200/70 dark:hover:bg-neutral-700/70 rounded-lg transition-colors border border-neutral-200/50 dark:border-neutral-700/50 select-none cursor-pointer"
            >
              <ShieldCheck className="h-3.5 w-3.5 text-neutral-500" />
              <span>{modeLabel}</span>
              <ChevronDown className="h-3 w-3 text-neutral-400" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" className="w-36">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              权限
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {PERMISSION_MODES.map((m) => {
                const active = m.value === mode;
                return (
                  <DropdownMenuItem
                    key={m.value}
                    onClick={() => void switchMode(m.value)}
                    className="flex items-center justify-between text-xs py-2 cursor-pointer"
                  >
                    <span className="font-medium text-neutral-800 dark:text-neutral-200">
                      {m.label}
                    </span>
                    {active && <Check className="h-3.5 w-3.5 text-emerald-600 shrink-0" />}
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Thinking Level Selector (only shown when active model supports reasoning) */}
        {hasReasoning && effectiveThinkingLevels.length > 0 && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                title={`思考推理强度: ${currentThinkingLabel}`}
                className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 dark:text-neutral-300 bg-neutral-100/80 dark:bg-neutral-800/80 hover:bg-neutral-200/70 dark:hover:bg-neutral-700/70 rounded-lg transition-colors border border-neutral-200/50 dark:border-neutral-700/50 select-none cursor-pointer"
              >
                <Brain
                  className={cn(
                    "h-3.5 w-3.5 shrink-0",
                    activeThinkingLevel === "off"
                      ? "text-neutral-400 dark:text-neutral-500"
                      : "text-purple-600 dark:text-purple-400"
                  )}
                />
                <span className="truncate max-w-[90px]">
                  {activeThinkingLevel === "off" ? "思考关闭" : `思考: ${currentThinkingLabel}`}
                </span>
                <ChevronDown className="h-3 w-3 text-neutral-400 shrink-0" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="w-48">
              <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
                思考推理强度
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                {effectiveThinkingLevels.map((lvl) => {
                  const active = lvl.value === activeThinkingLevel;
                  return (
                    <DropdownMenuItem
                      key={lvl.value}
                      onClick={() => void onSelectThinkingLevel(lvl.value)}
                      className="flex items-center justify-between text-xs py-2 cursor-pointer"
                    >
                      <div className="flex flex-col gap-0.5 min-w-0 pr-2">
                        <span className="font-medium text-neutral-800 dark:text-neutral-200">
                          {lvl.label}
                        </span>
                        <span className="text-[10.5px] text-neutral-400 dark:text-neutral-500 truncate">
                          {lvl.desc}
                        </span>
                      </div>
                      {active && (
                        <Check className="h-3.5 w-3.5 text-purple-600 dark:text-purple-400 shrink-0" />
                      )}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>

      {/* Right: real model selection from config.toml + Orb send button */}
      <div className="flex items-center gap-2">
        {/* Real Model Selector Dropdown — STRICTLY from config.toml */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              title={`当前模型: ${activeModel || "未选择"} (${activeProvider})`}
              className="flex items-center gap-1.5 px-2.5 py-1 text-[12px] font-medium text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 rounded-lg transition-colors select-none cursor-pointer"
            >
              <span className="w-2 h-2 rounded-full bg-emerald-500 inline-block shrink-0" />
              <span className="truncate max-w-[140px] font-mono text-[11.5px]">
                {activeModel || "选择模型"}
              </span>
              <ChevronDown className="h-3 w-3 text-neutral-400 shrink-0" />
            </button>
          </DropdownMenuTrigger>

          <DropdownMenuContent align="end" side="top" className="w-56">
            <DropdownMenuLabel className="text-xs text-neutral-500 font-normal">
              模型
            </DropdownMenuLabel>
            <DropdownMenuSeparator />

            {/* List models strictly defined in config.toml */}
            {models.length > 0 ? (
              <DropdownMenuGroup>
                {models.map((item) => {
                  const active = item.model === activeModel && item.provider === activeProvider;
                  return (
                    <DropdownMenuItem
                      key={`${item.provider}-${item.model}`}
                      onClick={() => void onSelectModel(item.provider, item.model)}
                      className="flex items-center justify-between text-xs py-2 cursor-pointer"
                    >
                      <div className="flex flex-col min-w-0 pr-2">
                        <span className="font-mono font-medium truncate text-neutral-900 dark:text-neutral-100">
                          {item.model}
                        </span>
                        <span className="text-[10px] text-neutral-400">
                          {item.provider}
                        </span>
                      </div>
                      {active && <Check className="h-3.5 w-3.5 text-emerald-600 shrink-0" />}
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuGroup>
            ) : (
              <div className="px-3 py-3 text-xs text-neutral-400 text-center">
                未检测到模型
              </div>
            )}
          </DropdownMenuContent>
        </DropdownMenu>

        {/* Send / Stop Button with Orb Loading & Disabled Gray State */}
        {streaming ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="group relative w-8 h-8 rounded-full bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 border border-neutral-200/80 dark:border-neutral-700 flex items-center justify-center cursor-pointer transition-all shadow-xs"
                onClick={() => void cancel()}
                aria-label="中断回复"
              >
                <Orb
                  variant="B3"
                  size={16}
                  className="text-neutral-800 dark:text-neutral-200 group-hover:opacity-0 transition-opacity duration-150"
                />
                <Square
                  className="h-2.5 w-2.5 fill-neutral-800 dark:fill-neutral-200 text-neutral-800 dark:text-neutral-200 absolute opacity-0 group-hover:opacity-100 transition-opacity duration-150"
                />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">中断当前回复</TooltipContent>
          </Tooltip>
        ) : canSubmit ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="w-8 h-8 rounded-full bg-[#18181b] hover:bg-black text-white dark:bg-neutral-100 dark:hover:bg-white dark:text-neutral-900 flex items-center justify-center cursor-pointer transition-all shadow-xs active:scale-95"
                onClick={() => void submit()}
                aria-label="发送"
              >
                <ArrowUp className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">发送 (Enter)</TooltipContent>
          </Tooltip>
        ) : (
          <button
            type="button"
            disabled
            className="w-8 h-8 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-300 dark:text-neutral-600 flex items-center justify-center cursor-not-allowed border border-neutral-200/50 dark:border-neutral-700/50 transition-all select-none"
            aria-label="无法发送 (请输入内容)"
          >
            <ArrowUp className="h-4 w-4" />
          </button>
        )}
      </div>
    </div>
  );
}
