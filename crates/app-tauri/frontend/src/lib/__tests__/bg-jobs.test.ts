// The wording the 后台 tab and the jobs pill share.

import { describe, expect, test } from "vitest";
import type { BgEndReason, BgJob } from "@/features/chat/hooks/stream-view";
import { formatElapsed, jobElapsedMs, jobStateLabel, jobTone, summarizeJobs } from "../bg-jobs";

const job = (id: string, state: BgJob["state"], elapsed_ms = 0): BgJob => ({
  id,
  command: `cmd ${id}`,
  state,
  started_at_ms: 1,
  elapsed_ms,
  notify: true,
  has_own_sink: false,
});
const running = { kind: "running" } as const;
const done = (label: string, code: number | null, reason: BgEndReason = code == null ? "killed" : "exited") =>
  ({ kind: "done", label, code, reason }) as const;

describe("job tone", () => {
  test("a normal exit reads by its code", () => {
    expect(jobTone(job("a", done("exited(0)", 0)))).toBe("ok");
    expect(jobTone(job("a", done("exited(2)", 2)))).toBe("failed");
    expect(jobStateLabel(job("a", done("exited(2)", 2)))).toBe("退出码 2");
  });

  test("a kill is a stop, but a cap the job ran into is a failure", () => {
    expect(jobTone(job("a", done("killed(sig 9)", null, "killed")))).toBe("stopped");
    expect(jobTone(job("a", done("killed(timeout)", null, "timeout")))).toBe("failed");
    expect(jobTone(job("a", done("killed(log over 64 MiB)", null, "log_cap")))).toBe("failed");
    expect(jobStateLabel(job("a", done("killed(timeout)", null, "timeout")))).toBe("超时终止");
  });

  test("the reason decides, not the label's wording", () => {
    // The same signal label: ours is a stop, anyone else's is a crash.
    expect(jobTone(job("a", done("killed(sig 9)", null, "killed")))).toBe("stopped");
    expect(jobTone(job("a", done("killed(sig 9)", null, "signaled")))).toBe("failed");
    expect(jobStateLabel(job("a", done("killed(sig 9)", null, "signaled")))).toBe("异常终止");
    // Reworded caps still read right.
    expect(jobStateLabel(job("a", done("capped(wall clock)", null, "timeout")))).toBe("超时终止");
    expect(jobTone(job("a", done("status error: x", null, "error")))).toBe("failed");
  });
});

describe("job clock", () => {
  test("a running job ticks on from its snapshot; a settled one does not", () => {
    expect(jobElapsedMs(job("a", running, 5000), 1000, 4000)).toBe(8000);
    expect(jobElapsedMs(job("a", { kind: "paused" }, 5000), 1000, 4000)).toBe(5000);
    expect(jobElapsedMs(job("a", done("exited(0)", 0), 5000), 1000, 4000)).toBe(5000);
  });

  test("formats like the tool", () => {
    expect(formatElapsed(42_000)).toBe("42s");
    expect(formatElapsed(185_000)).toBe("3m05s");
    expect(formatElapsed(3_720_000)).toBe("1h02m");
  });
});

describe("pill summary", () => {
  test("no jobs, no pill", () => {
    expect(summarizeJobs([])).toBeNull();
  });

  test("live jobs come first, paused ones count as live", () => {
    const s = summarizeJobs([job("a", running), job("b", { kind: "paused" }), job("c", done("exited(1)", 1))]);
    expect(s).toEqual({ text: "2 个任务运行中", tone: "running" });
  });

  test("once settled, a failure outranks the successes", () => {
    const s = summarizeJobs([job("a", done("exited(0)", 0)), job("b", done("exited(1)", 1))]);
    expect(s).toEqual({ text: "1 个任务失败", tone: "failed" });
  });

  test("all settled cleanly — a stop is not a failure", () => {
    const s = summarizeJobs([job("a", done("exited(0)", 0)), job("b", done("killed(sig 9)", null))]);
    expect(s).toEqual({ text: "2 个任务已结束", tone: "ok" });
  });
});
