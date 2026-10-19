// Charts use the existing local-time usage aggregation and theme palette.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Activity, Calendar, ChartColumn, Coins, Cpu, Folder, Layers } from "@keyline-icons/react";
import { Button } from "@/components/ui/button";
import { KvList } from "@/components/ui/kv-list";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState, ItemIcon, SettingsSection, Tag } from "@/features/settings/components";
import { ActivityHeatmap } from "@/features/settings/components/activity-heatmap";
import { aggregate, fmtTokens } from "@/features/settings/usage";
import { getUsageStats, type UsageRecord } from "@/lib/agent-ipc/sessions";
import { formatCacheRate } from "@/lib/cache-rate";
import { SURFACE_ACTIVITY_EMPTY } from "@/lib/theme";
import { cn } from "@/lib/utils";

const INPUT_COLOR = "bg-[color-mix(in_srgb,var(--husk-activity)_85%,transparent)]";
const OUTPUT_COLOR = "bg-[color-mix(in_srgb,var(--husk-accent)_70%,transparent)]";

export function StatsPane() {
  const [records, setRecords] = useState<UsageRecord[] | null>(null);
  const [error, setError] = useState("");
  const [loadAttempt, setLoadAttempt] = useState(0);

  useEffect(() => {
    let alive = true;
    setError("");
    void getUsageStats()
      .then((r) => { if (alive) setRecords(r.sessions ?? []); })
      .catch((e) => { if (alive) setError(String(e)); });
    return () => { alive = false; };
  }, [loadAttempt]);

  const stats = useMemo(() => records ? aggregate(records) : null, [records]);

  if (error) {
    return (
      <KvList>
        <EmptyState
          icon={<ChartColumn className="h-4 w-4" />}
          title="无法读取统计"
          description={<span className="break-all">{error}</span>}
          action={<Button size="sm" variant="outline" onClick={() => setLoadAttempt((n) => n + 1)}>重试</Button>}
        />
      </KvList>
    );
  }
  if (!stats) return <StatsSkeleton />;
  if (!records?.length) {
    return (
      <KvList>
        <EmptyState
          icon={<ChartColumn className="h-4 w-4" />}
          title="暂无用量记录"
          description="完成对话后，可在这里查看 Token 用量与活跃度"
        />
      </KvList>
    );
  }

  const { totals, daily, byModel, byWorkspace, streak, busiest } = stats;
  const last30 = daily.slice(-30);
  const peak30 = Math.max(0, ...last30.map((d) => d.tokens));
  const modelMax = Math.max(1, ...byModel.map((m) => m.prompt + m.completion));
  const wsMax = Math.max(1, ...byWorkspace.map((w) => w.prompt + w.completion));

  return (
    <div className="flex min-w-0 flex-col gap-8">
      <SettingsSection title="用量概览">
        <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
          <Kpi
            icon={<Coins className="h-4 w-4" />}
            label="总 Token"
            value={fmtTokens(totals.prompt + totals.completion)}
            hint={`输入 ${fmtTokens(totals.prompt)} · 输出 ${fmtTokens(totals.completion)}`}
          />
          <Kpi
            icon={<Layers className="h-4 w-4" />}
            label="会话数"
            value={String(totals.sessions)}
            hint={`活跃 ${totals.activeDays} 天 · 最长连续 ${streak} 天`}
          />
          <Kpi
            icon={<Activity className="h-4 w-4" />}
            label="缓存命中"
            value={formatCacheRate(totals.cached, totals.prompt) ?? "—"}
            hint={`${fmtTokens(totals.cached)} 命中 · ${fmtTokens(totals.cacheWrite)} 写入`}
          />
          <Kpi
            icon={<Calendar className="h-4 w-4" />}
            label="最忙的一天"
            value={busiest ? busiest.date.slice(5) : "—"}
            hint={busiest ? `${busiest.sessions} 个会话 · ${fmtTokens(busiest.tokens)} Token` : "暂无活跃记录"}
          />
        </div>
      </SettingsSection>

      <StatsCard icon={<Activity className="h-4 w-4" />} title="活跃对话" detail="近一年">
        <div className="overflow-x-auto pb-1">
          <div className="min-w-[480px]"><ActivityHeatmap daily={daily} /></div>
        </div>
      </StatsCard>

      <StatsCard icon={<ChartColumn className="h-4 w-4" />} title="Token 趋势" detail="近 30 天">
        <div className="mb-5 flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <span className="text-2xl font-semibold tabular-nums tracking-tight text-neutral-900">
            {fmtTokens(last30.reduce((sum, d) => sum + d.tokens, 0))}
          </span>
          <span className="text-xs text-neutral-500">Token</span>
          <span className="ml-auto text-[11px] tabular-nums text-neutral-500">峰值 {fmtTokens(peak30)} / 天</span>
        </div>
        <div className="flex h-24 items-end gap-1" role="img" aria-label="近 30 天每日 Token 用量">
          {last30.map((d) => (
            <span
              key={d.date}
              title={`${d.date} · ${fmtTokens(d.tokens)} Token`}
              className={cn(
                "min-w-0 flex-1 rounded-t-sm transition-opacity hover:opacity-70",
                d.tokens ? INPUT_COLOR : SURFACE_ACTIVITY_EMPTY,
              )}
              style={{ height: `${Math.max(3, d.tokens / (peak30 || 1) * 96)}px` }}
            />
          ))}
        </div>
        <div className="mt-2 flex justify-between text-[10px] tabular-nums text-neutral-500">
          <span>{last30[0]?.date.slice(5)}</span>
          <span>今天</span>
        </div>
      </StatsCard>

      {byWorkspace.length > 0 && (
        <StatsCard icon={<Folder className="h-4 w-4" />} title="工作区用量" detail={`${byWorkspace.length} 个工作区`}>
          <div className="flex flex-col gap-5">
            {byWorkspace.map((w) => (
              <UsageRow
                key={w.workspace}
                title={workspaceName(w.workspace)}
                description={w.workspace}
                prompt={w.prompt}
                completion={w.completion}
                max={wsMax}
                hint={`${w.sessions} 个会话 · ${dayAgo(w.lastTs)}`}
              />
            ))}
          </div>
          <UsageLegend />
        </StatsCard>
      )}

      {byModel.length > 0 && (
        <StatsCard icon={<Cpu className="h-4 w-4" />} title="模型用量" detail={`${byModel.length} 个模型`}>
          <div className="flex flex-col gap-5">
            {byModel.map((m) => (
              <UsageRow
                key={`${m.provider}:${m.model}`}
                title={m.model}
                provider={m.provider}
                prompt={m.prompt}
                completion={m.completion}
                max={modelMax}
                hint={`${m.sessions} 个会话`}
              />
            ))}
          </div>
          <UsageLegend />
        </StatsCard>
      )}
    </div>
  );
}

