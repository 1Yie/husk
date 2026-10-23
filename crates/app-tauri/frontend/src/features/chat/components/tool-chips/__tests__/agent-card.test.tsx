// The expanded subagent card: reasoning drawn as the main chat's thinking row,
// a run of tool calls grouped under one rule, steers as notes, and one status
// tone per state.

import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import { ToolChips, type ToolChipRow } from "../index";
import { renderWithProviders } from "@/test/render";

// Markdown parsing/highlighting is a separate surface; keep the reasoning
// body's mount identity so these tests stay about the card's own chrome.
vi.mock("@/features/chat/components/chat-stream/reasoning-body", () => ({
  ReasoningBody: ({ text }: { text: string }) => (
    <article data-testid="reasoning-body">{text}</article>
  ),
}));

const agent = (patch: Partial<ToolChipRow> = {}): ToolChipRow => ({
  id: "agent-1",
  kind: "agent",
  label: "review #1",
  chip: "检查鉴权分支",
  status: "running",
  ...patch,
});

const call = (id: string, label: string): ToolChipRow => ({
  id,
  label,
  chip: "src/auth.ts",
  status: "done",
});

describe("subagent card", () => {
  test("reasoning stays collapsed until asked, then shows its text", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ToolChips rows={[agent({ reasoning: "先读入口，再看中间件" })]} />);
    expect(screen.queryByTestId("reasoning-body")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /思考中/ }));
    expect(screen.getByTestId("reasoning-body")).toHaveTextContent("先读入口，再看中间件");
  });

  test("consecutive tool segments render as one group", () => {
    renderWithProviders(
      <ToolChips
        rows={[
          agent({
            status: "done",
            detail: ["先看看入口"],
            children: [call("c1", "read_file"), call("c2", "smart_grep")],
            segments: [
              { kind: "text", text: "先看看入口" },
              { kind: "tool", index: 0 },
              { kind: "tool", index: 1 },
            ],
          }),
        ]}
      />,
    );
    const groups = screen.getAllByRole("group", { name: "工具调用" });
    expect(groups).toHaveLength(1);
    expect(within(groups[0]).getByText("read_file")).toBeInTheDocument();
    expect(within(groups[0]).getByText("smart_grep")).toBeInTheDocument();
  });

  test("a muted steer reads as a note with its own label", () => {
    renderWithProviders(
      <ToolChips
        rows={[
          agent({
            segments: [{ kind: "text", text: "> 主代理回复：好的", muted: true }],
          }),
        ]}
      />,
    );
    expect(screen.getByText("主代理回复")).toBeInTheDocument();
    expect(screen.getByText("好的")).toBeInTheDocument();
    // The raw blockquote never reaches the reader.
    expect(screen.queryByText("> 主代理回复：好的")).not.toBeInTheDocument();
  });

  test("a failed child says 失败", () => {
    renderWithProviders(
      <ToolChips
        rows={[
          agent({
            status: "aborted",
            detail: ["工具返回了错误"],
            segments: [{ kind: "text", text: "工具返回了错误" }],
          }),
        ]}
      />,
    );
    expect(screen.getByText("失败")).toBeInTheDocument();
  });
});
