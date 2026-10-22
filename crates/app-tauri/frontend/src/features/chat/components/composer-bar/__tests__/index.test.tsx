import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ComposerBar } from "../index";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView, pendingApprovalOf } from "@/features/chat/hooks/stream-view";
import { useAgentStore } from "@/stores/agent-store";
import { renderWithProviders } from "@/test/render";
import { deferred, mockCommand, modelInfo, tauri } from "@/test/tauri";
import { CONFIG_CHANGED_EVENT, type ModelInfo } from "@/lib/agent-ipc/sessions";
import { toast } from "sonner";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
beforeEach(() => useAgentStore.setState(useAgentStore.getInitialState(), true));
const modelButton = () => screen.getByRole("button", { name: /^当前模型:/ });
async function loaded() { await screen.findByRole("button", { name: /当前模型: Model A/ }); }

describe("composer model and thinking controls", () => {
  test("a stale restored tier is normalized before it is displayed", async () => {
    mockCommand("agent_session", () => ({ ...modelInfo(), active_thinking_level: "xhigh" }), "model_info");
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    expect(modelButton()).toHaveTextContent("Low");
    expect(modelButton()).not.toHaveTextContent(/xhigh/i);
  });

  test("selecting a model matches its tiers and dispatches one atomic model command", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    // The popover opens on the summary; 模型 drills into the list.
    await user.click(modelButton());
    await user.click(screen.getByRole("menuitem", { name: /^模型/ }));
    await user.click(screen.getByRole("menuitem", { name: /Model B/ }));
    expect(modelButton()).toHaveTextContent("Model B");
    expect(modelButton()).toHaveTextContent("Med");
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { SetModel: { provider: "test", model: "model-b" } } });
    expect(tauri.invoke.mock.calls.filter(([cmd, args]) => cmd === "agent_cmd" && "SetThinkingLevel" in (args?.cmd as object))).toHaveLength(0);
    // 思考 drills into the stepped control; a gesture reports its tier.
    await user.click(modelButton());
    await user.click(screen.getByRole("menuitem", { name: /^思考/ }));
    const slider = screen.getByRole("slider", { name: "思考等级" });
    // End is the highest tier this model declares.
    fireEvent.keyDown(slider, { key: "End" });
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { SetThinkingLevel: { level: "xhigh" } } });
  });

  test("the summary level states the current pair and each row drills in", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    await user.click(modelButton());
    const modelRow = screen.getByRole("menuitem", { name: /^模型/ });
    const thinkingRow = screen.getByRole("menuitem", { name: /^思考/ });
    expect(modelRow).toHaveTextContent("Model A");
    expect(thinkingRow).toHaveTextContent("Low");

    // 思考 → the stepped control, with the tier's own wording.
    await user.click(thinkingRow);
    const rail = screen.getByRole("slider", { name: "思考等级" });
    // The control names the model and the tier it is sitting on.
    expect(rail).toHaveAttribute("aria-valuetext", expect.stringMatching(/Low$/));
    expect(rail).toHaveAttribute("aria-valuemax", "3");

    // Back returns to the summary, not out of the popover.
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByRole("menuitem", { name: /^模型/ })).toBeInTheDocument();

    // Keyboard moves one stop at a time.
    await user.click(screen.getByRole("menuitem", { name: /^思考/ }));
    fireEvent.keyDown(screen.getByRole("slider", { name: "思考等级" }), { key: "ArrowRight" });
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { SetThinkingLevel: { level: "high" } } });
  });

  test("a model without reasoning offers no thinking row target", async () => {
    const user = userEvent.setup();
    const info = modelInfo();
    for (const m of info.models) {
      m.reasoning = false;
      m.available_levels = [];
    }
    mockCommand("agent_session", () => info, "model_info");
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    await user.click(modelButton());
    const row = screen.getByRole("menuitem", { name: /^思考/ });
    expect(row).toHaveAttribute("data-disabled");
    // Clicking it opens nothing: the session has no tiers to pick from.
    await user.click(row);
    expect(screen.queryByRole("slider", { name: "思考等级" })).toBeNull();
  });

  test("late model info cannot roll a newer configuration refresh back", async () => {
    const old = deferred<ModelInfo>();
    mockCommand("agent_session", () => old.promise, "model_info");
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await waitFor(() => expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "model_info" }));
    mockCommand("agent_session", () => ({ ...modelInfo(), active_model: "model-b", active_thinking_level: "xhigh" }), "model_info");
    act(() => window.dispatchEvent(new Event(CONFIG_CHANGED_EVENT)));
    await screen.findByRole("button", { name: /当前模型: Model B/ });
    await act(async () => old.resolve(modelInfo()));
    expect(modelButton()).toHaveTextContent("Model B");
    expect(modelButton()).toHaveTextContent("Xhigh");
  });

  test("same model id from a different provider uses that provider's capabilities", async () => {
    const info = modelInfo();
    info.models.push({ ...info.models[0], provider: "other", name: "Other Model A", available_levels: ["off", "xhigh"] });
    mockCommand("agent_session", () => ({ ...info, active_provider: "other", active_thinking_level: "high" }), "model_info");
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await screen.findByRole("button", { name: /当前模型: Other Model A/ });
    expect(modelButton()).toHaveTextContent("Xhigh");
  });
});

