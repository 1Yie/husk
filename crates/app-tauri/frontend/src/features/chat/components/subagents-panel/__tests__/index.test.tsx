// The 子代理 view: one card per delegated child. The ones still out take a
// steer; the ones that ended keep their verdict and report.

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { toast } from "sonner";
import { SubagentsList } from "@/features/chat/components/subagents-panel";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { agentRowsByChild } from "@/features/chat/hooks/agent-rows";
import { extractChildren, type SessionChild } from "@/features/chat/hooks/running-children";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import { useAgentStore } from "@/stores/agent-store";
import { TestResizeObserver } from "@/test/dom";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import { WAITING_STEER_HINT } from "@/lib/subagents";
import type { ChatMessage, UiEvent } from "@/types";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
// Markdown is a separate surface; the transcript's own text is what these read.
vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));

const child = (childId: number, name: string, task: string, extra: Partial<SessionChild> = {}): SessionChild => ({
  childId,
  name,
  task,
  status: "running",
  ...extra,
});

/** The 子代理 body as the dock mounts it — the clear addresses this session. */
const list = (children: SessionChild[]) => (
  <SubagentsList children={children} root="/project" session={1} />
);

const dismissed = () => [...(useAgentStore.getState().dismissedChildren.get("/project:1") ?? [])];

