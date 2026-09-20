// Status bar — minimal bottom bar, shadcn Badge + Separator.

import type { SessionView } from "../../hooks/stream-view";
import type { AgentState } from "../../types";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

export function StatusBar({ view }: { view: SessionView }) {
  const label = stateLabel(view.state);
  const pct =
    view.usage.prompt + view.usage.completion > 0
      ? Math.round(
          (view.usage.completion / Math.max(1, view.usage.prompt + view.usage.completion)) * 100,
        )
      : null;

  return (
    <footer className="flex items-center gap-2 px-4 py-1.5 bg-white border-t border-neutral-200 text-[11px] text-neutral-500 select-none">
      <Badge
        variant="secondary"
        className={cn(
          "gap-1.5 border-0 font-normal text-[11px] px-2 py-0.5",
          view.streaming
            ? "bg-neutral-900 text-neutral-50"
            : "bg-neutral-100 text-neutral-500"
        )}
      >
        <span
          className={cn(
            "w-1.5 h-1.5 rounded-full",
            view.streaming ? "bg-emerald-400 animate-pulse" : "bg-neutral-400"
          )}
        />
        {label}
      </Badge>
      <span className="flex-1" />
      {(view.usage.prompt > 0 || view.usage.completion > 0) && (
        <>
          <span className="font-mono text-neutral-400">
            {fmtK(view.usage.prompt)} in · {fmtK(view.usage.completion)} out
          </span>
          {pct !== null && (
            <>
              <Separator orientation="vertical" className="h-3" />
              <span className="font-mono text-neutral-400">{pct}%</span>
            </>
          )}
        </>
      )}
    </footer>
  );
}

function fmtK(n: number) {
  return n >= 1000 ? `${Math.round(n / 1000)}K` : `${n}`;
}

function stateLabel(s: AgentState | null): string {
  if (!s) return "空闲";
  if (typeof s === "string") {
    switch (s) {
      case "Idle":              return "空闲";
      case "ScanningWorkspace": return "扫描工作区…";
      case "Reasoning":         return "思考中…";
      case "StreamingToken":    return "生成中…";
      case "Compacting":        return "压缩上下文中…";
      case "Finished":          return "已完成";
      default:                  return "处理中…";
    }
  }
  if ("AwaitingToolConfirmation" in s) return `等待确认 ${s.AwaitingToolConfirmation.tool_name}`;
  if ("AwaitingPluginConsent" in s)    return `等待授权 ${s.AwaitingPluginConsent.plugin_id}`;
  if ("AwaitingConsent" in s)          return `等待授权 ${s.AwaitingConsent.kind}`;
  if ("Branching" in s)                return `分支中 ×${s.Branching.candidates}`;
  if ("ExecutingTool" in s)            return `执行工具 ${s.ExecutingTool.tool_name}`;
  if ("Failed" in s)                   return `失败`;
  return "处理中…";
}
