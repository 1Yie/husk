// The delegate card's 报告 section: the kernel's `[subagent report — …]` header
// and a batch result's `── [i] ok · 3s ──` marker are metadata for the parent
// model, never report content — on the live stream and on a reopened log alike.

import { fireEvent, screen, within } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage, UiEvent } from "@/types";
import { renderWithProviders } from "@/test/render";

// Markdown is a separate surface; the card's own text is what these tests read.
vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

/** One card's wrapper — scoped by its header, since a batch draws one card per
 *  child and both stay open together. */
function cardOf(name: RegExp): HTMLElement {
  const card = screen.getByRole("button", { name }).parentElement;
  if (!card) throw new Error("card not rendered");
  return card;
}

/** The card's 报告 block: the FileText header row and the prose under it. */
function reportSection(name: RegExp): HTMLElement {
  const block = within(cardOf(name)).getByText("报告").parentElement;
  if (!block) throw new Error("报告 section not rendered");
  return block;
}

/** The tools phase and the child card both start closed. */
function openCard(name: RegExp) {
  fireEvent.click(screen.getByRole("button", { name: /次工具调用/ }));
  fireEvent.click(screen.getByRole("button", { name }));
}

/** The 报告 section — and the body prose above it, which is the same report on a
 *  card whose transcript never streamed — must read as the child's own answer. */
function expectCleanReport(name: RegExp, body: string) {
  const section = reportSection(name);
  expect(section.textContent).toContain(body);
  expect(section.textContent).not.toContain("[subagent report");
  expect(section.textContent).not.toContain("── [");
  expect(cardOf(name).textContent).not.toContain("[subagent report");
  expect(cardOf(name).textContent).not.toContain("── [");
}

describe("the 报告 section of a delegate card", () => {
  test("live single delegation: the header stays out of the report", () => {
    const view = applyEvents(emptyView(), [
      { UserPrompt: "委派子代理" },
      { ToolCallStarted: { name: "delegate", args_preview: "1 task" } },
      {
        ToolCallStarted: { name: "review #1", args_preview: "检查鉴权", parent: "delegate", child_id: 7 },
      },
      {
        ToolCallFinished: {
          name: "review #1",
          ok: true,
          content: "[subagent report — child_id 7 · 132 tool calls, 758s]\n结论：无问题",
          parent: "delegate",
          child_id: 7,
        },
      },
    ] as UiEvent[]);
    renderWithProviders(<ChatStream view={view} sessionKey="/project:1" />);
    openCard(/review #1/);
    expectCleanReport(/review #1/, "结论：无问题");
  });

  test("live batch: each child's card shows its own report, never its sibling's", () => {
    const view = applyEvents(emptyView(), [
      { UserPrompt: "并行委派" },
      { ToolCallStarted: { name: "delegate", args_preview: "2 tasks" } },
      { ToolCallStarted: { name: "subagent #1", args_preview: "看鉴权", parent: "delegate", child_id: 21 } },
      { ToolCallStarted: { name: "subagent #2", args_preview: "看性能", parent: "delegate", child_id: 22 } },
      {
        ToolCallFinished: {
          name: "subagent #1",
          ok: true,
          content: "[subagent report — child_id 21 · 2 tool calls, 3s]\n鉴权没问题",
          parent: "delegate",
          child_id: 21,
        },
      },
      {
        ToolCallFinished: {
          name: "subagent #2",
          ok: true,
          content: "[subagent report — child_id 22 · 1 tool calls, 5s]\n性能可以",
          parent: "delegate",
          child_id: 22,
        },
      },
    ] as UiEvent[]);
    renderWithProviders(<ChatStream view={view} sessionKey="/project:1" />);
    openCard(/subagent #1/);
    expectCleanReport(/subagent #1/, "鉴权没问题");
    expect(reportSection(/subagent #1/).textContent).not.toContain("性能可以");
  });

  test("history single delegation: the header stays out of the report", () => {
    const history: ChatMessage[] = [
      { role: "user", content: "委派子代理", ts: 10 },
      {
        role: "assistant",
        content: "",
        ts: 20,
        tool_calls: [{ id: "d1", type: "function", function: { name: "delegate", arguments: '{"task":"探索仓库"}' } }],
      },
      {
        role: "tool",
        content: "[subagent report — child_id 12 · 4 tool calls, 9s]\n结论：无问题",
        tool_call_id: "d1",
        ts: 30,
      },
    ];
    renderWithProviders(<ChatStream view={viewFromHistory(history)} sessionKey="/project:1" />);
    openCard(/subagent/);
    expectCleanReport(/subagent/, "结论：无问题");
  });

  test("history batch: the section marker and header stay out, the sibling's text too", () => {
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
    renderWithProviders(<ChatStream view={viewFromHistory(history)} sessionKey="/project:1" />);
    // The second child's card is the one whose section sat behind a marker.
    openCard(/subagent #2/);
    expectCleanReport(/subagent #2/, "性能可以");
    expect(reportSection(/subagent #2/).textContent).not.toContain("鉴权没问题");
  });
});
