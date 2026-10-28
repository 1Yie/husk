import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import type { FastModeConfig } from "@/lib/agent-ipc";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import { fastModeDraft, validateFastMode } from "../fast-mode";
import { ModelPane } from "../index";

function config(fastMode?: FastModeConfig, kind = "openai_responses", fastKey = "fastMode") {
  return {
    providers: {
      Vendor: {
        kind,
        base_url: "https://gateway.example.invalid/v1",
        api_key: "••••",
        default_model: "model-a",
        headers: { "X-Provider": "keep" },
        fastMode: { mode: "custom", params: { vendor_fast: true } },
        modelOverrides: { "model-b": { fast_mode: { mode: "speed" }, params: { keep: true } } },
        models: [{
          id: "model-a",
          name: "Model A",
          reasoning: true,
          context_window: 128000,
          thinking_level_map: { high: "high" },
          params: { temperature: 0.7 },
          custom_vendor_setting: { keep: true },
          ...(fastMode ? { [fastKey]: fastMode } : {}),
        }, "model-b"],
      },
    },
    agent: { default_provider: "Vendor", default_model: "model-a", provider_chain: ["Vendor"] },
    unrelated: { keep: true },
  };
}

function lastSaved() {
  return tauri.invoke.mock.calls
    .filter(([, args]) => args?.op === "save_app_config")
    .at(-1)?.[1]?.payload as ReturnType<typeof config> | undefined;
}

async function openEditor(fastMode?: FastModeConfig, kind?: string, fastKey?: string) {
  const user = userEvent.setup();
  const initial = config(fastMode, kind, fastKey);
  mockCommand("agent_session", () => ({ config: initial }), "get_app_config");
  mockCommand("agent_session", () => ({ success: true }), "save_app_config");
  renderWithProviders(<ModelPane ov={null} reload={() => {}} />);
  await user.click(await screen.findByRole("button", { name: /Model A/ }));
  const dialog = await screen.findByRole("dialog");
  return { user, dialog, initial, fields: within(dialog) };
}

async function selectMode(user: ReturnType<typeof userEvent.setup>, label: string | RegExp) {
  await user.click(screen.getByRole("button", { name: "Fast 模式" }));
  await user.click(await screen.findByRole("menuitemradio", { name: label }));
}

async function save(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(lastSaved()).toBeDefined());
  return lastSaved()!;
}

test("Fast is model-level opt-in, not enabled by the provider block, and preserves unrelated config", async () => {
  const { user, fields, initial } = await openEditor();
  expect(fields.queryByRole("switch")).not.toBeInTheDocument();
  expect(fields.getByRole("button", { name: "Fast 模式" })).toHaveTextContent("不开启");
  await selectMode(user, "service_tier: priority");
  expect(fields.getByRole("button", { name: "Fast 模式" })).toHaveTextContent("service_tier: priority");
  const saved = await save(user);
  const originalModel = initial.providers.Vendor.models[0];
  expect(saved).toEqual({
    ...initial,
    providers: {
      Vendor: {
        ...initial.providers.Vendor,
        models: [{ ...(originalModel as object), fastMode: { mode: "priority" } }, "model-b"],
      },
    },
  });
});

test("Anthropic uses the explicit speed choice without a separate switch", async () => {
  const { user, fields } = await openEditor(undefined, "anthropic");
  expect(fields.getByRole("button", { name: "Fast 模式" })).toHaveTextContent("不开启");
  await selectMode(user, "speed: fast");
  expect(fields.getByRole("button", { name: "Fast 模式" })).toHaveTextContent("speed: fast");
  const saved = await save(user);
  expect(saved.providers.Vendor.models[0]).toMatchObject({
    fastMode: { mode: "speed" },
    reasoning: true,
    thinking_level_map: { high: "high" },
  });
});

test("the mode selector saves an exact same-provider model ID and reopens the saved settings", async () => {
  const { user, fields } = await openEditor({ mode: "priority" });
  await selectMode(user, "model");
  expect(fields.getByRole("button", { name: "保存" })).toBeDisabled();
  expect(lastSaved()).toBeUndefined();
  const id = fields.getByRole("textbox", { name: "快速版模型 ID" });
  await user.type(id, "  vendor-exact-fast-id  ");
  const saved = await save(user);
  expect(saved.providers.Vendor.models[0]).toMatchObject({
    id: "model-a",
    fastMode: { mode: "model", model: "vendor-exact-fast-id" },
  });
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await user.click(screen.getByRole("button", { name: /Model A/ }));
  expect(screen.getByRole("textbox", { name: "快速版模型 ID" })).toHaveValue("vendor-exact-fast-id");
});

