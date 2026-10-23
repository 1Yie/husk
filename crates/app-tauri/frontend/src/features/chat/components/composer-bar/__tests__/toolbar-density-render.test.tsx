// The composer toolbar reads its own width and switches tiers: labels when the
// main column is wide, icons at 480px, and one folded mode button at 360px —
// the row that used to squeeze three pills until their labels vanished.

import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ComposerBar } from "../index";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders } from "@/test/render";
import { tauri } from "@/test/tauri";
import { useAgentStore } from "@/stores/agent-store";

/** The width every observed box reports; the toolbar must use its own. */
let reportedWidth = 720;

class WidthResizeObserver implements ResizeObserver {
  private callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }
  observe(target: Element) {
    const entry = { target, contentRect: { width: reportedWidth } as DOMRectReadOnly };
    this.callback([entry as ResizeObserverEntry], this);
  }
  unobserve() {}
  disconnect() {}
}

beforeEach(() => {
  reportedWidth = 720;
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  vi.stubGlobal("ResizeObserver", WidthResizeObserver);
});

/** Render the composer at one column width and wait for its model chip. */
async function composerAt(width: number) {
  reportedWidth = width;
  const user = userEvent.setup();
  renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
  await screen.findByRole("button", { name: /当前模型: Model A/ });
  return user;
}

describe("composer toolbar density", () => {
  test("a narrow row folds the mode pills into one menu button", async () => {
    const user = await composerAt(360);
    expect(screen.queryByRole("button", { name: "代理模式" })).toBeNull();
    expect(screen.queryByRole("button", { name: "权限模式" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "模式与权限" }));
    // Both groups stay one click away, each item naming itself in full.
    expect(await screen.findByRole("menuitem", { name: /构建/ })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /默认/ })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /计划/ })).toBeInTheDocument();

    // The folded menu drives the same handlers the pills always did.
    await user.click(screen.getByRole("menuitem", { name: /计划/ }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { SetAgentMode: { mode: "plan" } } });
  });

  test("a mid row keeps the mode icons and drops their labels", async () => {
    const user = await composerAt(480);
    // The label is gone from the row; the trigger still says the whole thing.
    expect(screen.queryByRole("button", { name: "代理模式" })).toBeNull();
    expect(screen.getByRole("button", { name: "代理模式：构建" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "权限模式：默认" })).toBeInTheDocument();
    expect(screen.queryByText("构建")).toBeNull();
    // The thinking chip keeps its word at this width — only the model name is
    // allowed to truncate, and the row still has room for the tier.
    expect(screen.getByRole("button", { name: /当前模型: Model A/ })).toHaveTextContent("Low");
    expect(screen.getByRole("button", { name: /当前模型: Model A/ })).toHaveTextContent("Model A");

    // An icon-only pill still opens its menu.
    await user.click(screen.getByRole("button", { name: "权限模式：默认" }));
    expect(await screen.findByRole("menuitem", { name: /不询问/ })).toBeInTheDocument();
  });

  test("a wide row shows the labelled pills", async () => {
    await composerAt(720);
    expect(screen.queryByRole("button", { name: "模式与权限" })).toBeNull();
    expect(screen.getByRole("button", { name: "代理模式" })).toHaveTextContent("构建");
    expect(screen.getByRole("button", { name: "权限模式" })).toHaveTextContent("默认");
    expect(screen.getByRole("button", { name: /当前模型: Model A/ })).toHaveTextContent("Low");
  });
});
