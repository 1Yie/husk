// The subagent card: closed until the reader opens it, then its transcript in
// arrival order — reasoning drawn as the main chat's thinking row, a run of
// tool calls grouped under one rule, steers as notes, one status tone per state.

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

/** The card's header — the only thing on screen while the card is closed. */
const openCard = (user: ReturnType<typeof userEvent.setup>) =>
  user.click(screen.getByRole("button", { name: /review #1/ }));

describe("subagent card", () => {
  test("starts closed: the body waits for the reader", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <ToolChips
        rows={[agent({ detail: ["先看入口"], segments: [{ kind: "text", text: "先看入口" }] })]}
      />,
    );
    const header = screen.getByRole("button", { name: /review #1/ });
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("先看入口")).not.toBeInTheDocument();
    await openCard(user);
    expect(header).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("先看入口")).toBeInTheDocument();
  });

  test("reasoning stays collapsed until asked, then shows its text", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <ToolChips rows={[agent({ segments: [{ kind: "thinking", text: "先读入口，再看中间件" }] })]} />,
    );
    await openCard(user);
    // The thinking row is its own toggle, collapsed like the main turn's.
    expect(screen.queryByTestId("reasoning-body")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /思考中/ }));
    expect(screen.getByTestId("reasoning-body")).toHaveTextContent("先读入口，再看中间件");
  });

  test("consecutive tool segments render as one group", async () => {
    const user = userEvent.setup();
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
    await openCard(user);
    const groups = screen.getAllByRole("group", { name: "工具调用" });
    expect(groups).toHaveLength(1);
    expect(within(groups[0]).getByText("read_file")).toBeInTheDocument();
    expect(within(groups[0]).getByText("smart_grep")).toBeInTheDocument();
  });

  test("a muted steer reads as a note with its own label", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <ToolChips
        rows={[
          agent({
            segments: [{ kind: "text", text: "> 主代理回复：好的", muted: true }],
          }),
        ]}
      />,
    );
    await openCard(user);
    expect(screen.getByText("主代理回复")).toBeInTheDocument();
    expect(screen.getByText("好的")).toBeInTheDocument();
    // The raw blockquote never reaches the reader.
    expect(screen.queryByText("> 主代理回复：好的")).not.toBeInTheDocument();
  });

  test("reasoning, prose and tool runs sit in the order they happened", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <ToolChips
        rows={[
          agent({
            status: "done",
            children: [call("c1", "read_file")],
            segments: [
              { kind: "thinking", text: "先想一遍" },
              { kind: "text", text: "先看入口" },
              { kind: "tool", index: 0 },
              { kind: "thinking", text: "再想一遍" },
              { kind: "text", text: "结论" },
            ],
          }),
        ]}
      />,
    );
    await openCard(user);
    // Both thinking rows are blocks of the body, not one pinned above it.
    const thinking = screen.getAllByRole("button", { name: /思考过程/ });
    expect(thinking).toHaveLength(2);
    const order = [
      thinking[0],
      screen.getByText("先看入口"),
      screen.getByText("read_file"),
      thinking[1],
      screen.getByText("结论"),
    ];
    for (let i = 1; i < order.length; i++) {
      expect(
        order[i - 1].compareDocumentPosition(order[i]) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }
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
