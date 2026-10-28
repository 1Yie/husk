// Provider editor — the session-affinity dialect and the client User-Agent.
// Both live under 兼容性与请求头 and must round-trip through save_app_config,
// because a gateway that routes by client (OpenCode-style) looks for them.

import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { ModelPane } from "../index";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";

const providers = {
  OpenCodeGo: {
    kind: "openai_completions",
    base_url: "https://opencode.ai/inference/openai/v1",
    api_key: "••••",
    models: [],
    compat: { sendSessionAffinityHeaders: true, sessionAffinityFormat: "opencode" },
    userAgent: "husk-oc/1.0",
  },
};

function lastSaved(): any {
  return tauri.invoke.mock.calls
    .filter(([, args]) => args?.op === "save_app_config")
    .at(-1)?.[1]?.payload;
}

/** Render the pane and open the (single) provider's editor, expanded. */
async function openEditor(configProviders: any = providers) {
  const user = userEvent.setup();
  mockCommand("agent_session", () => ({ config: { providers: configProviders } }), "get_app_config");
  mockCommand("agent_session", () => ({ success: true }), "save_app_config");
  renderWithProviders(<ModelPane ov={null} reload={() => {}} />);
  await user.click(await screen.findByRole("button", { name: "编辑提供商" }));
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByRole("button", { name: /兼容性与请求头/ }));
  return { user, dialog };
}

test("technical provider fields keep literal text defaults and save URL and JSON unchanged", async () => {
  const { user, dialog } = await openEditor();
  const baseUrl = within(dialog).getByDisplayValue(providers.OpenCodeGo.base_url);
  const headers = within(dialog).getByPlaceholderText(/"X-Title": "my-app"/);
  for (const field of [baseUrl, within(dialog).getByPlaceholderText("max_tokens"), headers]) {
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "none");
    expect(field).toHaveAttribute("spellcheck", "false");
  }

  const url = "https://gw.example.invalid/v1";
  await user.clear(baseUrl);
  await user.type(baseUrl, url);
  await user.click(headers);
  await user.paste('{"X-Model":"qwen3-coder:32b"}');
  await user.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeTruthy());
  expect(lastSaved().providers.OpenCodeGo).toMatchObject({
    base_url: url,
    headers: { "X-Model": "qwen3-coder:32b" },
  });
});

test("a stored dialect and User-Agent come back unchanged after a save", async () => {
  const { user, dialog } = await openEditor();
  // The editor shows what the config holds — camelCase keys included, which is
  // how a hand-written config usually spells them.
  expect(
    within(dialog).getByRole("button", { name: "opencode（x-opencode-session）" }),
  ).toBeInTheDocument();
  expect(within(dialog).getByRole("button", { name: "发送" })).toBeInTheDocument();
  expect(within(dialog).getByPlaceholderText("my-coding-agent/1.0")).toHaveValue("husk-oc/1.0");

  await user.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeTruthy());
  const saved = lastSaved().providers.OpenCodeGo;
  expect(saved).toMatchObject({
    compat: { send_session_affinity_headers: true, session_affinity_format: "opencode" },
  });
  // The scalar keeps whichever spelling the file already used.
  expect(saved.userAgent ?? saved.user_agent).toBe("husk-oc/1.0");
});

test("clearing the dialect and the User-Agent drops both from the config", async () => {
  const { user, dialog } = await openEditor();
  await user.click(
    within(dialog).getByRole("button", { name: "opencode（x-opencode-session）" }),
  );
  await user.click(await screen.findByRole("menuitemradio", { name: "默认（通用组）" }));
  await user.clear(within(dialog).getByPlaceholderText("my-coding-agent/1.0"));

  await user.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeTruthy());
  const saved = lastSaved().providers.OpenCodeGo;
  // The header switch stays on — the row dials the format, it does not gate it.
  expect(saved.compat).toEqual({ send_session_affinity_headers: true });
  const uaKey = "userAgent" in saved ? "userAgent" : "user_agent";
  expect(saved[uaKey]).toBeNull();
});

test("a provider names its own session header through 自定义", async () => {
  const { user, dialog } = await openEditor();
  await user.click(
    within(dialog).getByRole("button", { name: "opencode（x-opencode-session）" }),
  );
  await user.click(await screen.findByRole("menuitemradio", { name: "自定义…" }));
  await user.type(
    within(dialog).getByPlaceholderText("x-acme-session, x-acme-conv"),
    "x-acme-session, x-acme-conv",
  );

  await user.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeTruthy());
  const saved = lastSaved().providers.OpenCodeGo;
  expect(saved.session_affinity_headers).toEqual(["x-acme-session", "x-acme-conv"]);
  // The explicit names replace the dialect — the two never compete.
  expect(saved.compat.session_affinity_format).toBeUndefined();
});

test("explicit header names load as 自定义 rather than as a dialect", async () => {
  const { dialog } = await openEditor({
    Acme: {
      kind: "openai_completions",
      base_url: "https://acme.invalid/v1",
      api_key: "••••",
      models: [],
      sessionAffinityHeaders: ["x-acme-session"],
    },
  });
  expect(within(dialog).getByRole("button", { name: "自定义…" })).toBeInTheDocument();
  expect(within(dialog).getByPlaceholderText("x-acme-session, x-acme-conv")).toHaveValue(
    "x-acme-session",
  );
});

test("Codex WebSocket kind and account headers survive provider editing", async () => {
  const { user, dialog } = await openEditor({
    Codex: {
      kind: "openai_codex_responses",
      base_url: "https://chatgpt.com/backend-api/codex",
      api_key: "••••",
      models: [],
      headers: { "chatgpt-account-id": "fixture-account", "x-gateway-custom": "kept" },
    },
  });
  expect(within(dialog).getByRole("button", { name: "OpenAI Codex (WebSocket)" })).toBeInTheDocument();
  await user.click(within(dialog).getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeTruthy());
  expect(lastSaved().providers.Codex).toMatchObject({
    kind: "openai_codex_responses",
    base_url: "https://chatgpt.com/backend-api/codex",
    headers: { "chatgpt-account-id": "fixture-account", "x-gateway-custom": "kept" },
  });
});
