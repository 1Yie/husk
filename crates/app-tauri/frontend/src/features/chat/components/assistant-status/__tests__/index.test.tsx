import { act, fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import { AssistantStatus, formatElapsed } from "../index";
import { renderWithProviders } from "@/test/render";

// Markdown parsing/highlighting is a separate surface; retain its mount
// identity here so this test catches the former 16k subtree replacement.
vi.mock("@/features/chat/components/chat-stream/reasoning-body", () => ({
  ReasoningBody: ({ text }: { text: string }) => <article data-testid="reasoning-body">{text}</article>,
}));

describe("thinking status interactions", () => {
  test("null has no UI and an empty status cannot be expanded", () => {
    const { rerender, container } = renderWithProviders(<AssistantStatus mode={null} thinkingText="" />);
    expect(container).toBeEmptyDOMElement();
    rerender(<AssistantStatus mode="reply" thinkingText="" />);
    expect(screen.getByRole("button")).toBeDisabled();
  });

  test("reasoning expands on request and keeps its node across 16k characters", async () => {
    const user = userEvent.setup();
    const { rerender } = renderWithProviders(<AssistantStatus mode="thinking" thinkingText={"a".repeat(15000)} />);
    expect(screen.queryByTestId("reasoning-body")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button"));
    const body = screen.getByTestId("reasoning-body");
    rerender(<AssistantStatus mode="thinking" thinkingText={"a".repeat(18000)} />);
    expect(screen.getByTestId("reasoning-body")).toBe(body);
    expect(body.textContent).toHaveLength(18000);
    await user.click(screen.getByRole("button"));
    expect(screen.queryByTestId("reasoning-body")).not.toBeInTheDocument();
  });

  test("only the active orb mounts and settled thinking has none", () => {
    const { rerender } = renderWithProviders(<AssistantStatus mode="thinking" thinkingText="检查条件" />);
    expect(screen.getAllByRole("img")).toHaveLength(1);
    rerender(<AssistantStatus mode="tools" thinkingText="" />);
    expect(screen.getAllByRole("img")).toHaveLength(1);
    rerender(<AssistantStatus mode="thought" thinkingText="完成检查" />);
    expect(screen.queryAllByRole("img")).toHaveLength(0);
  });

  test("elapsed time banks completed passes and cleans up its timer", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2025-01-01T00:00:00Z"));
    const { rerender, unmount } = renderWithProviders(<AssistantStatus mode="thinking" thinkingText="分析" startedAt={Date.now()} />);
    act(() => vi.advanceTimersByTime(3000));
    expect(screen.getByText("· 3s")).toBeInTheDocument();
    rerender(<AssistantStatus mode="thought" thinkingText="分析" />);
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.getByText("· 3s")).toBeInTheDocument();
    rerender(<AssistantStatus mode="thinking" thinkingText="迟到的思考" />);
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.getByText("· 5s")).toBeInTheDocument();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  test("replayed durations are static and tool details toggle independently", () => {
    renderWithProviders(<AssistantStatus mode="tools_done" thinkingText="" doneLabel="3 次工具调用" elapsedMs={62000}>
      <div>工具结果</div>
    </AssistantStatus>);
    expect(screen.getByText("· 1:02")).toBeInTheDocument();
    expect(screen.queryByText("工具结果")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByText("工具结果")).toBeVisible();
  });
});

test.each([
  [-1000, "0s"], [999, "0s"], [20000, "20s"], [60000, "1:00"],
  [3599000, "59:59"], [3601000, "1:00:01"],
])("formatElapsed(%i) = %s", (ms, expected) => expect(formatElapsed(ms)).toBe(expected));
