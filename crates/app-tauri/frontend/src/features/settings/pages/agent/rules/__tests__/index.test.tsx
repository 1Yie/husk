import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";
import { RulesPane } from "../index";
import type { AgentOverview, RuleItem } from "@/lib/agent-ipc/sessions";
import { mockCommand, modelInfo, tauri } from "@/test/tauri";

const row = (over: Partial<RuleItem> = {}): RuleItem => ({
  name: "api-conventions",
  description: "接口约定",
  globs: [],
  always: true,
  path: "/project/.husk/rules/api-conventions.md",
  global: false,
  ...over,
});

const overview = (rules: RuleItem[]): AgentOverview => ({
  instructions: "",
  model: modelInfo(),
  skills: [],
  plugins: [],
  subagents: [],
  rules,
});

const GLOB_PLACEHOLDER = "src/**/*.ts, docs/*.md";

/** The title + tags line of a rule's row — the scope/mode tags live there,
 *  which the scope tabs would otherwise shadow by name. */
const rowOf = (name: string) => screen.getByText(name).parentElement;

describe("rules pane", () => {
  test("lists rules with scope tags, folded globs and tab counts", () => {
    render(
      <RulesPane
        ov={overview([
          row(),
          row({
            name: "styles",
            description: "样式约定",
            globs: ["src/**/*.css", "web/*.css", "docs/*.css"],
            always: false,
            global: true,
            path: "/home/.config/husk/rules/styles.md",
          }),
        ])}
        reload={() => {}}
      />,
    );

    expect(screen.getByText("api-conventions")).toBeInTheDocument();
    expect(rowOf("api-conventions")).toHaveTextContent("工作区");
    expect(rowOf("api-conventions")).toHaveTextContent("始终生效");
    expect(rowOf("styles")).toHaveTextContent("全局");
    expect(rowOf("styles")).toHaveTextContent("按文件匹配");
    // Only the first two globs show; the third folds into +1.
    expect(screen.getByText("src/**/*.css")).toBeInTheDocument();
    expect(screen.getByText("web/*.css")).toBeInTheDocument();
    expect(screen.queryByText("docs/*.css")).not.toBeInTheDocument();
    expect(screen.getByText("+1")).toBeInTheDocument();

    expect(screen.getByRole("button", { name: /全部/ })).toHaveTextContent("2");
    expect(screen.getByRole("button", { name: /工作区/ })).toHaveTextContent("1");
    expect(screen.getByRole("button", { name: /全局/ })).toHaveTextContent("1");
  });

  test("creating a glob-matched rule sends parsed globs with always=false", async () => {
    const user = userEvent.setup();
    mockCommand("agent_session", () => ({ success: true, path: "/p/.husk/rules/api.md" }), "save_rule");
    render(<RulesPane ov={overview([])} reload={() => {}} />);

    await user.click(screen.getByRole("button", { name: "添加" }));
    await user.type(screen.getByPlaceholderText("api-conventions"), "api-conventions");

    // 始终生效 is the default; 按文件匹配 is what reveals the glob input.
    expect(screen.getByRole("button", { name: "规则生效方式" })).toHaveTextContent("始终生效");
    await user.click(screen.getByRole("button", { name: "规则生效方式" }));
    await user.click(await screen.findByRole("menuitemradio", { name: /按文件匹配/ }));

    await user.type(screen.getByPlaceholderText(GLOB_PLACEHOLDER), "src/**/*.ts, docs/*.md");
    await user.type(screen.getByPlaceholderText("新增接口必须同时更新 OpenAPI 描述…"), "必须补 OpenAPI");
    await user.click(screen.getByRole("button", { name: "创建" }));

    await waitFor(() =>
      expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
        op: "save_rule",
        payload: {
          name: "api-conventions",
          scope: "workspace",
          description: "",
          globs: ["src/**/*.ts", "docs/*.md"],
          always: false,
          body: "必须补 OpenAPI",
        },
      }),
    );
  });

  test("editing loads read_rule fields", async () => {
    const user = userEvent.setup();
    mockCommand(
      "agent_session",
      () => ({
        name: "styles",
        description: "样式约定",
        globs: ["src/**/*.css"],
        always: false,
        body: "只用 tailwind",
        scope: "global",
      }),
      "read_rule",
    );
    render(
      <RulesPane
        ov={overview([row({ name: "styles", description: "样式约定", globs: ["src/**/*.css"], always: false })])}
        reload={() => {}}
      />,
    );

    await user.click(screen.getByRole("button", { name: "编辑" }));

    expect(await screen.findByRole("button", { name: "规则生效方式" })).toHaveTextContent("按文件匹配");
    expect(screen.getByRole("button", { name: "规则位置" })).toHaveTextContent("全局");
    expect(screen.getByPlaceholderText(GLOB_PLACEHOLDER)).toHaveValue("src/**/*.css");
    expect(screen.getByDisplayValue("样式约定")).toBeInTheDocument();
    expect(screen.getByDisplayValue("只用 tailwind")).toBeInTheDocument();
  });
});