describe("子代理 view", () => {
  test("gives each running child a card: its name, its task and 运行中", () => {
    renderWithProviders(
      list([child(1, "review #1", "检查鉴权"), child(2, "review #2", "看性能")]),
    );
    expect(screen.getByText("review #1")).toBeInTheDocument();
    expect(screen.getByText("检查鉴权")).toBeInTheDocument();
    expect(screen.getByText("review #2")).toBeInTheDocument();
    expect(screen.getAllByText("运行中")).toHaveLength(2);
  });

  test("a task the 3-line clamp cuts opens in a scrollable panel, not a native title", async () => {
    const long = Array.from({ length: 40 }, (_, i) => `第 ${i + 1} 步：把鉴权中间件从头看完`).join("\n");
    // Testing Library collapses whitespace when matching, the box does not.
    const shown = long.split("\n").join(" ");
    renderWithProviders(list([child(8, "review #1", long)]));
    const line = screen.getByText(shown);
    // jsdom lays nothing out: declare the box as cut, then let the panel measure.
    Object.defineProperties(line, {
      scrollHeight: { configurable: true, value: 240 },
      clientHeight: { configurable: true, value: 54 },
    });
    act(() => TestResizeObserver.resize(line));

    // The webview's own `title` bubble is what painted over the whole window.
    expect(line).not.toHaveAttribute("title");
    fireEvent.pointerEnter(line);
    fireEvent.pointerMove(line);

    await waitFor(() => expect(screen.getAllByText(shown)).toHaveLength(2));
    const [, panel] = screen.getAllByText(shown);
    expect(panel.className).toContain("overflow-y-auto");
    expect(panel.className).toContain("max-h-[min(20rem,");
  });

  test("a task measured before its font settled re-measures on hover", async () => {
    const long = Array.from({ length: 40 }, (_, i) => `第 ${i + 1} 步：把鉴权中间件从头看完`).join("\n");
    const shown = long.split("\n").join(" ");
    renderWithProviders(list([child(10, "review #1", long)]));
    const line = screen.getByText(shown);
    // The first measurement read the fallback font as fitting; nothing resizes
    // the box once the real font lands, so the observer never fires again.
    expect(screen.getAllByText(shown)).toHaveLength(1);
    Object.defineProperties(line, {
      scrollHeight: { configurable: true, value: 240 },
      clientHeight: { configurable: true, value: 54 },
    });
    fireEvent.pointerEnter(line);
    fireEvent.pointerMove(line);

    await waitFor(() => expect(screen.getAllByText(shown)).toHaveLength(2));
  });

  test("a task that fits stays quiet on hover", async () => {
    renderWithProviders(list([child(9, "review #1", "检查鉴权")]));
    const task = screen.getByText("检查鉴权");
    expect(task).not.toHaveAttribute("title");
    fireEvent.pointerEnter(task);
    fireEvent.pointerMove(task);
    await waitFor(() => expect(screen.getAllByText("检查鉴权")).toHaveLength(1));
  });

  test("a steer typed against a child reaches the kernel", async () => {
    const user = userEvent.setup();
    renderWithProviders(list([child(3, "review #1", "检查鉴权")]));
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", {
      cmd: { SteerChild: { child_id: 3, text: "先看中间件" } },
    });
    // The kernel echoes the steer back as a `subagent:steer` custom event, so
    // the box empties and nothing is drawn locally.
    await waitFor(() => expect(box).toHaveValue(""));
  });

  test("a rejected steer is reported and the draft kept", async () => {
    mockCommand("agent_cmd", () => Promise.reject(new Error("session gone")));
    const user = userEvent.setup();
    renderWithProviders(list([child(3, "review #1", "检查鉴权")]));
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("插话失败：Error: session gone"));
    expect(box).toHaveValue("先看中间件");
  });

  test("a parked child shows its question and says the steer waits", () => {
    renderWithProviders(
      list([child(4, "review #1", "检查鉴权", { status: "waiting", waiting: "用哪条分支？" })]),
    );
    expect(screen.getByText("等待主代理回复")).toBeInTheDocument();
    expect(screen.getByText("用哪条分支？")).toBeInTheDocument();
    expect(screen.getByText(WAITING_STEER_HINT)).toBeInTheDocument();
  });

  test("a finished child stays: its verdict, its report on demand, no steer", () => {
    renderWithProviders(
      list([child(5, "review #1", "检查鉴权", { status: "done", report: "结论：无问题" })]),
    );
    expect(screen.getByText("已完成")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("插话…")).not.toBeInTheDocument();
    expect(screen.queryByText("结论：无问题")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "报告" }));
    expect(screen.getByText("结论：无问题")).toBeInTheDocument();
  });

  test("a live report reaches the panel without the kernel's header", () => {
    // The card keeps the raw result; only what the panel draws is stripped.
    const view = applyEvents(emptyView(), [
      { UserPrompt: "委派" },
      { ToolCallStarted: { name: "review #1", args_preview: "检查鉴权", parent: "delegate", child_id: 5 } },
      {
        ToolCallFinished: {
          name: "review #1",
          ok: true,
          content: "[subagent report — child_id 5 · 132 tool calls, 758s]\n结论：无问题",
          parent: "delegate",
          child_id: 5,
        },
      },
    ] as UiEvent[]);
    const { container } = renderWithProviders(list(extractChildren(view)));
    fireEvent.click(screen.getByRole("button", { name: "报告" }));
    expect(screen.getByText("结论：无问题")).toBeInTheDocument();
    expect(container.textContent).not.toContain("[subagent report");
  });

  test("a batch result's marker and header stay out of every panel report", () => {
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
    const { container } = renderWithProviders(list(extractChildren(viewFromHistory(history))));
    for (const button of screen.getAllByRole("button", { name: "报告" })) fireEvent.click(button);
    expect(screen.getByText("鉴权没问题")).toBeInTheDocument();
    expect(screen.getByText("性能可以")).toBeInTheDocument();
    expect(container.textContent).not.toContain("[subagent report");
    expect(container.textContent).not.toContain("── [");
  });

  test("failed and aborted children read as such", () => {
    renderWithProviders(
      list([child(6, "a #1", "x", { status: "failed", report: "超时" }), child(7, "b #2", "y", { status: "aborted" })]),
    );
    expect(screen.getByText("失败")).toBeInTheDocument();
    expect(screen.getByText("已中断")).toBeInTheDocument();
  });

  test("children still out lead; the ended ones follow under 已结束, newest first", () => {
    const { container } = renderWithProviders(
      list([
        child(1, "old #1", "x", { status: "done" }),
        child(2, "live #2", "y"),
        child(3, "new #3", "z", { status: "done" }),
      ]),
    );
    const names = [...container.querySelectorAll("[data-child-status] .font-mono")].map((n) => n.textContent);
    expect(names).toEqual(["live #2", "new #3", "old #1"]);
    expect(screen.getByText("已结束")).toBeInTheDocument();
  });
});

