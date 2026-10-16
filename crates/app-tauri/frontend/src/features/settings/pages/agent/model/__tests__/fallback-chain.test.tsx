import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { ModelPane } from "../index";
import { renderWithProviders } from "@/test/render";
import { deferred, mockCommand, tauri } from "@/test/tauri";

const providers = {
  alpha: { kind: "anthropic", base_url: "https://example.invalid/alpha", api_key: "••••", models: [] },
  beta: { kind: "openai_responses", base_url: "https://example.invalid/beta", api_key: "••••", models: [] },
  gamma: { kind: "gemini", base_url: "https://example.invalid/gamma", api_key: "env:TEST_KEY", models: [] },
};

function lastSaved() {
  return tauri.invoke.mock.calls.filter(([, args]) => args?.op === "save_app_config").at(-1)?.[1]?.payload;
}

test("the chain is above providers and adding, reordering and removing preserve the rest of the config", async () => {
  const user = userEvent.setup();
  const pending = deferred<{ success: boolean }>();
  mockCommand("agent_session", () => ({
    config: { providers, active_provider: "gamma", fallback_chain: ["alpha", "beta"] },
  }), "get_app_config");
  mockCommand("agent_session", () => pending.promise, "save_app_config");
  renderWithProviders(<ModelPane ov={null} reload={() => {}} />);

  const card = await screen.findByRole("region", { name: "模型降级链" });
  await within(card).findByRole("button", { name: "下移 alpha" });
  expect(card.compareDocumentPosition(screen.getByText("模型提供商")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await user.click(within(card).getByRole("button", { name: "下移 alpha" }));
  expect(lastSaved()).toMatchObject({
    fallback_chain: ["beta", "alpha"], active_provider: "gamma", providers,
  });
  expect(within(card).getByRole("button", { name: "添加备用提供商" })).toBeDisabled();
  expect(within(card).getByRole("button", { name: "移除 beta" })).toBeDisabled();

  await act(async () => pending.resolve({ success: true }));
  const names = () => within(card).getAllByRole("listitem").map((row) => row.querySelector("[title]")?.textContent);
  expect(names()).toEqual(["beta", "alpha"]);
  expect(within(card).getByRole("button", { name: "上移 beta" })).toBeDisabled();
  expect(within(card).getByRole("button", { name: "下移 alpha" })).toBeDisabled();

  mockCommand("agent_session", () => ({ success: true }), "save_app_config");
  await user.click(within(card).getByRole("button", { name: "移除 beta" }));
  await waitFor(() => expect(names()).toEqual(["alpha"]));
  expect(lastSaved()).toMatchObject({ fallback_chain: ["alpha"], providers });

  await user.click(within(card).getByRole("button", { name: "添加备用提供商" }));
  expect(screen.queryByRole("menuitemradio", { name: "alpha" })).not.toBeInTheDocument();
  await user.click(await screen.findByRole("menuitemradio", { name: "gamma" }));
  await waitFor(() => expect(names()).toEqual(["alpha", "gamma"]));
  expect(lastSaved()).toMatchObject({ fallback_chain: ["alpha", "gamma"], active_provider: "gamma", providers });
});

test("a failed reorder keeps the saved order and retries using the existing camelCase config key", async () => {
  const user = userEvent.setup();
  mockCommand("agent_session", () => ({
    config: { providers, fallbackChain: ["missing", "beta"] },
  }), "get_app_config");
  mockCommand("agent_session", () => Promise.reject(new Error("write failed")), "save_app_config");
  renderWithProviders(<ModelPane ov={null} reload={() => {}} />);

  const card = await screen.findByRole("region", { name: "模型降级链" });
  expect(await within(card).findByText("提供商已不存在")).toBeInTheDocument();
  await user.click(within(card).getByRole("button", { name: "上移 beta" }));
  await waitFor(() => expect(within(card).getByRole("button", { name: "上移 beta" })).toBeEnabled());
  expect(within(card).getByRole("button", { name: "上移 missing" })).toBeDisabled();

  mockCommand("agent_session", () => ({ success: true }), "save_app_config");
  await user.click(within(card).getByRole("button", { name: "上移 beta" }));
  await waitFor(() => expect(within(card).getByRole("button", { name: "上移 beta" })).toBeDisabled());
  expect(lastSaved()).toMatchObject({ fallbackChain: ["beta", "missing"], providers });
  expect(lastSaved()).not.toHaveProperty("fallback_chain");
});
