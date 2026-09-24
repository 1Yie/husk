// 活跃对话热力图 — a GitHub-contribution-style calendar of the days the agent
// worked.
//
// Matches the contribution graph's geometry rather than inventing one: 53
// week-columns × 7 day-rows, 10px cells with 3px gaps and a 2px radius, no
// border on empty days, month labels above the column where a month starts and
// weekday labels down the left. Hand-rolled CSS grid — the shape is fixed, so a
// charting dependency would cost more than the ~60 lines it replaces.

import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { SURFACE_ACTIVITY_EMPTY } from "@/lib/theme";
import type { DayBucket } from "@/features/settings/usage";

/** 53 weeks, the span the contribution graph uses. */
const WEEKS = 53;

/** Shading steps. Level 0 is the neutral "nothing happened" square; 1-4 ramp
 *  the activity colour, the way GitHub ramps its greens. */
const LEVELS = [
  // Empty cells are a flat neutral in BOTH themes — see SURFACE_ACTIVITY_EMPTY.
  SURFACE_ACTIVITY_EMPTY,
  "bg-[color-mix(in_srgb,var(--husk-activity)_22%,transparent)]",
  "bg-[color-mix(in_srgb,var(--husk-activity)_45%,transparent)]",
  "bg-[color-mix(in_srgb,var(--husk-activity)_70%,transparent)]",
  "bg-[var(--husk-activity)]",
];

/** Only every other weekday is labelled, as GitHub does — a label per row would
 *  crowd the 10px grid. Rows are Sunday-first. */
const DAY_LABELS: Record<number, string> = { 1: "一", 3: "三", 5: "五" };

function levelOf(value: number, max: number): number {
  if (value <= 0) return 0;
  if (max <= 1) return 4;
  const ratio = value / max;
  if (ratio > 0.75) return 4;
  if (ratio > 0.5) return 3;
  if (ratio > 0.25) return 2;
  return 1;
}

interface Props {
  daily: DayBucket[];
  metric?: "sessions" | "tokens";
}

export function ActivityHeatmap({ daily, metric = "sessions" }: Props) {
  const [hover, setHover] = useState<DayBucket | null>(null);
  const value = (d: DayBucket) => (metric === "sessions" ? d.sessions : d.tokens);
  const unit = metric === "sessions" ? "个会话" : "tokens";

  const { grid, months, max } = useMemo(() => {
    const byDate = new Map(daily.map((d) => [d.date, d]));
    const max = daily.reduce((m, d) => Math.max(m, value(d)), 0);

    // The last column is the week containing the newest day in `daily`; walk
    // back 52 more weeks so the grid always ends flush with today's row.
    const last = daily.length ? new Date(`${daily[daily.length - 1].date}T00:00:00`) : new Date();
    const lastDow = last.getDay();
    const start = new Date(last);
    start.setDate(start.getDate() - (lastDow + (WEEKS - 1) * 7));

    const grid: (DayBucket | null)[][] = [];
    const months: (string | null)[] = [];
    let prevMonth = -1;
    for (let w = 0; w < WEEKS; w++) {
      const col: (DayBucket | null)[] = [];
      for (let d = 0; d < 7; d++) {
        const cur = new Date(start);
        cur.setDate(start.getDate() + w * 7 + d);
        const key = `${cur.getFullYear()}-${String(cur.getMonth() + 1).padStart(2, "0")}-${String(cur.getDate()).padStart(2, "0")}`;
        // Days after the newest record still occupy their slot (the shape must
        // stay rectangular) but carry nothing to show.
        col.push(cur > last ? null : (byDate.get(key) ?? { date: key, sessions: 0, tokens: 0 }));
      }
      grid.push(col);
      const first = col.find(Boolean);
      const m = first ? new Date(`${first.date}T00:00:00`).getMonth() : prevMonth;
      // A label only where the month changes, and never in the first column
      // (it would be clipped by the weekday gutter).
      months.push(w > 0 && m !== prevMonth ? `${m + 1}月` : null);
      prevMonth = m;
    }
    return { grid, months, max };
  }, [daily, metric]);

  const total = daily.reduce((n, d) => n + value(d), 0);

  return (
    <div className="flex flex-col gap-2">
      {/* GitHub's geometry: a fixed 7-row grid, columns stretched to fill the
          card, 3px gaps, 2px radius, no border on empty cells. */}
      <div className="flex gap-2">
        {/* Weekday gutter — labels aligned to rows 2/4/6 (Mon/Wed/Fri). */}
        <div
          className="grid shrink-0 text-[9px] leading-none text-neutral-400"
          style={{ gridTemplateRows: "repeat(7, minmax(0, 1fr))", rowGap: 3, marginTop: 15 }}
        >
          {Array.from({ length: 7 }).map((_, d) => (
            <span key={d} className="flex items-center">
              {DAY_LABELS[d] ?? ""}
            </span>
          ))}
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
          {/* Month strip — same 53-column track as the grid below it. */}
          <div
            className="grid h-[12px] text-[9px] leading-none text-neutral-400"
            style={{ gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`, columnGap: 3 }}
          >
            {months.map((m, i) => (
              <span key={i} className="relative">
                {m && <span className="absolute left-0 top-0 whitespace-nowrap">{m}</span>}
              </span>
            ))}
          </div>

          <div
            className="grid"
            style={{ gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`, columnGap: 3 }}
          >
            {grid.map((col, wi) => (
              <div key={wi} className="grid" style={{ gridTemplateRows: "repeat(7, minmax(0, 1fr))", rowGap: 3 }}>
                {col.map((cell, di) =>
                  cell === null ? (
                    <span key={di} className="aspect-square w-full" />
                  ) : (
                    <span
                      key={cell.date}
                      onMouseEnter={() => setHover(cell)}
                      onMouseLeave={() => setHover(null)}
                      title={`${cell.date} · ${cell.sessions} 个会话 · ${cell.tokens} tokens`}
                      className={cn(
                        "aspect-square w-full rounded-[2px] transition-colors",
                        LEVELS[levelOf(value(cell), max)]
                      )}
                    />
                  )
                )}
              </div>
            ))}
          </div>
        </div>
      </div>

      <div className="flex items-center justify-between text-[11px] text-neutral-500">
        <span className="tabular-nums">
          {hover
            ? `${hover.date} · ${hover.sessions} 个会话 · ${hover.tokens} tokens`
            : `近一年 ${total} ${unit}`}
        </span>
        <span className="flex items-center gap-1">
          少
          {LEVELS.map((c, i) => (
            <span key={i} className={cn("h-[9px] w-[9px] rounded-[2px]", c)} />
          ))}
          多
        </span>
      </div>
    </div>
  );
}
