// The 统计 aggregation: distillation is its own channel.

import { describe, expect, test } from "vitest";
import { aggregate } from "../usage";
import type { UsageRecord } from "@/lib/agent-ipc/sessions";

const record = (over: Partial<UsageRecord> = {}): UsageRecord => ({
  session: 1,
  title: "t",
  workspace: "/w",
  // Inside the heatmap window, so the daily bucket lands on today.
  ts: Math.floor(Date.now() / 1000),
  prompt: 100,
  completion: 20,
  cached: 40,
  cache_write: 10,
  context_window: 1000,
  model: "m",
  provider: "p",
  ...over,
});

describe("usage aggregation", () => {
  test("distillation spend is summed apart from the conversation", () => {
    const stats = aggregate([
      record({ distill_calls: 2, distill_prompt: 300, distill_completion: 80 }),
      record({ session: 2, distill_calls: 1, distill_prompt: 150, distill_completion: 40 }),
    ]);
    expect(stats.totals).toMatchObject({
      prompt: 200,
      completion: 40,
      distillCalls: 3,
      distillPrompt: 450,
      distillCompletion: 120,
    });
    // The conversation buckets never include it.
    expect(stats.byModel[0]).toMatchObject({ prompt: 200, completion: 40 });
    expect(stats.byWorkspace[0]).toMatchObject({ prompt: 200, completion: 40 });
    expect(stats.daily.at(-1)?.tokens).toBe(240);
  });

  test("rows from a build without the channel read as zero", () => {
    const stats = aggregate([record()]);
    expect(stats.totals.distillCalls).toBe(0);
    expect(stats.totals.distillPrompt).toBe(0);
    expect(stats.totals.distillCompletion).toBe(0);
  });
});
