import { useLayoutEffect, useRef, useState } from "react";
import { WindowControls } from "@/components/window-controls";
import { useCurrencySymbol } from "@/lib/appearance";
import { formatCacheRate } from "@/lib/cache-rate";
import { GitBranch, Zap, BarChartHorizontalStart, Database } from "@keyline-icons/react";
import { CardBody, CardDivider, CardHeader, CardRow, StatPopover } from "./stat-popover";
import { TooltipSimple } from "@/components/ui/tooltip";
import { StreamHealth } from "@/features/chat/components/stream-health";
import type { ContextBreakdown, SessionView } from "@/features/chat/hooks/stream-view";
import type { GitInfo, ModelItem } from "@/lib/agent-ipc/index";

interface TitleBarProps {
  title?: string;
  view?: SessionView;
  gitInfo?: GitInfo | null;
  /** Active model's context window — used as the meter denominator before
   * the first `Usage` event arrives. */
  contextWindowHint?: number;
  /** Active model's $/1M-token pricing — the cost chip renders only when
   * the model carries a `cost` block. */
  modelCost?: ModelItem["cost"];
  /** Opens the raw-JSON history viewer for the active session. */
  onShowRaw?: () => void;
  /** No workspace open — hide title, raw button and the stats cluster. */
  noWorkspace?: boolean;
  /** No ACTIVE session (workspace open, zero sessions) — git stays, the
   *  session-scoped stats (ctx/tokens/cost/rate, raw-JSON) would only show
   *  config defaults, so they hide. */
  noSession?: boolean;
}

function fmtRate(n: number) {
  return n >= 100 ? `${Math.round(n)}` : n.toFixed(1);
}

/** Thousands-separated exact count — the cards' detail figures. */
function fmtNum(n: number) {
  return n.toLocaleString("en-US");
}

