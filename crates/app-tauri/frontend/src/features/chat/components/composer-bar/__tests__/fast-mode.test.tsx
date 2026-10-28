import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ComposerBar } from "../index";
import { emptyView as sessionView } from "@/features/chat/hooks/stream-view";
import { renderWithProviders as render } from "@/test/render";
import { deferred, mockCommand, modelInfo, tauri } from "@/test/tauri";
import { CONFIG_CHANGED_EVENT } from "@/lib/agent-ipc/sessions";
import { toast } from "sonner";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
beforeEach(() => {
  mockCommand("agent_session", () => ({ workspace: [], session: [] }), "linked_roots");
});

function fastModelInfo() {
  const info = modelInfo();
  return {
    ...info,
    active_fast_mode: false,
    models: info.models.map((model) => ({ ...model, supports_fast_mode: true })),
  };
}

async function openPicker() {
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /当前模型: Model A/ }));
  expect(screen.queryByRole("switch", { name: "Fast 模式" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitemcheckbox", { name: "Fast 模式" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: /^思考/ }));
  return user;
}

describe("Fast mode", () => {
  test("toggles from the thinking header without changing the thinking tier or closing the menu", async () => {
    mockCommand("agent_session", fastModelInfo, "model_info");
    mockCommand("agent_session", () => ({ fast_mode: true }), "set_fast_mode");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />);
    const user = await openPicker();
    const fast = screen.getByRole("switch", { name: "Fast 模式" });
    expect(fast).not.toHaveAttribute("title");
    expect(fast).toHaveAttribute("aria-checked", "false");
    await user.click(fast);
    await waitFor(() => expect(fast).toHaveAttribute("aria-checked", "true"));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_fast_mode", id: 1, path: "/repo", payload: { enabled: true },
    });
    expect(screen.getByRole("slider", { name: "思考等级" })).toHaveAttribute("aria-valuenow", "1");
    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(tauri.invoke.mock.calls.some(([command]) => command === "agent_cmd")).toBe(false);
  });

  test("stays inside thinking even when the model has no thinking levels", async () => {
    const info = fastModelInfo();
    mockCommand("agent_session", () => ({
      ...info,
      models: info.models.map((model) => ({ ...model, reasoning: false, available_levels: [] })),
    }), "model_info");
    mockCommand("agent_session", () => ({ fast_mode: true }), "set_fast_mode");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />);
    const user = await openPicker();
    expect(screen.queryByRole("slider", { name: "思考等级" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Fast 模式" }));
    await waitFor(() => expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "true"));
  });

  test("does not infer Fast support from model names or from a different provider", async () => {
    const info = modelInfo();
    mockCommand("agent_session", () => ({
      ...info,
      models: [...info.models, { ...info.models[0], provider: "other", supports_fast_mode: true }],
    }), "model_info");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />);
    await openPicker();
    expect(screen.queryByRole("switch", { name: "Fast 模式" })).not.toBeInTheDocument();
  });

  test("waits for acknowledgement and leaves the previous state intact on error", async () => {
    mockCommand("agent_session", fastModelInfo, "model_info");
    const response = deferred<{ fast_mode: boolean }>();
    mockCommand("agent_session", () => response.promise, "set_fast_mode");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />);
    const user = await openPicker();
    const fast = screen.getByRole("switch", { name: "Fast 模式" });
    await user.click(fast);
    expect(fast).toHaveAttribute("aria-disabled", "true");
    expect(fast).toHaveAttribute("aria-checked", "false");
    response.reject(new Error("当前模型未配置 Fast"));
    await waitFor(() => expect(fast).not.toHaveAttribute("aria-disabled", "true"));
    expect(fast).toHaveAttribute("aria-checked", "false");
    expect(toast.error).toHaveBeenCalledWith(expect.stringContaining("当前模型未配置 Fast"));
  });

  test("a late acknowledgement cannot enable Fast in another session", async () => {
    mockCommand("agent_session", fastModelInfo, "model_info");
    const response = deferred<{ fast_mode: boolean }>();
    mockCommand("agent_session", () => response.promise, "set_fast_mode");
    const { rerender } = render(
      <ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />,
    );
    const user = await openPicker();
    await user.click(screen.getByRole("switch", { name: "Fast 模式" }));
    rerender(<ComposerBar view={sessionView()} workspaceRoot="/other" sessionKey="/other::1" />);
    response.resolve({ fast_mode: true });
    await waitFor(() => expect(screen.getByRole("switch")).not.toHaveAttribute("aria-disabled", "true"));
    expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false");
  });

  test("keyboard toggling stays in the menu and Escape dismisses it", async () => {
    mockCommand("agent_session", fastModelInfo, "model_info");
    mockCommand("agent_session", () => ({ fast_mode: true }), "set_fast_mode");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo::1" />);
    const user = await openPicker();
    const fast = screen.getByRole("switch", { name: "Fast 模式" });
    fast.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(fast).toHaveAttribute("aria-checked", "true"));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  test("restores the session preference and can turn it off", async () => {
    mockCommand("agent_session", () => ({ ...fastModelInfo(), active_fast_mode: true }), "model_info");
    mockCommand("agent_session", () => ({ fast_mode: false }), "set_fast_mode");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo:1" />);
    const user = await openPicker();
    const fast = screen.getByRole("switch", { name: "Fast 模式" });
    expect(fast).toHaveAttribute("aria-checked", "true");
    await user.click(fast);
    await waitFor(() => expect(fast).toHaveAttribute("aria-checked", "false"));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_fast_mode", id: 1, path: "/repo", payload: { enabled: false },
    });
  });

  test("removing model capability hides the switch without adding Fast to the outer menu", async () => {
    mockCommand("agent_session", () => ({ ...fastModelInfo(), active_fast_mode: true }), "model_info");
    render(<ComposerBar view={sessionView()} workspaceRoot="/repo" sessionKey="/repo:1" />);
    await openPicker();
    mockCommand("agent_session", () => ({ ...modelInfo(), active_fast_mode: true }), "model_info");
    act(() => window.dispatchEvent(new CustomEvent(CONFIG_CHANGED_EVENT)));
    await waitFor(() => expect(screen.queryByRole("switch")).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: /当前模型:/ })).not.toHaveTextContent("Fast");
  });
});
