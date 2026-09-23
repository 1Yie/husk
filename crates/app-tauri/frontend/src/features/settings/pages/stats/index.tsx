// 「统计」pane — what the agent has cost and when it was busy.
//
// The kernel hands back raw per-session records (`usage_stats`); every bucket
// here is computed in `features/settings/usage.ts` so the rules are testable and
// the timezone is the reader's. Charts are plain divs/SVG — a charting
// dependency would outweigh the three shapes actually needed.

import { useEffect, useMemo, useState } from "react";
import { Activity, Coins, Layers, Sparkles } from "@keyline-icons/react";
import { cn } from "@/lib/utils";
import { getUsageStats, type UsageRecord } from "@/lib/agent-ipc/sessions";
import { aggregate, fmtTokens, type UsageStats } from "@/features/settings/usage";
import { ActivityHeatmap } from "@/features/settings/components/activity-heatmap";

export function StatsPane() {
  const [records, setRecords] = useState<UsageRecord[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let alive = true;
    void getUsageStats()
      .then((r) => alive && setRecords(r.sessions ?? []))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, []);

  const stats: UsageStats | null = useMemo(
    () => (records ? aggregate(records) : null),
    [records]
  );

  if (error) return <p className="text-xs text-red-500">读取统计失败：{error}</p>;
  if (!stats)
    return <p className="text-xs text-neutral-500">读取中…</p>;
  if (records && records.length === 0)
    return (
      <p className="text-xs text-neutral-500">
        还没有用量记录 —— 完成一轮对话后这里会显示 token 与活跃度。
      </p>
    );

  const { totals, daily, byModel, byWorkspace, streak, busiest } = stats;
  const last30 = daily.slice(-30);
  const peak30 = last30.reduce((m, d) => Math.max(m, d.tokens), 0) || 1;
  const modelMax =
    byModel.reduce((m, x) => Math.max(m, x.prompt + x.completion), 0) || 1;
  const wsMax =
    byWorkspace.reduce((m, x) => Math.max(m, x.prompt + x.completion), 0) || 1;

  return (
    <div className="flex flex-col gap-6">
      {/* KPI 行 */}
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        <Kpi
          Icon={Coins}
          label="总 token"
          value={fmtTokens(totals.prompt + totals.completion)}
          hint={`输入 ${fmtTokens(totals.prompt)} · 输出 ${fmtTokens(totals.completion)}`}
        />
        <Kpi
          Icon={Layers}
          label="会话"
          value={String(totals.sessions)}
          hint={`活跃 ${totals.activeDays} 天 · 连续 ${streak} 天`}
        />
        <Kpi
          Icon={Activity}
          label="缓存命中"
          value={
            totals.prompt + totals.cached > 0
              ? `${Math.round((totals.cached / (totals.prompt + totals.cached)) * 100)}%`
              : "—"
          }
          hint={`${fmtTokens(totals.cached)} tokens 走缓存`}
        />
        <Kpi
          Icon={Sparkles}
          label="最忙的一天"
          value={busiest ? busiest.date.slice(5) : "—"}
          hint={busiest ? `${busiest.sessions} 个会话 · ${fmtTokens(busiest.tokens)} tokens` : ""}
        />
      </div>

      {/* 热力图 */}
      <section className="rounded-[14px] border border-hairline bg-card px-4 py-3.5">
        <header className="mb-3 flex items-baseline justify-between">
          <h2 className="text-[13px] font-medium text-neutral-800">活跃对话</h2>
          <span className="text-[11px] text-neutral-500">每日 · 会话数</span>
        </header>
        <ActivityHeatmap daily={daily} />
      </section>

      {/* 近 30 天 token 趋势 */}
      <section className="rounded-[14px] border border-hairline bg-card px-4 py-3.5">
        <header className="mb-3 flex items-baseline justify-between">
          <h2 className="text-[13px] font-medium text-neutral-800">近 30 天 token</h2>
          <span className="text-[11px] text-neutral-500">峰值 {fmtTokens(peak30)}/天</span>
        </header>
        <div className="flex h-[72px] items-end gap-[3px]">
          {last30.map((d) => (
            <span
              key={d.date}
              title={`${d.date} · ${fmtTokens(d.tokens)} tokens`}
              className={cn(
                "min-h-[2px] flex-1 rounded-[2px] bg-[color-mix(in_srgb,var(--husk-activity)_70%,transparent)]",
                d.tokens === 0 && "bg-[var(--husk-n100)] dark:bg-[var(--husk-n800)]"
              )}
              style={{ height: `${Math.max(2, (d.tokens / peak30) * 72)}px` }}
            />
          ))}
        </div>
      </section>

      {/* 逐工作区 — which PROJECTS spent the tokens. Same two-share bar as the
          model breakdown, sorted by volume, newest activity as the hint. */}
      {byWorkspace.length > 0 && (
        <section className="rounded-[14px] border border-hairline bg-card px-4 py-3.5">
          <header className="mb-3 flex items-baseline justify-between">
            <h2 className="text-[13px] font-medium text-neutral-800">工作区用量</h2>
            <span className="text-[11px] text-neutral-500">{byWorkspace.length} 个工作区</span>
          </header>
          <div className="flex flex-col gap-2.5">
            {byWorkspace.map((w) => {
              const total = w.prompt + w.completion;
              return (
                <div key={w.workspace} className="flex flex-col gap-1">
                  <div className="flex items-baseline justify-between text-[12px]">
                    <span className="truncate text-neutral-800" title={w.workspace}>
                      {w.workspace}
                    </span>
                    <span className="shrink-0 tabular-nums text-neutral-500">
                      {fmtTokens(total)} · {w.sessions} 会话 · {dayAgo(w.lastTs)}
                    </span>
                  </div>
                  <div className="flex h-[6px] overflow-hidden rounded-full bg-[var(--husk-n100)] dark:bg-[var(--husk-n800)]">
                    <span
                      className="bg-[color-mix(in_srgb,var(--husk-activity)_85%,transparent)]"
                      style={{ width: `${(w.prompt / wsMax) * 100}%` }}
                      title={`输入 ${w.prompt}`}
                    />
                    <span
                      className="bg-[color-mix(in_srgb,var(--husk-accent)_70%,transparent)]"
                      style={{ width: `${(w.completion / wsMax) * 100}%` }}
                      title={`输出 ${w.completion}`}
                    />
                  </div>
                </div>
              );
            })}
          </div>
          <div className="mt-3 flex items-center gap-4 text-[11px] text-neutral-500">
            <Legend className="bg-[color-mix(in_srgb,var(--husk-activity)_85%,transparent)]" label="输入" />
            <Legend className="bg-[color-mix(in_srgb,var(--husk-accent)_70%,transparent)]" label="输出" />
          </div>
        </section>
      )}

      {/* 逐模型 */}
      {byModel.length > 0 && (
        <section className="rounded-[14px] border border-hairline bg-card px-4 py-3.5">
          <header className="mb-3 flex items-baseline justify-between">
            <h2 className="text-[13px] font-medium text-neutral-800">模型用量</h2>
            <span className="text-[11px] text-neutral-500">{byModel.length} 个模型</span>
          </header>
          <div className="flex flex-col gap-2.5">
            {byModel.map((m) => {
              const total = m.prompt + m.completion;
              return (
                <div key={`${m.provider}:${m.model}`} className="flex flex-col gap-1">
                  <div className="flex items-baseline justify-between text-[12px]">
                    <span className="truncate font-mono text-neutral-800">
                      {m.model}
                      {m.provider && (
                        <span className="ml-2 font-sans text-[11px] text-neutral-500">
                          {m.provider}
                        </span>
                      )}
                    </span>
                    <span className="shrink-0 tabular-nums text-neutral-500">
                      {fmtTokens(total)} · {m.sessions} 会话
                    </span>
                  </div>
                  {/* 输入/输出分段条 — one bar, two shares. */}
                  <div className="flex h-[6px] overflow-hidden rounded-full bg-[var(--husk-n100)] dark:bg-[var(--husk-n800)]">
                    <span
                      className="bg-[color-mix(in_srgb,var(--husk-activity)_85%,transparent)]"
                      style={{ width: `${(m.prompt / modelMax) * 100}%` }}
                      title={`输入 ${m.prompt}`}
                    />
                    <span
                      className="bg-[color-mix(in_srgb,var(--husk-accent)_70%,transparent)]"
                      style={{ width: `${(m.completion / modelMax) * 100}%` }}
                      title={`输出 ${m.completion}`}
                    />
                  </div>
                </div>
              );
            })}
          </div>
          <div className="mt-3 flex items-center gap-4 text-[11px] text-neutral-500">
            <Legend className="bg-[color-mix(in_srgb,var(--husk-activity)_85%,transparent)]" label="输入" />
            <Legend className="bg-[color-mix(in_srgb,var(--husk-accent)_70%,transparent)]" label="输出" />
          </div>
        </section>
      )}
    </div>
  );
}

/** `3 天前`-style hint for a workspace's newest activity. */
function dayAgo(ts: number): string {
  if (!ts) return "";
  const days = Math.floor((Date.now() / 1000 - ts) / 86400);
  if (days <= 0) return "今天";
  if (days === 1) return "昨天";
  if (days < 30) return `${days} 天前`;
  return `${Math.floor(days / 30)} 个月前`;
}

function Kpi({
  Icon,
  label,
  value,
  hint,
}: {
  Icon: (p: { className?: string }) => JSX.Element;
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <div className="rounded-[14px] border border-hairline bg-card px-3.5 py-3">
      <span className="flex items-center gap-1.5 text-[11px] text-neutral-500">
        <Icon className="h-3.5 w-3.5" />
        {label}
      </span>
      <span className="mt-1 block text-[18px] font-semibold tabular-nums tracking-tight text-neutral-900">
        {value}
      </span>
      {hint && <span className="mt-0.5 block text-[11px] text-neutral-500">{hint}</span>}
    </div>
  );
}

function Legend({ className, label }: { className: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className={cn("h-[6px] w-[14px] rounded-full", className)} />
      {label}
    </span>
  );
}
