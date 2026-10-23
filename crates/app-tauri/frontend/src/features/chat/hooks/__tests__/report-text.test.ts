// A report as the UI shows it: the kernel's header and a batch result's section
// marker are stripped at display time only — the row keeps the raw text, so the
// model's own context and the persisted log are untouched.

import { describe, expect, test } from "vitest";
import { reportBody } from "@/features/chat/hooks/report-text";
import { extractChildren } from "@/features/chat/hooks/running-children";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage } from "@/types";

const HEADER = "[subagent report — child_id 7 · 132 tool calls, 758s]";

describe("reportBody", () => {
  test("drops the kernel's header line", () => {
    expect(reportBody(`${HEADER}\n结论：无问题`)).toBe("结论：无问题");
  });

  test("drops a batch section marker in front of the header", () => {
    expect(reportBody(`── [0] ok · 3s ──\n${HEADER}\n鉴权没问题`)).toBe("鉴权没问题");
  });

  test("the same wording inside a report is the child's own text", () => {
    const body = `结论：无问题\n${HEADER}`;
    expect(reportBody(body)).toBe(body);
  });

  test("text that is not a report is untouched", () => {
    expect(reportBody("编译失败")).toBe("编译失败");
    expect(reportBody("")).toBe("");
  });
});

describe("a report's raw text", () => {
  test("stays on the tool row while the view shows the body under it", () => {
    const history: ChatMessage[] = [
      { role: "user", content: "并行委派", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [
          { id: "d2", type: "function", function: { name: "delegate", arguments: '{"tasks":["看鉴权","看性能"]}' } },
        ],
      },
      {
        role: "tool",
        content:
          "[delegate — 2 children]\n" +
          "── [0] ok · 3s ──\n[subagent report — child_id 21 · 2 tool calls, 3s]\n鉴权没问题\n" +
          "── [1] ok · 5s ──\n[subagent report — child_id 22 · 1 tool calls, 5s]\n性能可以",
        tool_call_id: "d2",
        ts: 30,
      },
    ];
    const view = viewFromHistory(history);
    const row = view.items.find((it) => it.kind === "tool" && it.name === "delegate");
    // The row keeps exactly what the kernel wrote — header and markers included.
    if (row?.kind === "tool") {
      expect(row.content).toContain("[subagent report");
      expect(row.content).toContain("── [0] ok");
    }
    // The 子代理 view shows only each child's own body.
    expect(extractChildren(view).map((c) => c.report)).toEqual(["鉴权没问题", "性能可以"]);
  });
});