describe("composer message lifecycle", () => {
  test("a fresh message sends Prompt while streaming messages enqueue", async () => {
    const user = userEvent.setup();
    const view = emptyView();
    const { rerender } = renderWithProviders(<ComposerBar view={view} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "检查代码");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { Prompt: { text: "检查代码" } } });
    expect(input).toHaveValue("");
    rerender(<ComposerBar view={{ ...view, streaming: true }} sessionKey="/ws:1" />);
    await user.type(input, "完成后继续");
    await user.click(screen.getByRole("button", { name: "排队发送" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { Enqueue: { text: "完成后继续" } } });
    await user.click(screen.getByRole("button", { name: "中断回复" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: "Cancel" });
  });

  test("a rejected send restores the draft and reports failure", async () => {
    const user = userEvent.setup();
    vi.spyOn(console, "error").mockImplementation(() => {});
    mockCommand("agent_cmd", () => { throw new Error("session closed"); });
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    await user.type(screen.getByRole("textbox"), "保留这个问题");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveValue("保留这个问题"));
    expect(toast.error).toHaveBeenCalledWith(expect.stringContaining("发送失败"));
  });

  test("a promoted steer waits on its own row until the host echoes it", async () => {
    const user = userEvent.setup();
    const seeded = { ...emptyView(), streaming: true, queuedPrompts: ["先别改 config"] };
    useAgentStore.setState({ workspaceRoot: "/ws", activeId: 1 });
    useAgentStore.getState().loadView("/ws", 1, seeded);
    // The view prop normally comes from the store, so the click's own write
    // has to reach the composer the same way.
    const Live = () => {
      const live = useAgentStore((s) => s.views.get("/ws:1"));
      return <ComposerBar view={live ?? seeded} sessionKey="/ws:1" />;
    };
    renderWithProviders(<Live />);
    await loaded();
    await user.click(screen.getByRole("button", { name: "立即发送" }));
    expect(await screen.findByText("等待插入")).toBeInTheDocument();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { Steer: { text: "先别改 config" } } });
  });

  test("reasoning updates preserve the draft while queue and approval state still update", async () => {
    const user = userEvent.setup();
    let view = applyEvents(emptyView(), [{ UserPrompt: "分析" }, { StateChanged: "Reasoning" }, { ReasoningDelta: { text: "推导", parent: null } }]);
    const { rerender } = renderWithProviders(<ComposerBar view={view} sessionKey="/ws:1" />);
    await loaded();
    await user.type(screen.getByRole("textbox"), "正在编辑的跟进");
    view = applyEvents(view, [{ ReasoningDelta: { text: "更多推导", parent: null } }]);
    rerender(<ComposerBar view={view} sessionKey="/ws:1" />);
    expect(screen.getByRole("textbox")).toHaveValue("正在编辑的跟进");
    view = applyEvents(view, [{ QueuedPrompts: { items: ["排队的消息"] } },
      { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
      { StateChanged: { AwaitingToolConfirmation: { tool_name: "fuzzy_patch", diff_summary: "patch preview" } } },
      { ApprovalRequested: { request_id: 12, tool_name: "fuzzy_patch", diff: "patch preview", fuzzy: false } }]);
    rerender(<ComposerBar view={view} sessionKey="/ws:1" />);
    expect(screen.getByText("排队的消息")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "允许" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { ToolDecision: { request_id: 12, approved: true } } });
    expect(screen.getByRole("textbox")).toHaveValue("正在编辑的跟进");
  });

  test("a pending question dispatches the chosen answer", async () => {
    const user = userEvent.setup();
    const view = applyEvents(emptyView(), [{ QuestionAsked: { request_id: 4, question: "选择范围", options: [{ label: "当前项目" }] } }]);
    renderWithProviders(<ComposerBar view={view} sessionKey="/ws:1" />);
    await loaded();
    await user.click(screen.getByRole("button", { name: /当前项目/ }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { AnswerQuestion: { request_id: 4, answer: "当前项目" } } });
    expect(screen.queryByText("选择范围")).not.toBeInTheDocument();
  });

  test("an approved card leaves at once and cannot be answered twice", async () => {
    const user = userEvent.setup();
    const asked = applyEvents(emptyView(), [
      { UserPrompt: "改文件" },
      { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
      { StateChanged: { AwaitingToolConfirmation: { tool_name: "fuzzy_patch", diff_summary: "patch preview" } } },
      { ApprovalRequested: { request_id: 21, tool_name: "fuzzy_patch", diff: "patch preview", fuzzy: false } },
    ]);
    const { rerender } = renderWithProviders(<ComposerBar view={asked} sessionKey="/ws:1" />);
    await loaded();
    expect(pendingApprovalOf(asked)?.requestId).toBe(21);
    const allow = screen.getByRole("button", { name: "允许" });
    await user.click(allow);
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { ToolDecision: { request_id: 21, approved: true } } });
    // A second click on the still-mounted button cannot send the verdict
    // again — a duplicate would overwrite the first one in the engine's slot.
    await user.click(allow);
    expect(
      tauri.invoke.mock.calls.filter(([cmd, args]) =>
        cmd === "agent_cmd" && "ToolDecision" in ((args as { cmd?: object })?.cmd ?? {})),
    ).toHaveLength(1);

    // The kernel answers a verdict by entering the tool, not by echoing it
    // back — the strip must not sit there offering a second click.
    rerender(
      <ComposerBar
        view={applyEvents(asked, [{ StateChanged: { ExecutingTool: { tool_name: "fuzzy_patch" } } }])}
        sessionKey="/ws:1"
      />,
    );
    expect(screen.queryByRole("button", { name: "允许" })).not.toBeInTheDocument();
  });

  const awaitingBash = () =>
    applyEvents(emptyView(), [
      { UserPrompt: "跑脚本" },
      { ToolCallStarted: { name: "bash", args_preview: "cargo test" } },
      { StateChanged: { AwaitingToolConfirmation: { tool_name: "bash", diff_summary: "cargo test" } } },
      { ApprovalRequested: { request_id: 31, tool_name: "bash", diff: "", fuzzy: false } },
    ]);

  test("a delivered approval stays answered however long its events take", async () => {
    // The reported bug: the shell delivered the verdict and the script ran,
    // but the state change trailed behind a busy event path, and a timer on
    // the stream declared the approval failed and put the card back.
    renderWithProviders(<ComposerBar view={awaitingBash()} sessionKey="/ws:1" />);
    await loaded();
    vi.useFakeTimers();
    try {
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "允许" })); });
      await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
      expect(toast.error).not.toHaveBeenCalled();
      expect(screen.queryByRole("button", { name: "允许" })).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  test("a verdict the shell could not deliver brings the card back with the reason", async () => {
    const user = userEvent.setup();
    vi.spyOn(console, "error").mockImplementation(() => {});
    mockCommand("agent_cmd", () => { throw "审批 #31 没有送达，当前没有等待中的审批"; });
    renderWithProviders(<ComposerBar view={awaitingBash()} sessionKey="/ws:1" />);
    await loaded();
    await user.click(screen.getByRole("button", { name: "允许" }));
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("审批 #31 没有送达，当前没有等待中的审批"));
    expect(screen.getByRole("button", { name: "允许" })).toBeInTheDocument();
  });
});

describe("composer model picker sections", () => {
  test("models are grouped under their provider, sections in alphabetical order", async () => {
    const user = userEvent.setup();
    const info = modelInfo();
    // Providers come out of a backend map, so the picker imposes the order.
    info.models.push(
      { provider: "zeta", model: "z-1", name: "Zeta One" },
      { provider: "alpha", model: "a-1", name: "Alpha One" },
    );
    mockCommand("agent_session", () => info, "model_info");
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    await user.click(modelButton());
    // The list lives one level down, behind the 模型 row.
    await user.click(screen.getByRole("menuitem", { name: /^模型/ }));
    const menu = (await screen.findAllByRole("menu")).at(-1)!;
    // `getAllBy*` returns document order, which is exactly what is under test.
    const inMenu = (text: RegExp) =>
      screen.getAllByText(text).filter((el) => menu.contains(el)).map((el) => el.textContent);
    expect(inMenu(/^(alpha|test|zeta)$/)).toEqual(["alpha", "test", "zeta"]);
    expect(inMenu(/^(Alpha One|Model A|Model B|Zeta One)$/)).toEqual([
      "Alpha One",
      "Model A",
      "Model B",
      "Zeta One",
    ]);
  });
});
