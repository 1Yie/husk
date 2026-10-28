import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, test, vi } from "vitest";
import { ApiKeyInput } from "../api-key-input";
import { ModelPane } from "../index";
import { deferred, mockCommand, tauri } from "@/test/tauri";
import { renderWithProviders } from "@/test/render";

const MASK = "\u2022\u2022\u2022\u2022";
const TEST_KEY = "test-provider-key";

function Editor({ initial = "", provider }: { initial?: string; provider?: string }) {
  const [value, setValue] = useState(initial);
  return <ApiKeyInput value={value} onChange={setValue} provider={provider} />;
}

describe("API Key visibility", () => {
  test("toggles a pasted key without changing its contents or requesting a stored key", async () => {
    const user = userEvent.setup();
    render(<Editor />);
    const input = screen.getByLabelText("API Key");
    const expectLiteralEditing = () => {
      expect(input).toHaveAttribute("autocorrect", "off");
      expect(input).toHaveAttribute("autocapitalize", "none");
      expect(input).toHaveAttribute("spellcheck", "false");
      expect(input).toHaveAttribute("autocomplete", "off");
    };
    input.focus();
    await user.paste(TEST_KEY);
    expect(input).toHaveAttribute("type", "password");
    expect(input).toHaveValue(TEST_KEY);
    expectLiteralEditing();
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(input).toHaveAttribute("type", "text");
    expect(input).toHaveValue(TEST_KEY);
    expect(input).toHaveFocus();
    expectLiteralEditing();
    await user.click(screen.getByRole("button", { name: "隐藏 API Key" }));
    expect(input).toHaveAttribute("type", "password");
    expect(input).toHaveValue(TEST_KEY);
    expectLiteralEditing();
    expect(tauri.invoke).not.toHaveBeenCalled();
  });

  test("shows a saved placeholder and only fetches the value when the eye is clicked", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    mockCommand("agent_session", () => TEST_KEY, "get_provider_api_key");
    render(<ApiKeyInput value={MASK} onChange={onChange} provider="test" />);
    const input = screen.getByLabelText("API Key");
    expect(input).toHaveValue("");
    expect(input).toHaveAttribute("placeholder", "已保存，输入新 Key 可替换");
    expect(tauri.invoke).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(input).toHaveAttribute("type", "text");
    expect(input).toHaveValue(TEST_KEY);
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "get_provider_api_key", payload: { provider: "test" },
    });
    expect(onChange).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "隐藏 API Key" }));
    expect(input).toHaveValue("");
    expect(input).toHaveAttribute("type", "password");
    expect(onChange).not.toHaveBeenCalled();
  });

  test("does not overwrite a replacement typed while the stored key is loading", async () => {
    const user = userEvent.setup();
    const request = deferred<string>();
    mockCommand("agent_session", () => request.promise, "get_provider_api_key");
    render(<Editor initial={MASK} provider="test" />);
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(screen.getByRole("button", { name: "显示 API Key" })).toBeDisabled();
    const input = screen.getByLabelText("API Key");
    fireEvent.change(input, { target: { value: "replacement-key" } });
    await act(async () => request.resolve(TEST_KEY));
    expect(input).toHaveValue("replacement-key");
    expect(input).toHaveAttribute("type", "password");
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(input).toHaveValue("replacement-key");
    expect(input).toHaveAttribute("type", "text");
    expect(tauri.invoke).toHaveBeenCalledTimes(1);
  });

  test("discards the previous provider's pending reveal and resets to hidden", async () => {
    const user = userEvent.setup();
    const request = deferred<string>();
    mockCommand("agent_session", () => request.promise, "get_provider_api_key");
    const { rerender } = render(<ApiKeyInput value={MASK} onChange={() => {}} provider="first" />);
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    rerender(<ApiKeyInput value={MASK} onChange={() => {}} provider="second" />);
    await act(async () => request.resolve(TEST_KEY));
    expect(screen.getByLabelText("API Key")).toHaveValue("");
    expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "显示 API Key" })).toBeEnabled();
  });

  test("a failed reveal remains hidden and can be retried", async () => {
    const user = userEvent.setup();
    mockCommand("agent_session", () => Promise.reject(new Error("read failed")), "get_provider_api_key");
    render(<Editor initial={MASK} provider="test" />);
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(screen.getByLabelText("API Key")).toHaveValue("");
    expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "显示 API Key" })).toBeEnabled();
    mockCommand("agent_session", () => TEST_KEY, "get_provider_api_key");
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(screen.getByLabelText("API Key")).toHaveValue(TEST_KEY);
  });

  test("environment references are shown as configured instead of fetching or replacing them", async () => {
    const user = userEvent.setup();
    render(<Editor initial="env:TEST_PROVIDER_KEY" provider="test" />);
    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(screen.getByLabelText("API Key")).toHaveValue("env:TEST_PROVIDER_KEY");
    expect(tauri.invoke).not.toHaveBeenCalled();
  });
});

test("viewing a stored key then saving a renamed provider preserves the stored-key sentinel", async () => {
  const user = userEvent.setup();
  mockCommand("agent_session", () => ({
    path: "/test/config.toml",
    config: {
      active_provider: "test",
      providers: {
        test: { kind: "anthropic", base_url: "https://example.invalid", api_key: MASK, models: [] },
      },
    },
  }), "get_app_config");
  mockCommand("agent_session", () => TEST_KEY, "get_provider_api_key");
  mockCommand("agent_session", () => ({ success: true }), "save_app_config");
  renderWithProviders(<ModelPane ov={null} reload={() => {}} />);
  await user.click(await screen.findByRole("button", { name: "编辑提供商" }));
  await user.click(screen.getByRole("button", { name: "显示 API Key" }));
  expect(screen.getByLabelText("API Key")).toHaveValue(TEST_KEY);
  fireEvent.change(screen.getByPlaceholderText("openai / anthropic / …"), { target: { value: "renamed" } });
  await user.click(screen.getByRole("button", { name: /^保存$/ }));
  await waitFor(() => {
    const saved = tauri.invoke.mock.calls.find(([, args]) => args?.op === "save_app_config");
    expect(saved?.[1]?.payload).toMatchObject({
      active_provider: "renamed",
      providers: { renamed: { api_key: MASK, renamed_from: "test" } },
    });
  });
});
