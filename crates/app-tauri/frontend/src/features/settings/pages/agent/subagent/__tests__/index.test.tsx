import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { SubagentPane } from "../index";
import type { AgentOverview, SubagentItem } from "@/lib/agent-ipc/sessions";
import { mockCommand, modelInfo, tauri } from "@/test/tauri";

const row = (over: Partial<SubagentItem> = {}): SubagentItem => ({
  name: "hunter",
  description: "手写的审查员",
  prompt: "HUNT",
  builtin: false,
  path: "/project/.husk/agents/hunter.md",
  global: false,
  readonly: true,
  ...over,
});

const overview = (subagents: SubagentItem[]): AgentOverview => ({
  instructions: "",
  model: modelInfo(),
  skills: [],
  plugins: [],
  subagents,
});

/** The dialog's own selects — 位置 has no aria-label, so it is reached by the
 *  label it shows. */
const scopeSelect = () => screen.getByRole("button", { name: "工作区" });
const permissionSelect = () => screen.getByRole("button", { name: "子代理权限" });

describe("subagent profile dialog", () => {
  test("creates with the chosen scope and permission", async () => {
    const user = userEvent.setup();
    mockCommand(
      "agent_session",
      () => ({ success: true, path: "/config/husk/agents/hunter.md" }),
      "create_subagent",
    );
    render(<SubagentPane ov={overview([])} reload={() => {}} />);

    await user.click(screen.getByRole("button", { name: "添加" }));
    await user.type(screen.getByPlaceholderText("reviewer"), "hunter");
    await user.type(screen.getByPlaceholderText("你是一个代码审查专家，专注于…"), "HUNT");

    // A new profile starts 只读; 可读写 is an explicit choice.
    expect(permissionSelect()).toHaveTextContent("只读");
    await user.click(permissionSelect());
    await user.click(await screen.findByRole("menuitemradio", { name: /可读写/ }));
    // A write-capable child runs headless — the dialog says so.
    expect(screen.getByText(/需要人工确认的操作会被直接拒绝/)).toBeInTheDocument();

    await user.click(scopeSelect());
    await user.click(await screen.findByRole("menuitemradio", { name: "全局" }));

    await user.click(screen.getByRole("button", { name: "创建" }));
    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
        op: "create_subagent",
        payload: {
          name: "hunter",
          description: "",
          scope: "global",
          prompt: "HUNT",
          model: "",
          thinking: "",
          readonly: false,
        },
      }),
    );
  });

  test("an untouched permission stays 只读", async () => {
    const user = userEvent.setup();
    mockCommand("agent_session", () => ({ success: true, path: "/x.md" }), "create_subagent");
    render(<SubagentPane ov={overview([])} reload={() => {}} />);

    await user.click(screen.getByRole("button", { name: "添加" }));
    await user.type(screen.getByPlaceholderText("reviewer"), "hunter");
    await user.type(screen.getByPlaceholderText("你是一个代码审查专家，专注于…"), "HUNT");
    await user.click(screen.getByRole("button", { name: "创建" }));

    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
        op: "create_subagent",
        payload: {
          name: "hunter",
          description: "",
          scope: "workspace",
          prompt: "HUNT",
          model: "",
          thinking: "",
          readonly: true,
        },
      }),
    );
  });

  test("editing reuses the manifest's scope and permission", async () => {
    const user = userEvent.setup();
    mockCommand(
      "agent_session",
      () => ({
        name: "hunter",
        description: "手写的审查员",
        prompt: "HUNT",
        scope: "global",
        model: null,
        thinking: null,
        builtin: false,
        readonly: false,
      }),
      "read_subagent",
    );
    mockCommand("agent_session", () => ({ success: true, path: "/x.md" }), "create_subagent");
    render(<SubagentPane ov={overview([row({ readonly: false, global: true })])} reload={() => {}} />);

    await user.click(screen.getByRole("button", { name: "编辑" }));
    expect(await screen.findByRole("button", { name: "子代理权限" })).toHaveTextContent("可读写");
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
        op: "create_subagent",
        payload: {
          name: "hunter",
          description: "手写的审查员",
          scope: "global",
          prompt: "HUNT",
          model: "",
          thinking: "",
          readonly: false,
        },
      }),
    );
  });
});