/** Compact magnitude for chips: 912 · 12.4K · 161M. */
function fmtCompact(n: number) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}K`;
  return `${n}`;
}

/** Chip glyph box — sized to the 11px label. The half-pixel lift matches the
 * label's glyph box, which sits 0.5px above the flex centre at
 * `leading-none`; without it the icons read low beside the text. */
const CHIP_ICON = "block h-3 w-3 flex-none -translate-y-[0.5px]";

/** Context card categories, in draw order; empty ones are dropped. */
const CONTEXT_CATEGORIES: { key: keyof ContextBreakdown; label: string; swatch: string }[] = [
  { key: "system", label: "系统提示词", swatch: "bg-neutral-400" },
  { key: "tools", label: "工具定义", swatch: "bg-violet-400" },
  { key: "memory", label: "记忆召回", swatch: "bg-pink-400" },
  { key: "user", label: "用户消息", swatch: "bg-lime-500" },
  { key: "assistant", label: "模型回复", swatch: "bg-accent" },
  { key: "reasoning", label: "思考过程", swatch: "bg-cyan-400" },
  { key: "tool_calls", label: "工具调用", swatch: "bg-amber-500" },
  { key: "tool_results", label: "工具结果", swatch: "bg-emerald-500" },
  { key: "other", label: "其它", swatch: "bg-stone-400" },
];

/** Split the reported prompt total by the backend's estimated shares.
 * Without a breakdown (reopened session, before the first live round) the
 * card falls back to the cache split, which the wire reports exactly. */
function contextComposition(
  breakdown: ContextBreakdown | undefined,
  prompt: number,
  cached: number,
  cacheWrite: number,
  uncached: number,
): { key: string; label: string; swatch: string; tokens: number }[] {
  const est = breakdown ? CONTEXT_CATEGORIES.reduce((n, c) => n + (breakdown[c.key] ?? 0), 0) : 0;
  if (!breakdown || est === 0) {
    return [
      { key: "cached", label: "缓存前缀", swatch: "bg-emerald-500", tokens: cached },
      { key: "write", label: "新写入缓存", swatch: "bg-amber-500", tokens: cacheWrite },
      { key: "uncached", label: "未缓存", swatch: "bg-neutral-400", tokens: uncached },
    ].filter((c) => c.tokens > 0 || c.key === "uncached");
  }
  const scale = prompt > 0 ? prompt / est : 1;
  return CONTEXT_CATEGORIES.map((c) => ({ ...c, tokens: Math.round((breakdown[c.key] ?? 0) * scale) }))
    .filter((c) => c.tokens > 0);
}

/** Context-fill ring — the chip's at-a-glance meter. */
function ContextRing({ pct }: { pct: number }) {
  const r = 5.5;
  const c = 2 * Math.PI * r;
  const fill = Math.min(100, Math.max(0, pct));
  const stroke =
    pct >= 80 ? "stroke-red-500" : pct >= 50 ? "stroke-amber-500" : "stroke-accent";
  return (
    <svg viewBox="0 0 14 14" className="block h-[13px] w-[13px] flex-none -translate-y-[0.5px] -rotate-90" aria-hidden>
      <circle cx="7" cy="7" r={r} fill="none" strokeWidth="2" className="stroke-[color-mix(in_srgb,var(--husk-n300)_80%,transparent)]" />
      <circle
        cx="7"
        cy="7"
        r={r}
        fill="none"
        strokeWidth="2"
        strokeLinecap="round"
        strokeDasharray={`${(fill / 100) * c} ${c}`}
        className={`${stroke} transition-[stroke-dasharray] duration-500`}
      />
    </svg>
  );
}

/** Stacked horizontal bar; the unfilled remainder is the track. */
function SegmentBar({
  total,
  segments,
}: {
  total: number;
  segments: { value: number; className: string }[];
}) {
  return (
    <div className="mx-3.5 mt-2.5 mb-1 flex h-1.5 gap-px overflow-hidden rounded-full bg-[color-mix(in_srgb,var(--husk-n300)_80%,transparent)]">
      {segments.map((s, i) =>
        s.value > 0 ? (
          <div
            key={i}
            className={`${s.className} h-full transition-[width] duration-500`}
            style={{ width: `${Math.min(100, (s.value / Math.max(1, total)) * 100)}%` }}
          />
        ) : null,
      )}
    </div>
  );
}

/** Sub-dollar precision: 4 decimals under a cent, 3 under a dollar. */
function fmtCost(n: number, sym: string) {
  if (n === 0) return `${sym}0.00`;
  if (n < 0.01) return `${sym}${n.toFixed(4)}`;
  if (n < 1) return `${sym}${n.toFixed(3)}`;
  return `${sym}${n.toFixed(2)}`;
}

/** Running $ for the displayed usage — fresh prompt at `input`, cache hits
 * at `cache_read`, cache writes at `cache_write`, completion at `output`.
 * Prices are $/1M tokens. Anthropic reports writes separately; OpenAI-style
 * wires leave `cacheWriteTokens` at 0. */
function turnCost(
  prompt: number,
  cached: number,
  cacheWrite: number,
  completion: number,
  cost: ModelItem["cost"],
): number | null {
  if (!cost) return null;
  const input = cost.input ?? 0;
  const cacheRead = cost.cache_read ?? input;
  const cacheWritePrice = cost.cache_write ?? input;
  const output = cost.output ?? 0;
  return (
    (Math.max(0, prompt - cached - cacheWrite) * input +
      cached * cacheRead +
      cacheWrite * cacheWritePrice +
      completion * output) /
    1e6
  );
}

export function TitleBar({ title = "新会话", view, gitInfo, contextWindowHint, modelCost, onShowRaw, noWorkspace = false, noSession = false }: TitleBarProps) {
  const prompt = view?.usage.prompt ?? 0;
  const completion = view?.usage.completion ?? 0;
  const cached = view?.usage.cachedTokens ?? 0;
  const cacheWrite = view?.usage.cacheWriteTokens ?? 0;
  const uncached = Math.max(0, prompt - cached - cacheWrite);
  // Share of this request's prompt served from the provider cache. Kept as a
  // truncated decimal string: 99.9x% must not round up to 100%.
  const cacheRate = formatCacheRate(cached, prompt);
  // The hint (active model's window) wins over the turn-level usage: usage
  // remembers whatever model produced the LAST turn, so after a mid-session
  // model switch it would keep showing the old window until the next turn.
  // Switching down past current usage just reads ">100%" and goes red.
  const ctxWin = contextWindowHint || view?.usage.contextWindow || 256000;
  const pct = Math.round((prompt / ctxWin) * 100);
  const toks = view?.toksPerSec ?? 0;
  const cost = turnCost(prompt, cached, cacheWrite, completion, modelCost);
  const sym = useCurrencySymbol();
  const ctxColor =
    pct >= 80 ? "text-red-500 dark:text-red-400" : pct >= 50 ? "text-amber-500 dark:text-amber-400" : undefined;

  // Collapse threshold — below this the chips drop their text and keep
  // only icon + meter, so the window controls on the right are never
  // clipped. We flip on the bar's own width, not the chips', so a long
  // title can't hold it open and still starve the controls.
  const barRef = useRef<HTMLDivElement>(null);
  const [narrow, setNarrow] = useState(false);
  useLayoutEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setNarrow(el.clientWidth < 880);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Turn / tool-step tallies for the activity card. Tools count the loaded
  // slice only — older pages are summarised by the turn total.
  const items = view?.items ?? [];
  let toolSteps = 0;
  let toolFailed = 0;
  let userTurns = 0;
  let assistantRuns = 0;
  for (const it of items) {
    if (it.kind === "tool") {
      toolSteps += 1;
      if (it.ok === false) toolFailed += 1;
    } else if (it.kind === "user") {
      // A steer joins the running turn; it is not a turn of its own.
      if (!it.steer) userTurns += 1;
    } else if (it.kind === "assistant") assistantRuns += 1;
  }
  // Real execution counts, not "user sent a prompt": one prompt can run the
  // model/tool loop many times. Old snapshots have no totals; fall back to the
  // loaded slice rather than the user-turn count.
  //
  // The vocabulary follows the lifecycle the kernel implements: a **turn** is
  // one accepted prompt drained to completion, a **step** is one model request
  // plus the tool calls it triggered, and a **call** is one of those tool
  // executions (several per step is the normal case).
  const turns = view?.turnTotal ?? userTurns;
  // The totals are authoritative (persisted, then advanced by live events and
  // re-based by `husk:session_counts`), so they can go DOWN after /clear or
  // compaction. Only snapshots older than the counts fall back to the slice.
  const runs = view?.runTotal ?? assistantRuns;
  const tasks = view?.taskTotal ?? toolSteps;
  const breakdown = view?.contextBreakdown;
  const composition = contextComposition(breakdown, prompt, cached, cacheWrite, uncached);
  const totalTokens = prompt + completion;

  // The stat chips — `compact` drops the text and keeps icon + glyph, so
  // a narrow bar still has every card one hover away.
  const statsChips = (compact: boolean) => (
    <>
      {!noSession && <StreamHealth />}
      {/* Only while there is something to act on. A clean branch — everything
          committed — is not news, and a permanently lit chip in a 36px bar
          costs more than the branch name is worth. */}
      {gitInfo?.branch && gitInfo.dirty > 0 && (
        <StatPopover
          label={`Git 分支 ${gitInfo.branch}`}
          className="w-64"
          trigger={
            <>
              <GitBranch className={CHIP_ICON} />
              {!compact && <span className="max-w-[140px] truncate">{gitInfo.branch}</span>}
              {gitInfo.dirty > 0 && (
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500" aria-hidden />
              )}
            </>
          }
        >
          <CardHeader icon={<GitBranch className="h-4 w-4" />} title="Git" />
          <CardDivider />
          <CardBody>
            <CardRow label="分支" value={<span className="font-mono">{gitInfo.branch}</span>} />
            <CardRow
              label="未提交修改"
              value={gitInfo.dirty > 0 ? `${gitInfo.dirty} 处` : "无"}
              muted={gitInfo.dirty === 0}
            />
          </CardBody>
        </StatPopover>
      )}
      {!noSession && (<>
      <StatPopover
        label="生成活动"
        trigger={
          <>
            <Zap className={CHIP_ICON} />
            {!compact && (
              <span>
                {runs} 步 · {tasks} 调用<span className="mx-1 text-neutral-400">·</span>
                {fmtRate(toks)} tok/s
              </span>
            )}
          </>
        }
      >
        <CardHeader icon={<Zap className="h-4 w-4" />} title="生成活动" value={`${fmtRate(toks)} tok/s`} />
        <CardDivider />
        <CardBody>
          <CardRow label="用户轮次" value={turns} />
          <CardRow label="模型步数" value={runs} />
          <CardRow label="工具调用" value={tasks} />
          {toolFailed > 0 && <CardRow label="工具失败" value={<span className="text-red-500">{toolFailed}</span>} />}
          <CardRow label="本轮生成" value={`${fmtNum(completion)} tok`} />
        </CardBody>
      </StatPopover>
      <StatPopover
        label="Token 用量"
        trigger={
          <>
            <Database className={CHIP_ICON} />
            {!compact && <span>{fmtCompact(totalTokens)} tok</span>}
            {!compact && cacheRate !== null && (
              <span className="text-neutral-500">缓存 {cacheRate}</span>
            )}
            {!compact && cost !== null && (
              <span className="text-neutral-500">{fmtCost(cost, sym)}</span>
            )}
          </>
        }
      >
        <CardHeader
          icon={<Database className="h-4 w-4" />}
          title="Token 用量"
          value={`${fmtNum(totalTokens)} tok`}
        />
        <CardDivider />
        <CardBody>
          <CardRow label="缓存命中" value={cacheRate ?? "—"} />
          <CardRow label="缓存读取" value={`${fmtNum(cached)} tok`} swatch="bg-emerald-500" />
          {cacheWrite > 0 && (
            <CardRow label="缓存写入" value={`${fmtNum(cacheWrite)} tok`} swatch="bg-amber-500" />
          )}
          <CardRow label="未缓存输入" value={`${fmtNum(uncached)} tok`} swatch="bg-neutral-400" />
          <CardRow label="输出" value={`${fmtNum(completion)} tok`} swatch="bg-accent" />
        </CardBody>
        {cost !== null && (
          <>
            <CardDivider />
            <CardBody>
              <CardRow label="本轮花费" value={fmtCost(cost, sym)} />
              <CardRow
                label="单价 / M"
                muted
                value={
                  `入 ${sym}${modelCost?.input ?? 0}` +
                  (modelCost?.cache_read != null ? ` · 读 ${sym}${modelCost.cache_read}` : "") +
                  ` · 出 ${sym}${modelCost?.output ?? 0}`
                }
              />
            </CardBody>
          </>
        )}
        <p className="px-3.5 pb-3 text-[11px] text-neutral-400">按最近一次请求统计</p>
      </StatPopover>
      <StatPopover
        label="上下文窗口"
        trigger={
          <>
            <ContextRing pct={pct} />
            {!compact && <span className={ctxColor}>{pct}%</span>}
          </>
        }
      >
        <div className="flex items-baseline gap-2 px-3.5 pt-3">
          <span className="text-neutral-500">上下文已用</span>
          <span className={`font-medium ${ctxColor ?? "text-neutral-800"}`}>{pct}%</span>
          <span className="ml-auto tabular-nums text-neutral-800">
            ~{fmtCompact(prompt)} / {fmtCompact(ctxWin)}
          </span>
        </div>
        <SegmentBar
          total={ctxWin}
          segments={composition.map((c) => ({ value: c.tokens, className: c.swatch }))}
        />
        <CardBody>
          {composition.map((c) => (
            <CardRow key={c.key} label={c.label} value={`~${fmtCompact(c.tokens)}`} swatch={c.swatch} />
          ))}
          <CardRow
            label="剩余空间"
            value={`~${fmtCompact(Math.max(0, ctxWin - prompt))}`}
            swatch="bg-[color-mix(in_srgb,var(--husk-n300)_80%,transparent)]"
            muted
          />
        </CardBody>
        <CardDivider />
        <CardBody>
          <CardRow label="缓存命中" value={cacheRate !== null ? `${cacheRate} · ~${fmtCompact(cached)}` : "—"} muted />
        </CardBody>
        {breakdown && (
          <p className="px-3.5 pb-3 text-[11px] text-neutral-400">各项为按字符估算后按实际用量缩放</p>
        )}
      </StatPopover>
      </>)}
    </>
  );

  return (
    <div
      ref={barRef}
      data-tauri-drag-region="deep"
      className="flex items-center h-9 flex-none bg-white border-b border-[color-mix(in_srgb,var(--husk-n200)_80%,transparent)] select-none px-3 justify-between"
    >
      {!noWorkspace && (
      <div className="flex items-center min-w-0 max-w-[500px]">
        <TooltipSimple content={title} side="bottom">
          <span
            className="text-[13px] font-medium text-neutral-800 truncate cursor-default"
          >
            {title}
          </span>
        </TooltipSimple>
        {onShowRaw && !noSession && (
          <TooltipSimple content="查看原始对话 (JSON)" side="bottom">
            <button
              type="button"
              data-tauri-drag-region="false"
              onClick={onShowRaw}
              className="ml-1 flex-none rounded p-1 text-neutral-500 hover:text-neutral-700 hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)] transition-colors"
              aria-label="查看原始对话 JSON"
            >
              <BarChartHorizontalStart className="h-3.5 w-3.5" />
            </button>
          </TooltipSimple>
        )}
      </div>
      )}

      <div className="flex-1 h-full" />

      {/* Session stats — always rendered (zeroed before the first turn) so
       * the meter cluster doesn't pop in mid-conversation. Each chip opens
       * its detail card on hover; a click pins it. Below the collapse
       * threshold the chips shrink to icon + meter. */}
      {!noWorkspace && (
        // Optical nudge, not layout: these chips are 11px text sitting next to
        // 18–20px window glyphs, so a box-perfect centre still reads a touch
        // high. 1px down puts the row's visual mass on the same line as the
        // minimise/maximise/close icons.
        <div className="flex items-center gap-0.5 flex-none mr-2 min-w-0 translate-y-[1px]">
          {statsChips(narrow)}
        </div>
      )}

      <WindowControls />
    </div>
  );
}
