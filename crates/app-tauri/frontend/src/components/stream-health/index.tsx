// Stream health chip — the kernel's UI-queue delivery counters, plus the
// webview's own IPC event rate.
//
// Why it exists: the kernel merges deltas into ~120-char events and the
// webview folds arrivals into one `setViews` per 16ms, so in the steady
// state this whole pipeline is nearly free. When that stops being true
// (a stalled consumer, a bursty provider, a backgrounded tab), the old
// symptom was "the reply looks wrong / the pill never stops" with nothing
// on screen to point at the cause. These counters make it visible:
//
//   sent       events enqueued as new entries
//   coalesced  deltas merged into the queue tail — text preserved
//   dropped    deltas actually discarded — text lost (control events are
//              never dropped, so this is always a real defect signal)
//   depth_peak queue high-water mark, in events
//
// Production renders only the `dropped` warning; dev also shows the full
// counter strip and the measured IPC rate.

import { useEffect, useRef, useState } from "react";
import { Activity, TriangleAlert } from "lucide-react";
import { TooltipSimple } from "@/components/ui/tooltip";
import { getUiStats, onAgentEvent } from "../../invoke/agent";
import type { UiStats } from "../../types";

/** Kernel counters, polled — they are cumulative atomics, so a slow poll
 * loses nothing. */
function useUiStats(intervalMs: number): UiStats | null {
  const [stats, setStats] = useState<UiStats | null>(null);
  useEffect(() => {
    let dead = false;
    const tick = () => {
      void getUiStats()
        .then((s) => {
          if (!dead) setStats(s);
        })
        .catch(() => {});
    };
    tick();
    const timer = setInterval(tick, intervalMs);
    return () => {
      dead = true;
      clearInterval(timer);
    };
  }, [intervalMs]);
  return stats;
}

/** Events/s arriving from the kernel — how loud the IPC hop actually is.
 * Dev-only: it costs one extra listener on `agent://event`. */
function useIpcRate(): number {
  const [rate, setRate] = useState(0);
  const countRef = useRef(0);
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let dead = false;
    const un = onAgentEvent(() => {
      countRef.current += 1;
    });
    const timer = setInterval(() => {
      if (dead) return;
      setRate(countRef.current);
      countRef.current = 0;
    }, 1000);
    return () => {
      dead = true;
      clearInterval(timer);
      un.then((f) => f());
    };
  }, []);
  return rate;
}

const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);

export function StreamHealth() {
  const stats = useUiStats(3000);
  const ipc = useIpcRate();
  if (!stats) return null;

  const lost = stats.dropped > 0;
  if (!lost && !import.meta.env.DEV) return null;

  const detail =
    `UI 事件队列：新增 ${stats.sent} · 合并 ${stats.coalesced} · ` +
    `丢弃 ${stats.dropped} · 峰值深度 ${stats.depth_peak}` +
    (import.meta.env.DEV ? ` · IPC ${ipc} 事件/s` : "");

  if (lost) {
    return (
      <TooltipSimple
        content={`${detail}——丢弃的都是文本增量（控制事件从不丢弃），说明 webview 消费落后于生成速度。`}
        side="bottom"
      >
        <span className="flex items-center gap-1 cursor-default text-amber-500 dark:text-amber-400">
          <TriangleAlert className="h-3 w-3" />
          Δ丢失 {fmt(stats.dropped)}
        </span>
      </TooltipSimple>
    );
  }

  return (
    <TooltipSimple content={detail} side="bottom">
      <span className="flex items-center gap-1 cursor-default">
        <Activity className="h-3 w-3" />
        队列 {fmt(stats.sent)} · 合并 {fmt(stats.coalesced)} · 峰值 {stats.depth_peak} · IPC {ipc}/s
      </span>
    </TooltipSimple>
  );
}
