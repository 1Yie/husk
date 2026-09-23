// Aggregation for the 统计 pane. Pure functions over the raw records the kernel
// returns, so the bucketing rules are testable and the components stay dumb.

import type { UsageRecord } from "@/lib/agent-ipc/sessions";

/** Local `YYYY-MM-DD` — `toISOString` would shift the day for anyone east of
 *  UTC, which is exactly the bug a stats page cannot afford. */
function dayKey(seconds: number): string {
  const d = new Date(seconds * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export interface DayBucket {
  date: string;
  /** Sessions that ran that day. */
  sessions: number;
  tokens: number;
}

export interface WorkspaceBucket {
  workspace: string;
  prompt: number;
  completion: number;
  sessions: number;
  /** Newest activity in this workspace (unix seconds) — the sort key. */
  lastTs: number;
}

export interface ModelBucket {
  model: string;
  provider: string;
  prompt: number;
  completion: number;
  sessions: number;
}

export interface UsageStats {
  /** One entry per day in the window, oldest first (gaps included, zeroed). */
  daily: DayBucket[];
  byModel: ModelBucket[];
  byWorkspace: WorkspaceBucket[];
  totals: { prompt: number; completion: number; cached: number; sessions: number; activeDays: number };
  /** Longest run of consecutive active days. */
  streak: number;
  busiest: DayBucket | null;
}

/** Days spanned by the heatmap window (53 weeks ≈ a year, like the calendar
 *  grid it mirrors). Internal: callers read `daily.length`. */
const WINDOW_DAYS = 371;

/**
 * Bucket raw records into the heatmap window.
 *
 * `sessions` counts a session once per day it ran (a session's `updated_at` is
 * its LAST activity, so it can only ever land on its final day — the count is
 * "sessions that ended that day", which is what the store can actually prove).
 */
export function aggregate(records: UsageRecord[], now = Date.now()): UsageStats {
  const byDay = new Map<string, { sessions: number; tokens: number }>();
  const models = new Map<string, ModelBucket>();
  const workspaces = new Map<string, WorkspaceBucket>();
  const totals = { prompt: 0, completion: 0, cached: 0, sessions: 0, activeDays: 0 };

  for (const r of records) {
    totals.prompt += r.prompt;
    totals.completion += r.completion;
    totals.cached += r.cached;
    totals.sessions += 1;

    const key = dayKey(r.ts);
    const day = byDay.get(key) ?? { sessions: 0, tokens: 0 };
    day.sessions += 1;
    day.tokens += r.prompt + r.completion;
    byDay.set(key, day);

    const ws = r.workspace || "未命名工作区";
    const w = workspaces.get(ws) ?? { workspace: ws, prompt: 0, completion: 0, sessions: 0, lastTs: 0 };
    w.prompt += r.prompt;
    w.completion += r.completion;
    w.sessions += 1;
    w.lastTs = Math.max(w.lastTs, r.ts);
    workspaces.set(ws, w);

    const model = r.model || "未记录";
    const provider = r.provider || "";
    const m = models.get(`${provider}::${model}`) ?? { model, provider, prompt: 0, completion: 0, sessions: 0 };
    m.prompt += r.prompt;
    m.completion += r.completion;
    m.sessions += 1;
    models.set(`${provider}::${model}`, m);
  }

  // The window ends today (local) and reaches back WINDOW_DAYS.
  const daily: DayBucket[] = [];
  const end = new Date(now);
  end.setHours(0, 0, 0, 0);
  for (let i = WINDOW_DAYS - 1; i >= 0; i--) {
    const d = new Date(end);
    d.setDate(d.getDate() - i);
    const date = dayKey(d.getTime() / 1000);
    const hit = byDay.get(date);
    daily.push({ date, sessions: hit?.sessions ?? 0, tokens: hit?.tokens ?? 0 });
  }

  totals.activeDays = byDay.size;
  let streak = 0;
  let run = 0;
  for (const d of daily) {
    if (d.sessions > 0) {
      run += 1;
      streak = Math.max(streak, run);
    } else run = 0;
  }
  const busiest = daily.reduce<DayBucket | null>(
    (best, d) => (d.sessions > (best?.sessions ?? 0) ? d : best),
    null
  );

  return {
    daily,
    byModel: [...models.values()].sort(
      (a, b) => b.prompt + b.completion - (a.prompt + a.completion)
    ),
    byWorkspace: [...workspaces.values()].sort(
      (a, b) => b.prompt + b.completion - (a.prompt + a.completion)
    ),
    totals,
    streak,
    busiest: busiest && busiest.sessions > 0 ? busiest : null,
  };
}

/** Compact token count: 1_234 → `1.2k`, 1_234_567 → `1.2M`. */
export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}