test("custom params and string headers save and survive reopening without changing normal parameters", async () => {
  const { user, fields } = await openEditor({ mode: "priority" });
  await selectMode(user, "自定义参数");
  expect(fields.getByRole("button", { name: "保存" })).toBeDisabled();
  fireEvent.change(fields.getByRole("textbox", { name: "Fast 参数 JSON" }), {
    target: { value: '{"service_tier":"priority","nested":{"keep":[1,true]}}' },
  });
  fireEvent.change(fields.getByRole("textbox", { name: "Fast 请求头 JSON" }), {
    target: { value: '{"X-Fast":"enabled"}' },
  });
  const saved = await save(user);
  const expected = {
    mode: "custom",
    params: { service_tier: "priority", nested: { keep: [1, true] } },
    headers: { "X-Fast": "enabled" },
  };
  expect(saved.providers.Vendor.models[0]).toMatchObject({
    params: { temperature: 0.7 },
    fastMode: expected,
    reasoning: true,
  });
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await user.click(screen.getByRole("button", { name: /Model A/ }));
  expect(JSON.parse((screen.getByRole("textbox", { name: "Fast 参数 JSON" }) as HTMLTextAreaElement).value))
    .toEqual(expected.params);
  expect(JSON.parse((screen.getByRole("textbox", { name: "Fast 请求头 JSON" }) as HTMLTextAreaElement).value))
    .toEqual(expected.headers);
});

test("switching to speed keeps explicit JSON overrides and never modifies thinking", async () => {
  const overrides = { params: { vendor: true }, headers: { "X-Fast": "1" } };
  const { user } = await openEditor({ mode: "custom", ...overrides });
  await selectMode(user, "speed: fast");
  const saved = await save(user);
  expect(saved.providers.Vendor.models[0]).toMatchObject({
    fastMode: { mode: "speed", ...overrides },
    thinking_level_map: { high: "high" },
    reasoning: true,
  });
});

test("invalid JSON stays editable, blocks page save, and disabling removes the entire Fast block", async () => {
  const { user, fields, initial } = await openEditor({
    mode: "custom", params: { enabled: true }, headers: { "X-Fast": "1" },
  });
  const params = fields.getByRole("textbox", { name: "Fast 参数 JSON" });
  fireEvent.change(params, { target: { value: '{"partial":' } });
  expect(params).toHaveValue('{"partial":');
  expect(fields.getByRole("alert")).toHaveTextContent("有效的 JSON 对象");
  expect(fields.getByRole("button", { name: "保存" })).toBeDisabled();
  await user.click(fields.getByRole("button", { name: "保存" }));
  expect(lastSaved()).toBeUndefined();
  await selectMode(user, "不开启");
  expect(fields.getByRole("button", { name: "保存" })).toBeEnabled();
  const saved = await save(user);
  expect(saved.providers.Vendor.models[0]).not.toHaveProperty("fastMode");
  expect(saved.providers.Vendor.fastMode).toEqual(initial.providers.Vendor.fastMode);
  expect(saved.providers.Vendor.models[0]).toMatchObject({ params: { temperature: 0.7 } });
});

test("optional protocol fields stay collapsed and use the shared form disclosure", async () => {
  const { user, fields } = await openEditor({ mode: "priority" });
  expect(fields.queryByRole("textbox", { name: "Fast 参数 JSON" })).not.toBeInTheDocument();
  const advanced = fields.getByRole("button", { name: "高级参数" });
  expect(advanced).toHaveAttribute("aria-expanded", "false");
  await user.click(advanced);
  expect(advanced).toHaveAttribute("aria-expanded", "true");
  expect(fields.getByRole("textbox", { name: "Fast 参数 JSON" })).toBeInTheDocument();
  expect(fields.getByRole("textbox", { name: "Fast 请求头 JSON" })).toBeInTheDocument();
  await user.click(advanced);
  expect(advanced).toHaveAttribute("aria-expanded", "false");
});

