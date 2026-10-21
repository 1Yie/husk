// Background-job wording shared by the 后台 dock tab, the composer pill and the
// notification rules — one place decides what a job's state reads as.

import type { BgJob } from "@/features/chat/hooks/stream-view";

export type JobTone = "running" | "paused" | "ok" | "failed" | "stopped";

/** How a job reads at a glance. A normal non-zero exit is a failure; a kill
 *  on request (the user, the model, the session ending) is a stop — but the
 *  caps a job hit by misbehaving (timeout, runaway log) and a signal nobody
 *  sent (a crash) are failures. */
export function jobTone(job: BgJob): JobTone {
  const st = job.state;
  if (st.kind === "running") return "running";
  if (st.kind === "paused") return "paused";
  switch (st.reason) {
    case "exited":
      return st.code === 0 ? "ok" : "failed";
    case "killed":
      return "stopped";
    default:
      return "failed";
  }
}

/** The state as a short Chinese label for the row. */
export function jobStateLabel(job: BgJob): string {
  const st = job.state;
  if (st.kind === "running") return "运行中";
  if (st.kind === "paused") return "已暂停";
  switch (st.reason) {
    case "exited":
      return st.code === 0 ? "已完成" : `退出码 ${st.code ?? "?"}`;
    case "killed":
      return "已终止";
    case "timeout":
      return "超时终止";
    case "log_cap":
      return "日志过大终止";
    case "signaled":
      return "异常终止";
    default:
      return st.label;
  }
}

/** Run time now: a running job's clock ticks on from the snapshot, a paused
 *  or finished one stays where the snapshot left it. */
export function jobElapsedMs(job: BgJob, snapshotAt: number, now: number): number {
  if (job.state.kind !== "running" || snapshotAt <= 0) return job.elapsed_ms;
  return job.elapsed_ms + Math.max(0, now - snapshotAt);
}

/** `42s`, `3m05s`, `1h02m` — the tool's own format. */
export function formatElapsed(ms: number): string {
  const secs = Math.floor(ms / 1000);
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m${String(secs % 60).padStart(2, "0")}s`;
  return `${Math.floor(secs / 3600)}h${String(Math.floor((secs % 3600) / 60)).padStart(2, "0")}m`;
}

export interface JobsSummary {
  /** What the pill says, or `null` when there is nothing to show. */
  text: string;
  tone: "running" | "failed" | "ok";
}

/** The composer pill's one line. Live jobs come first — something is still
 *  happening; once everything settled, a failure outranks the successes. */
export function summarizeJobs(jobs: readonly BgJob[]): JobsSummary | null {
  if (jobs.length === 0) return null;
  const live = jobs.filter((j) => j.state.kind === "running" || j.state.kind === "paused").length;
  if (live > 0) return { text: `${live} 个任务运行中`, tone: "running" };
  const failed = jobs.filter((j) => jobTone(j) === "failed").length;
  if (failed > 0) return { text: `${failed} 个任务失败`, tone: "failed" };
  return { text: `${jobs.length} 个任务已结束`, tone: "ok" };
}
