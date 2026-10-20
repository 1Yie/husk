import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/appearance", () => ({ useCurrencySymbol: () => "$" }));
vi.mock("@/components/window-controls", () => ({ WindowControls: () => null }));
vi.mock("@/features/chat/components/stream-health", () => ({ StreamHealth: () => null }));
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { TitleBar } from "../index";
import { emptyView } from "@/features/chat/hooks/stream-view";

function viewWith() {
  const v = emptyView();
  v.usage = { prompt: 100_000, completion: 2_000, contextWindow: 200_000, cachedTokens: 90_000, cacheWriteTokens: 0 };
  v.toksPerSec = 42;
  v.items = [
    { kind: "user", text: "hi" },
    { kind: "tool", name: "read", args: "{}", ok: true },
    { kind: "tool", name: "bash", args: "{}", ok: false },
    { kind: "assistant", text: "done", streaming: false },
  ];
  return v;
}

function renderBar(view = viewWith()) {
  return render(
    <TooltipProvider>
      <TitleBar title="t" view={view} contextWindowHint={200_000} gitInfo={{ branch: "main", dirty: 2 }} />
    </TooltipProvider>,
  );
}

describe("TitleBar stat chips", () => {
  it("summarises activity, tokens and context on the bar", () => {
    renderBar();
    expect(screen.getByRole("button", { name: "生成活动" }).textContent).toContain("1 步 · 2 调用");
    const tokens = screen.getByRole("button", { name: "Token 用量" }).textContent;
    expect(tokens).toContain("102K tok");
    expect(tokens).toContain("缓存 90%");
    expect(screen.getByRole("button", { name: "上下文窗口" }).textContent).toContain("50%");
  });

  it("opens the detail card on click", async () => {
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "Token 用量" }));
    expect(await screen.findByText("未缓存输入")).toBeTruthy();
    expect(screen.getByText("90,000 tok")).toBeTruthy();
  });

  it("keeps a sub-100% cache rate from rounding up to 100%", () => {
    const v = viewWith();
    v.usage = { ...v.usage, prompt: 100_000, cachedTokens: 99_999 };
    renderBar(v);
    expect(screen.getByRole("button", { name: "Token 用量" }).textContent).toContain("缓存 99.99%");
    fireEvent.click(screen.getByRole("button", { name: "Token 用量" }));
    expect(screen.getByText("99.99%")).toBeTruthy();
  });

  it("lists failed tool calls in the activity card", async () => {
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "生成活动" }));
    expect(await screen.findByText("工具失败")).toBeTruthy();
  });
});

/** jsdom's PointerEvent drops `pointerType`; React's enter/leave derive
 * from over/out, so dispatch those with the field set by hand. */
function mousePointer(type: "pointerover" | "pointerout") {
  const e = new MouseEvent(type, { bubbles: true, relatedTarget: document.body });
  Object.defineProperty(e, "pointerType", { value: "mouse" });
  return e;
}

describe("context card", () => {
  it("scales the estimated breakdown to the reported prompt", async () => {
    const v = viewWith();
    v.contextBreakdown = {
      system: 10, tools: 10, user: 5, memory: 0, assistant: 5,
      reasoning: 10, tool_calls: 10, tool_results: 50, other: 0,
    };
    renderBar(v);
    fireEvent.click(screen.getByRole("button", { name: "上下文窗口" }));
    expect(await screen.findByText("工具结果")).toBeTruthy();
    // 50% of 100K reported prompt tokens.
    expect(screen.getByText("~50K")).toBeTruthy();
    expect(screen.getByText("思考过程")).toBeTruthy();
    expect(screen.queryByText("记忆召回")).toBeNull();
  });

  it("falls back to the cache split without a breakdown", async () => {
    renderBar();
    fireEvent.click(screen.getByRole("button", { name: "上下文窗口" }));
    expect(await screen.findByText("缓存前缀")).toBeTruthy();
  });
});

describe("StatPopover focus", () => {
  it("does not ring the chip when a hover card closes", async () => {
    renderBar();
    const chip = screen.getByRole("button", { name: "Token 用量" });
    fireEvent(chip, mousePointer("pointerover"));
    await screen.findByText("未缓存输入");
    fireEvent(chip, mousePointer("pointerout"));
    await waitFor(() => expect(screen.queryByText("未缓存输入")).toBeNull());
    expect(document.activeElement).not.toBe(chip);
  });

  it("returns focus to the chip on Esc", async () => {
    renderBar();
    const chip = screen.getByRole("button", { name: "Token 用量" });
    fireEvent.click(chip);
    await screen.findByText("未缓存输入");
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByText("未缓存输入")).toBeNull());
    expect(document.activeElement).toBe(chip);
  });
});
