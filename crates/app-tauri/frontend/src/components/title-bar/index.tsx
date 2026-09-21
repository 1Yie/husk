import { WindowControls } from "@/components/window-controls";
import { isMac } from "@/lib/platform";
import { GitBranch, ChartPie, Zap, BarChartHorizontalStart, Inbox } from "@keyline-icons/react";
import { BrainCircuit } from "lucide-react";
import { TooltipSimple } from "@/components/ui/tooltip";
import type { SessionView } from "../../hooks/stream-view";
import type { GitInfo } from "../../invoke/agent";

interface TitleBarProps {
  title?: string;
  view?: SessionView;
  gitInfo?: GitInfo | null;
  /** Active model's context window — used as the meter denominator before
   * the first `Usage` event arrives. */
  contextWindowHint?: number;
  /** Opens the raw-JSON history viewer for the active session. */
  onShowRaw?: () => void;
}

function fmtK(n: number) {
  return n >= 1000 ? `${Math.round(n / 1000)}K` : `${n}`;
}

function fmtRate(n: number) {
  return n >= 100 ? `${Math.round(n)}` : n.toFixed(1);
}

export function TitleBar({ title = "新会话", view, gitInfo, contextWindowHint, onShowRaw }: TitleBarProps) {
  const prompt = view?.usage.prompt ?? 0;
  const completion = view?.usage.completion ?? 0;
  const cached = view?.usage.cachedTokens ?? 0;
  const uncached = Math.max(0, prompt - cached);
  const ctxWin = view?.usage.contextWindow || contextWindowHint || 256000;
  const pct = Math.round((prompt / ctxWin) * 100);
  const toks = view?.toksPerSec ?? 0;
  const ctxColor =
    pct >= 80 ? "text-red-500 dark:text-red-400" : pct >= 50 ? "text-amber-500 dark:text-amber-400" : undefined;

  return (
    <div
      data-tauri-drag-region="deep"
      className="flex items-center h-9 flex-none bg-white border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] select-none px-3 justify-between"
    >
      {isMac && <div className="w-[78px] shrink-0" />}

      <div className="flex items-center min-w-0 max-w-[500px]">
        <TooltipSimple content={title} side="bottom">
          <span
            className="text-[13px] font-medium text-neutral-800 truncate cursor-default"
          >
            {title}
          </span>
        </TooltipSimple>
        {onShowRaw && (
          <TooltipSimple content="查看原始对话 (JSON)" side="bottom">
            <button
              type="button"
              data-tauri-drag-region="false"
              onClick={onShowRaw}
              className="ml-1 flex-none rounded p-1 text-neutral-400 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] transition-colors"
              aria-label="查看原始对话 JSON"
            >
              <BarChartHorizontalStart className="h-3.5 w-3.5" />
            </button>
          </TooltipSimple>
        )}
      </div>

      <div className="flex-1 h-full" />

      {/* Session stats — always rendered (zeroed before the first turn) so
       * the meter cluster doesn't pop in mid-conversation. The git chip is
       * the only conditional one: outside a repo there is no branch to show. */}
      <div
        className="flex items-center gap-3 flex-none mr-2 text-[11px] font-mono text-neutral-400"
      >
        {gitInfo?.branch && (
          <TooltipSimple content={`Git 分支: ${gitInfo.branch}${gitInfo.dirty > 0 ? ` (${gitInfo.dirty} 处未提交修改)` : ""}`} side="bottom">
            <span className="flex items-center gap-1 cursor-default">
              <GitBranch className="h-3 w-3" />
              {gitInfo.branch}
              {gitInfo.dirty > 0 && (
                <span className="text-amber-500 dark:text-amber-400">·{gitInfo.dirty}</span>
              )}
            </span>
          </TooltipSimple>
        )}
        <TooltipSimple content={`上下文窗口占用: ${fmtK(prompt)}/${fmtK(ctxWin)} (${pct}%)`} side="bottom">
          <span
            className={`flex items-center gap-1 cursor-default ${ctxColor ?? ""}`}
          >
            <ChartPie className="h-3 w-3" />
            {fmtK(prompt)}/{fmtK(ctxWin)} · {pct}%
          </span>
        </TooltipSimple>
        <TooltipSimple content={`本轮模型生成 Token: ${fmtK(completion)}`} side="bottom">
          <span className="flex items-center gap-1 cursor-default">
            <BrainCircuit className="h-3 w-3" />
            {fmtK(completion)}
          </span>
        </TooltipSimple>
        <TooltipSimple
          content={`提示词缓存命中: ${fmtK(cached)} · 未缓存: ${fmtK(uncached)}`}
          side="bottom"
        >
          <span className="flex items-center gap-1 cursor-default">
            <Inbox className="h-3 w-3" />
            {fmtK(cached)}/{fmtK(uncached)}
          </span>
        </TooltipSimple>
        <TooltipSimple content={`生成速率: ${fmtRate(toks)} tok/s`} side="bottom">
          <span className="flex items-center gap-1 cursor-default">
            <Zap className="h-3 w-3" />
            {fmtRate(toks)} tok/s
          </span>
        </TooltipSimple>
      </div>

      <WindowControls />
    </div>
  );
}
