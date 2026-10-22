import { describe, expect, test, vi } from "vitest";
import { applyChangesFeed, opensUserTurn, steerTextOf, viewFromHistory, viewFromHistoryChunked } from "../view-from-history";
import { SUMMARY_TOOLS } from "../apply-event";
import type { ChatMessage } from "@/types";

const history: ChatMessage[] = [
  { role: "system", content: "internal prompt" },
  { role: "user", content: "internal nudge", notice: "hidden" },
  { role: "system", content: "已切换模型", notice: "system" },
  { role: "user", content: "检查文件", ts: 100 },
  { role: "assistant", content: "", reasoning: "先读文件", ts: 150, duration_ms: 20,
    tool_calls: [{ id: "c1", type: "function", function: { name: "smart_read", arguments: '{"path":"src/a.ts"}' } }] },
  { role: "tool", content: "permission denied", tool_call_id: "c1", is_error: true, ts: 200, duration_ms: 30 },
  { role: "assistant", content: "部分答案\n\n*[cancelled by user]*", ts: 250 },
];

describe("persisted history replay", () => {
  test("internal context is hidden and user-visible notices stay standalone", () => {
    const view = viewFromHistory(history, undefined, 40);
    expect(view.items.map((item) => item.kind)).toEqual(["system", "user", "thinking", "tool", "assistant", "system"]);
    expect(view.items[0]).toMatchObject({ text: "已切换模型", standalone: true, hi: 42 });
    expect(view.items[1]).toMatchObject({ text: "检查文件", hi: 43, ts: 100 });
  });

  test("reasoning time, tool error and cancelled output survive reopening", () => {
    const view = viewFromHistory(history);
    expect(view.items[2]).toMatchObject({ kind: "thinking", text: "先读文件", done: true, startedAt: 130, ts: 150 });
    expect(view.items[3]).toMatchObject({ kind: "tool", name: "smart_read", args: "src/a.ts", ok: false, startedAt: 170 });
    expect(view.items[4]).toMatchObject({ text: "部分答案", streaming: false });
    expect(view.items[5]).toMatchObject({ kind: "system", text: "已被用户中断" });
  });

  test("an older snapshot without timing still replays and malformed cards are skipped", () => {
    const view = viewFromHistory([
      { role: "system", notice: "compacted", content: "broken json" },
      { role: "system", notice: "plan", content: "broken json" },
      { role: "user", content: "The user interrupted: 先停止修改" },
      { role: "assistant", content: "[call: smart_read {}]\n已读", reasoning: "推导" },
    ]);
    expect(view.items).toMatchObject([
      { kind: "user", text: "先停止修改", steer: true },
      { kind: "thinking", text: "推导", done: true },
      { kind: "assistant", text: "已读", streaming: false },
    ]);
  });

  test("a reopened session rebuilds its 改动 list from the write results", () => {
    const view = viewFromHistory([
      { role: "user", content: "改一下", ts: 100 },
      {
        role: "assistant",
        content: "",
        ts: 150,
        tool_calls: [
          {
            id: "c9",
            type: "function",
            function: { name: "fuzzy_patch", arguments: '{"path":"src/a.ts"}' },
          },
        ],
      },
      {
        role: "tool",
        tool_call_id: "c9",
        ts: 200,
        content:
          "patched src/a.ts (line 3)\n\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,3 @@\n-old\n+new\n+extra\n",
      },
    ]);
    expect(view.changes).toMatchObject([{ path: "src/a.ts", adds: 2, dels: 1 }]);
  });

  test("a real prompt is not marked as a steer", () => {
    const view = viewFromHistory([{ role: "user", content: "检查文件" }]);
    expect(view.items[0]).not.toHaveProperty("steer");
  });

  test("a tagged steer is read from its tag, and the prefix is stripped", () => {
    const view = viewFromHistory([
      { role: "user", content: "The user interrupted: 先别改 config", notice: "steer" },
    ]);
    expect(view.items).toMatchObject([{ kind: "user", text: "先别改 config", steer: true }]);
  });

  test("a hidden row that quotes the prefix stays hidden and is not a steer", () => {
    const view = viewFromHistory([
      { role: "user", content: "The user interrupted: nudge", notice: "hidden" },
    ]);
    expect(view.items).toEqual([]);
    expect(steerTextOf({ role: "user", content: "The user interrupted: x", notice: "hidden" })).toBeNull();
  });

  test("only prompts open a user turn", () => {
    expect(opensUserTurn({ role: "user", content: "检查文件" })).toBe(true);
    expect(opensUserTurn({ role: "user", content: "The user interrupted: x", notice: "steer" })).toBe(false);
    expect(opensUserTurn({ role: "user", content: "The user interrupted: x" })).toBe(false);
    expect(opensUserTurn({ role: "user", content: "nudge", notice: "hidden" })).toBe(false);
    expect(opensUserTurn({ role: "assistant", content: "ok" })).toBe(false);
  });

  test("persisted usage seeds the visible context meter", () => {
    const view = viewFromHistory([], {
      prompt: 120,
      completion: 30,
      context_window: 8000,
      cached: 40,
      cache_write: 12,
    });
    expect(view.usage).toEqual({
      prompt: 120,
      completion: 30,
      contextWindow: 8000,
      cachedTokens: 40,
      cacheWriteTokens: 12,
    });
  });

  test("persisted usage restores the context breakdown", async () => {
    const breakdown = {
      system: 10, tools: 5, user: 3, memory: 0, assistant: 4,
      reasoning: 6, tool_calls: 2, tool_results: 20, other: 0,
    };
    const usage = { prompt: 120, completion: 30, context_window: 8000, breakdown };
    expect(viewFromHistory([], usage).contextBreakdown).toEqual(breakdown);
    vi.useFakeTimers();
    const task = viewFromHistoryChunked([], usage);
    await vi.runAllTimersAsync();
    expect((await task).contextBreakdown).toEqual(breakdown);
    vi.useRealTimers();
  });

  test("time-sliced rebuild matches sync replay across multiple pages", async () => {
    vi.useFakeTimers();
    const messages = Array.from({ length: 60 }, (_, index): ChatMessage => ({ role: index % 2 ? "assistant" : "user", content: `message ${index}` }));
    const task = viewFromHistoryChunked(messages, undefined, 30);
    await vi.runAllTimersAsync();
    await expect(task).resolves.toEqual(viewFromHistory(messages, undefined, 30));
  });
});

