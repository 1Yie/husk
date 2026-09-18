// State pill — the "agent is working" indicator appended to the stream.

import type { AgentState } from "../../types";
import { Orb } from "../agent-orb";

export function StatePill({ state }: { state: AgentState | null }) {
  const label = stateLabel(state);
  return (
    <div className="py-1">
      <span className="inline-flex items-center gap-1.5 h-[30px] pl-2 pr-3.5 rounded-full bg-white border border-neutral-200/90 text-neutral-700 text-xs shadow-sm">
        <Orb variant="S1" size={16} className="text-purple-600" />
        <span className="font-medium">{label}</span>
      </span>
    </div>
  );
}

function stateLabel(s: AgentState | null): string {
  if (!s) return "处理中…";
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
  if ("Failed" in s)                   return "失败";
  return "处理中…";
}