function StatsCard({ icon, title, detail, children }: {
  icon: ReactNode;
  title: string;
  detail: string;
  children: ReactNode;
}) {
  return (
    <KvList>
      <header className="flex flex-wrap items-center gap-3 border-b border-hairline px-5 py-4">
        <ItemIcon className="mt-0">{icon}</ItemIcon>
        <h2 className="text-sm font-medium text-neutral-900">{title}</h2>
        <span className="ml-auto text-[11px] text-neutral-500">{detail}</span>
      </header>
      <div className="p-5">{children}</div>
    </KvList>
  );
}

function Kpi({ icon, label, value, hint }: { icon: ReactNode; label: string; value: string; hint: string }) {
  return (
    <KvList className="min-w-0 p-4">
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <ItemIcon className="mt-0 h-7 w-7 rounded-md">{icon}</ItemIcon>
        <span className="text-[11px] text-neutral-500">{label}</span>
      </div>
      <span className="block break-all text-2xl font-semibold tabular-nums tracking-tight text-neutral-900">{value}</span>
      <span className="mt-1 block text-[11px] leading-relaxed text-neutral-500">{hint}</span>
    </KvList>
  );
}

function UsageRow({ title, description, provider, prompt, completion, max, hint }: {
  title: string;
  description?: string;
  provider?: string;
  prompt: number;
  completion: number;
  max: number;
  hint: string;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1.5">
        <div className="min-w-0 flex-[1_1_180px]">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-[13px] text-neutral-800" title={title}>{title}</span>
            {provider && <Tag className="max-w-[45%] [&>span]:truncate"><span>{provider}</span></Tag>}
          </div>
          {description && (
            <span title={description} className="mt-1 block truncate font-mono text-[10px] text-neutral-500 select-text">{description}</span>
          )}
        </div>
        <div className="ml-auto text-right">
          <span className="text-[13px] font-medium tabular-nums text-neutral-800">{fmtTokens(prompt + completion)}</span>
          <span className="ml-1 text-[10px] text-neutral-500">Token</span>
          <span className="mt-0.5 block text-[10px] tabular-nums text-neutral-500">{hint}</span>
        </div>
      </div>
      <div className={cn("flex h-1.5 overflow-hidden rounded-full", SURFACE_ACTIVITY_EMPTY)}>
        <span className={INPUT_COLOR} style={{ width: `${prompt / max * 100}%` }} title={`输入 ${prompt}`} />
        <span className={OUTPUT_COLOR} style={{ width: `${completion / max * 100}%` }} title={`输出 ${completion}`} />
      </div>
    </div>
  );
}

function UsageLegend() {
  return (
    <div className="mt-5 flex items-center gap-4 text-[11px] text-neutral-500">
      <span className="flex items-center gap-1.5"><span className={cn("h-1.5 w-3 rounded-full", INPUT_COLOR)} />输入</span>
      <span className="flex items-center gap-1.5"><span className={cn("h-1.5 w-3 rounded-full", OUTPUT_COLOR)} />输出</span>
    </div>
  );
}

function StatsSkeleton() {
  return (
    <div className="flex flex-col gap-8 animate-pulse" aria-busy="true" aria-label="正在加载统计">
      <div className="flex flex-col gap-3">
        <Skeleton className="h-4 w-20" />
        <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
          {[0, 1, 2, 3].map((i) => (
            <KvList key={i} className="min-w-0 p-4">
              <Skeleton className="mb-3 h-7 w-7" />
              <Skeleton className="h-6 w-16" />
              <Skeleton className="mt-2 h-3 w-full max-w-28" />
            </KvList>
          ))}
        </div>
      </div>
      {[0, 1, 2, 3].map((i) => (
        <KvList key={i}>
          <div className="flex items-center gap-3 border-b border-hairline px-5 py-4">
            <Skeleton className="h-8 w-8" />
            <Skeleton className="h-4 w-20" />
          </div>
          <div className="p-5"><Skeleton className="h-28 w-full rounded-xl" /></div>
        </KvList>
      ))}
    </div>
  );
}

function workspaceName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) || path;
}

function dayAgo(ts: number): string {
  if (!ts) return "";
  const days = Math.floor((Date.now() / 1000 - ts) / 86400);
  if (days <= 0) return "今天";
  if (days === 1) return "昨天";
  if (days < 30) return `${days} 天前`;
  return `${Math.floor(days / 30)} 个月前`;
}
