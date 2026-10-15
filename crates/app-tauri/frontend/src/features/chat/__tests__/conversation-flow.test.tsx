import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { ComposerBar } from "@/features/chat/components/composer-bar";
import { activeViewNow, initAgentStore, useActiveView, useAgentStore } from "@/stores/agent-store";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";
import { emitAgentEvent, tauri } from "@/test/tauri";
import type { UiEvent } from "@/types";

// Exercise real reducers/store/components. Rich Markdown/highlighting and
// browser layout have their own boundary; jsdom is not a frame-time benchmark.
vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
let dispose: () => void;
function Conversation() {
  const view = useActiveView();
  return <>
    <ChatStream view={view} sessionKey="/project:1" hasMore={false} loading={false} />
    <ComposerBar view={view} sessionKey="/project:1" />
  </>;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2025-01-01T00:00:00Z"));
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  dispose = initAgentStore();
});
afterEach(() => dispose());
async function deliver(...events: UiEvent[]) {
  await act(async () => {
    for (const event of events) emitAgentEvent({ root: "/project", session: 1, event });
    await vi.advanceTimersByTimeAsync(16);
  });
}

describe("conversation lifecycle through the event subscription", () => {
  test("send → reasoning → answer → finish preserves the draft and commits final text once", async () => {
    renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "请分析问题" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { Prompt: { text: "请分析问题" } } });
    await deliver(
      { UserPrompt: "请分析问题" }, { StateChanged: "Reasoning" },
      { ReasoningDelta: { text: "先检查状态", parent: null } },
    );
    expect(screen.getByText("请分析问题")).toBeInTheDocument();
    fireEvent.change(input, { target: { value: "跟进草稿" } });
    await deliver({ ReasoningDelta: { text: "与边界条件", parent: null } });
    expect(input).toHaveValue("跟进草稿");
    fireEvent.click(screen.getByRole("button", { name: /思考中/ }));
    expect(screen.getAllByText("先检查状态与边界条件").length).toBeGreaterThan(0);
    await deliver(
      { TextDelta: { text: "最终", parent: null } }, { TextDelta: { text: "答案", parent: null } },
      { AssistantMessage: "最终答案" }, { StateChanged: "Finished" },
    );
    expect(screen.getByText("最终答案")).toBeInTheDocument();
    expect(activeViewNow().items.filter((item) => item.kind === "assistant")).toHaveLength(1);
    expect(input).toHaveValue("跟进草稿");
    expect(screen.queryByRole("button", { name: "中断回复" })).not.toBeInTheDocument();
  });

  test("approval and question updates remain actionable during a streamed turn", async () => {
    renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    await deliver(
      { UserPrompt: "修改文件" }, { StateChanged: "Reasoning" },
      { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
      { ApprovalRequested: { request_id: 6, tool_name: "fuzzy_patch", diff: "preview", fuzzy: false } },
    );
    fireEvent.click(screen.getByRole("button", { name: "允许" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { ToolDecision: { request_id: 6, approved: true } } });
    await deliver(
      { ToolCallFinished: { name: "fuzzy_patch", ok: true, content: "写入完成" } },
      { QuestionAsked: { request_id: 8, question: "继续范围", options: [{ label: "当前文件" }] } },
    );
    fireEvent.click(screen.getByRole("button", { name: /当前文件/ }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { AnswerQuestion: { request_id: 8, answer: "当前文件" } } });
    await deliver({ QuestionAnswered: { request_id: 8 } }, { StateChanged: "Finished" });
    expect(screen.queryByText("继续范围")).not.toBeInTheDocument();
  });

  test("a loaded standalone notice cannot create an unloaded-history rail dot", async () => {
    act(() => useAgentStore.getState().loadView("/project", 1, {
      ...emptyView(), historyStart: 0, historyTotal: 3, turnTotal: 1, turnOffset: 0,
      items: [
        { kind: "system", standalone: true, text: "已切换模型", hi: 0 },
        { kind: "user", text: "你好", hi: 1, ts: 1 },
        { kind: "assistant", text: "已准备就绪", streaming: false, hi: 2 },
      ],
    }));
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    expect(screen.getByText("已切换模型")).toBeInTheDocument();
    const marks = container.querySelectorAll(".group\\/item");
    // top handle, system notice, user, answer, bottom handle — no fake dot
    expect(marks).toHaveLength(5);
  });
});