describe("changes feed", () => {
  const page: ChatMessage[] = [
    { role: "user", content: "改第二个文件" },
    { role: "assistant", content: "",
      tool_calls: [{ id: "c2", type: "function", function: { name: "fuzzy_patch", arguments: '{"path":"b.ts"}' } }] },
    { role: "tool", tool_call_id: "c2",
      content: "patched b.ts (line 3)\n\n--- a/b.ts\n+++ b/b.ts\n@@ -1 +1 @@\n-old\n+new" },
  ];

  test("the whole log rebuilds files the mounted page no longer carries", () => {
    const view = viewFromHistory(page);
    expect(view.changes.map((c) => c.path)).toEqual(["b.ts"]);

    // `open` returns only the newest page; the earlier write lives only in the
    // feed — without it the panel reopens one file short.
    applyChangesFeed(view, [
      { tool: "fuzzy_patch", args: '{"path":"a.ts"}',
        content: "patched a.ts (line 1)\n\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-x\n+y" },
      { tool: "fuzzy_patch", args: '{"path":"b.ts"}',
        content: "patched b.ts (line 3)\n\n--- a/b.ts\n+++ b/b.ts\n@@ -1 +1 @@\n-old\n+new" },
    ]);
    expect(view.changes.map((c) => c.path).sort()).toEqual(["a.ts", "b.ts"]);
    // The feed replaces the page-derived list rather than stacking onto it.
    expect(view.changes.find((c) => c.path === "b.ts")?.patches).toHaveLength(1);
  });

  test("the artifacts dock is rebuilt from the same feed, not emptied by it", () => {
    const view = viewFromHistory([
      { role: "user", content: "做个 PPT" },
      { role: "assistant", content: "",
        tool_calls: [{ id: "o1", type: "function", function: { name: "office_create", arguments: '{"file":"q4/deck.pptx"}' } }] },
      { role: "tool", tool_call_id: "o1", content: "created q4/deck.pptx" },
    ]);
    expect(view.artifacts.map((a) => a.path)).toEqual(["q4/deck.pptx"]);

    applyChangesFeed(view, [
      { tool: "web_download", args: '{"path":"q4/assets/logo.png","url":"https://x/logo.png"}', content: "saved" },
      { tool: "office_create", args: '{"file":"q4/deck.pptx"}', content: "created q4/deck.pptx" },
    ]);
    expect(view.artifacts.map((a) => a.path)).toEqual(["q4/assets/logo.png", "q4/deck.pptx"]);
  });
});

test("the feed asks for every tool the summary parsers read", () => {
  for (const tool of ["fuzzy_patch", "apply_patch", "office_create", "web_download", "office_exec"]) {
    expect(SUMMARY_TOOLS).toContain(tool);
  }
});