describe("工作过程", () => {
  // A running child with its own card: one streamed sentence, then a tool call
  // still out. The card is what the page builds from the stream items.
  const working = () => {
    const rows = agentRowsByChild([
      {
        kind: "tool",
        name: "review #1",
        args: "检查鉴权",
        parent: "delegate",
        childId: 7,
        childIndex: 0,
        hi: 3,
        segments: [
          { kind: "text", text: "先看入口文件" },
          { kind: "tool", index: 0 },
        ],
      },
      { kind: "tool", name: "smart_grep", args: "pattern: auth", parent: "review #1", childId: 7, childIndex: 0, hi: 3 },
    ]);
    return rows;
  };

  test("collapsed by default: the glance shows the running tool, not the transcript", () => {
    renderWithProviders(
      <SubagentsList children={[child(7, "review #1", "检查鉴权")]} childRows={working()} root="/project" session={1} />,
    );
    const toggle = screen.getByRole("button", { name: /工作过程/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("先看入口文件")).not.toBeInTheDocument();
    expect(screen.getByText(/1 次调用/)).toBeInTheDocument();
  });

  test("expanding shows the child's text and its tool call", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <SubagentsList children={[child(7, "review #1", "检查鉴权")]} childRows={working()} root="/project" session={1} />,
    );
    await user.click(screen.getByRole("button", { name: /工作过程/ }));
    expect(screen.getByRole("button", { name: /工作过程/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("先看入口文件")).toBeInTheDocument();
    expect(screen.getByText("smart_grep")).toBeInTheDocument();
  });

  test("an ended child's report sits in the transcript once expanded", async () => {
    const user = userEvent.setup();
    const done = agentRowsByChild([
      {
        kind: "tool",
        name: "review #1",
        args: "检查鉴权",
        parent: "delegate",
        childId: 9,
        content: "[subagent report — child_id 9 · 1 tool calls, 2s]\n结论：无问题",
        ok: true,
        childIndex: 0,
        hi: 4,
      },
    ]);
    renderWithProviders(
      <SubagentsList
        children={[child(9, "review #1", "检查鉴权", { status: "done", report: "结论：无问题" })]}
        childRows={done}
        root="/project"
        session={1}
      />,
    );
    expect(screen.queryByText("结论：无问题")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /工作过程/ }));
    // The body and the 报告 section both carry the report for a card that never
    // streamed its answer — the same deliverable shown twice, as the chat card does.
    expect(screen.getByText("报告")).toBeInTheDocument();
    expect(screen.getAllByText("结论：无问题").length).toBeGreaterThan(0);
  });

  test("a feed-only child keeps its report under 报告", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <SubagentsList children={[child(6, "a #1", "x", { status: "done", report: "结论：只剩报告" })]} root="/project" session={1} />,
    );
    await user.click(screen.getByRole("button", { name: "报告" }));
    expect(screen.getByText("结论：只剩报告")).toBeInTheDocument();
  });

  test("a live child with no card shows no 工作过程 section", () => {
    renderWithProviders(list([child(5, "review #1", "检查鉴权")]));
    expect(screen.queryByRole("button", { name: /工作过程/ })).not.toBeInTheDocument();
  });
});

describe("清除已结束", () => {
  beforeEach(() => useAgentStore.setState({ dismissedChildren: new Map() }));

  const mixed = () => [
    child(1, "live #1", "跑着"),
    child(2, "parked #2", "等着", { status: "waiting", waiting: "用哪条分支？" }),
    child(3, "done #3", "做完了", { status: "done", report: "结论：无问题" }),
    child(4, "failed #4", "炸了", { status: "failed", report: "超时" }),
    child(5, "stopped #5", "断了", { status: "aborted" }),
  ];

  test("clears every ended child and keeps the ones still out", () => {
    renderWithProviders(list(mixed()));
    fireEvent.click(screen.getByRole("button", { name: "清除已结束" }));
    // The running and the parked child are the ones a steer can still reach.
    expect(dismissed()).toEqual(["c5", "c4", "c3"]);
  });

  test("nothing to clear, no button", () => {
    renderWithProviders(list(mixed().slice(0, 2)));
    expect(screen.queryByRole("button", { name: "清除已结束" })).toBeNull();
  });
});