test("invalid headers block saving until corrected; header-only custom configuration is supported", async () => {
  const { user, fields } = await openEditor({ mode: "priority" });
  await selectMode(user, "自定义参数");
  const headers = fields.getByRole("textbox", { name: "Fast 请求头 JSON" });
  fireEvent.change(headers, { target: { value: '{"X-Fast":true}' } });
  expect(fields.getByRole("button", { name: "保存" })).toBeDisabled();
  expect(fields.getByRole("alert")).toHaveTextContent("字符串");
  fireEvent.change(headers, { target: { value: '{"X-Fast":"true"}' } });
  const saved = await save(user);
  expect(saved.providers.Vendor.models[0]).toMatchObject({
    fastMode: { mode: "custom", headers: { "X-Fast": "true" } },
  });
});

test("fast_mode aliases round-trip without adding a duplicate fastMode and can be removed", async () => {
  const { user, fields, initial } = await openEditor({ mode: "speed" }, undefined, "fast_mode");
  expect(fields.getByRole("button", { name: "Fast 模式" })).not.toHaveTextContent("不开启");
  let saved = await save(user);
  expect(saved.providers.Vendor.models[0]).toHaveProperty("fast_mode", { mode: "speed" });
  expect(saved.providers.Vendor.models[0]).not.toHaveProperty("fastMode");
  expect(saved.providers.Vendor.modelOverrides).toEqual(initial.providers.Vendor.modelOverrides);
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await user.click(screen.getByRole("button", { name: /Model A/ }));
  await selectMode(user, "不开启");
  await user.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  saved = lastSaved()!;
  expect(saved.providers.Vendor.models[0]).not.toHaveProperty("fast_mode");
  expect(saved.providers.Vendor.models[0]).not.toHaveProperty("fastMode");
  expect(saved.providers.Vendor.modelOverrides).toEqual(initial.providers.Vendor.modelOverrides);
});

describe("Fast draft validation", () => {
  test.each(["{", "[]", "null", "true", "1", '"text"'])("rejects non-object params %s", (params) => {
    expect(validateFastMode({ ...fastModeDraft({ mode: "custom" }), params }).error).toBeTruthy();
  });
  test.each(["{", "[]", "null", '{"X":1}', '{"X":null}', '{"X":{}}'])("rejects invalid headers %s", (headers) => {
    expect(validateFastMode({ ...fastModeDraft({ mode: "priority" }), headers }).error).toBeTruthy();
  });
  test("requires a non-blank alternate ID and ignores incomplete drafts when disabled", () => {
    const draft = { ...fastModeDraft({ mode: "model" }), model: "  ", params: "{" };
    expect(validateFastMode(draft).error).toContain("模型 ID");
    expect(validateFastMode({ ...draft, enabled: false })).toEqual({});
  });
  test("does not infer a Fast capability or model ID from provider names", () => {
    expect(fastModeDraft(undefined, "MiniMax-highspeed")).toEqual({
      enabled: false, mode: "custom", model: "", params: "", headers: "",
    });
  });
  test("custom params and headers must not both be empty objects", () => {
    expect(validateFastMode({
      ...fastModeDraft({ mode: "custom" }), params: "{}", headers: "{}",
    }).error).toContain("非空");
  });
  test("Codex responses defaults to priority without automatically enabling Fast", () => {
    expect(fastModeDraft(undefined, "openai_codex_responses")).toMatchObject({
      enabled: false, mode: "priority",
    });
  });
  test.each(["fastMode", "fast_mode", "model"])("rejects app-only or identity parameter %s", (key) => {
    expect(validateFastMode({
      ...fastModeDraft({ mode: "custom" }), params: JSON.stringify({ [key]: "value" }),
    }).error).toBeTruthy();
  });
  test.each([
    { "": "value" },
    { "Bad Header": "value" },
    { "Bad:Header": "value" },
    { "X-Fast": "value\r\nInjected: true" },
    { "X-Fast": "value\n" },
    { "X-Fast": "value\u0000" },
    { "X-Fast": "value\u007f" },
  ])("rejects invalid HTTP header %j", (headers) => {
    expect(validateFastMode({
      ...fastModeDraft({ mode: "priority" }), headers: JSON.stringify(headers),
    }).error).toContain("请求头");
  });
});
