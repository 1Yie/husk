import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ComposerBar } from "../index";
import { buildPayload } from "../queued-payload";
import { applyEvents } from "@/features/chat/hooks/apply-event";
import { emptyView, pendingApprovalOf } from "@/features/chat/hooks/stream-view";
import { useAgentStore } from "@/stores/agent-store";
import { renderWithProviders } from "@/test/render";
import { deferred, mockCommand, modelInfo, tauri } from "@/test/tauri";
import { CONFIG_CHANGED_EVENT, type Attachment, type ModelInfo } from "@/lib/agent-ipc/sessions";
import { useDockStore } from "@/features/chat/components/dock/state";
import { onOpenSettings } from "@/lib/settings-bus";
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
    // The host echoes the emptied queue: the panel must stay up for the
    // pending row alone, not fold away with the last queued draft.
    act(() => {
      useAgentStore.setState((s) => {
        const views = new Map(s.views);
        const v = views.get("/ws:1")!;
        views.set("/ws:1", applyEvents(v, [{ QueuedPrompts: { items: [] } }]));
        return { views };
      });
    });
    expect(screen.queryByText("排队消息")).not.toBeInTheDocument();
    expect(screen.getByText("等待插入")).toBeInTheDocument();
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

describe("composer IME keyboard handling", () => {
  const imeEvents = [
    { label: "active composition", isComposing: true, keyCode: 13 },
    { label: "WebKit compositionend before keydown", isComposing: false, keyCode: 229 },
  ];
  const commands = () => tauri.invoke.mock.calls.filter(([command]) => command === "agent_cmd");
  const confirmIme = (input: HTMLElement, event: typeof imeEvents[number], key = "Enter") => {
    fireEvent.compositionStart(input);
    // macOS WebKit can end composition before delivering its final keydown.
    if (!event.isComposing) fireEvent.compositionEnd(input);
    const unhandled = fireEvent.keyDown(input, { key, isComposing: event.isComposing, keyCode: event.keyCode });
    if (event.isComposing) fireEvent.compositionEnd(input);
    return unhandled;
  };

  test.each(imeEvents.flatMap((event) => [false, true].map((streaming) => ({ ...event, streaming }))))(
    "$label does not send or enqueue while streaming=$streaming and preserves the draft",
    async (event) => {
      renderWithProviders(<ComposerBar view={{ ...emptyView(), streaming: event.streaming }} sessionKey="/ws:1" />);
      await loaded();
      const input = screen.getByRole("textbox");
      fireEvent.change(input, { target: { value: "确认中文草稿" } });
      const unhandled = confirmIme(input, event);
      expect(commands()).toHaveLength(0);
      expect(input).toHaveValue("确认中文草稿");
      expect(unhandled).toBe(true); // Do not prevent the input method's default behavior.

      fireEvent.keyDown(input, { key: "Enter", isComposing: false, keyCode: 13 });
      const payload = { text: "确认中文草稿" };
      expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", {
        cmd: event.streaming ? { Enqueue: payload } : { Prompt: payload },
      });
      expect(commands()).toHaveLength(1);
      expect(input).toHaveValue("");
    },
  );

  test("Shift+Enter inserts a newline instead of sending", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "第一行");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(input).toHaveValue("第一行\n");
    expect(commands()).toHaveLength(0);
  });

  test.each(imeEvents.flatMap((event) => ["Enter", "Tab"].map((acceptKey) => ({ ...event, acceptKey }))))(
    "$label leaves mention candidates alone, then ordinary $acceptKey accepts",
    async (event) => {
      const user = userEvent.setup();
      renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
      await loaded();
      const input = screen.getByRole("textbox");
      await user.type(input, "/rem");
      const candidate = await screen.findByRole("option", { name: /^\/remember/ });
      const unhandled = confirmIme(input, event);
      expect(input).toHaveValue("/rem");
      expect(candidate).toBeInTheDocument();
      expect(unhandled).toBe(true);
      expect(commands()).toHaveLength(0);

      fireEvent.keyDown(input, { key: event.acceptKey, keyCode: event.acceptKey === "Enter" ? 13 : 9 });
      expect(input).toHaveValue("/remember ");
      expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
      expect(commands()).toHaveLength(0);
    },
  );

  test.each(imeEvents)("$label does not navigate or dismiss mentions", async (event) => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/");
    const options = await screen.findAllByRole("option");
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    for (const key of ["ArrowDown", "ArrowUp", "Escape", "Tab"]) {
      expect(confirmIme(input, event, key)).toBe(true);
      expect(input).toHaveValue("/");
      expect(options[0]).toHaveAttribute("aria-selected", "true");
      expect(screen.getByRole("listbox")).toBeInTheDocument();
    }
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(input).toHaveValue("/");
    expect(commands()).toHaveLength(0);
  });

  test.each(imeEvents)("$label does not submit a custom answer, then ordinary Enter replies", async (event) => {
    const view = applyEvents(emptyView(), [
      { QuestionAsked: { request_id: 4, question: "选择范围", options: [] } },
    ]);
    renderWithProviders(<ComposerBar view={view} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByPlaceholderText("自定义回答…");
    fireEvent.change(input, { target: { value: "当前项目" } });
    const unhandled = confirmIme(input, event);
    expect(commands()).toHaveLength(0);
    expect(input).toHaveValue("当前项目");
    expect(screen.getByText("选择范围")).toBeInTheDocument();
    expect(unhandled).toBe(true);

    fireEvent.keyDown(input, { key: "Enter", isComposing: false, keyCode: 13 });
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", {
      cmd: { AnswerQuestion: { request_id: 4, answer: "当前项目" } },
    });
    expect(commands()).toHaveLength(1);
    expect(screen.queryByText("选择范围")).not.toBeInTheDocument();
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

describe("slash command panel", () => {
  test("typing / groups the catalog under a 命令 header", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    await user.type(screen.getByRole("textbox"), "/");
    expect(await screen.findByText("命令")).toBeInTheDocument();
    // A command that takes an argument shows its placeholder.
    expect(screen.getByText("<内容>")).toBeInTheDocument();
    // A local command says what it does instead of promising an insert.
    expect(screen.getByRole("option", { name: /^\/diff 查看改动 打开$/ })).toBeInTheDocument();
  });

  test("picking /diff opens the dock on 改动 and never sends", async () => {
    const user = userEvent.setup();
    useDockStore.setState({ open: false, tab: "artifacts" });
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/diff");
    await user.click(screen.getByRole("option", { name: /^\/diff/ }));
    expect(useDockStore.getState().open).toBe(true);
    expect(useDockStore.getState().tab).toBe("changes");
    // A local pick leaves no text behind.
    expect(input).toHaveValue("");
    expect(tauri.invoke.mock.calls.filter(([cmd]) => cmd === "agent_cmd")).toHaveLength(0);
  });

  test("Enter on a bare /model opens the model picker instead of sending", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/model");
    // Dismiss the panel first: a bare local command is a submission, not a pick.
    await user.keyboard("{Escape}");
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("menu")).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /^模型/ })).toBeInTheDocument();
    expect(input).toHaveValue("");
    expect(tauri.invoke.mock.calls.filter(([cmd]) => cmd === "agent_cmd")).toHaveLength(0);
  });

  test("picking /remember inserts the command and leaves room for its argument", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/remember");
    await user.click(screen.getByRole("option", { name: /^\/remember/ }));
    expect(input).toHaveValue("/remember ");
  });

  test("picking /skills asks the shell to open 设置 on 技能", async () => {
    const user = userEvent.setup();
    const heard: string[] = [];
    const off = onOpenSettings((page) => heard.push(page));
    renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/skills");
    await user.click(screen.getByRole("option", { name: /^\/skills/ }));
    off();
    expect(heard).toEqual(["skills"]);
    expect(input).toHaveValue("");
  });

  test("the panel reports its open state so the page can hide the pills", async () => {
    const user = userEvent.setup();
    const onMentionOpenChange = vi.fn();
    renderWithProviders(
      <ComposerBar view={emptyView()} sessionKey="/ws:1" onMentionOpenChange={onMentionOpenChange} />,
    );
    await loaded();
    const input = screen.getByRole("textbox");
    await user.type(input, "/");
    expect(onMentionOpenChange).toHaveBeenLastCalledWith(true);
    await user.keyboard("{Escape}");
    expect(onMentionOpenChange).toHaveBeenLastCalledWith(false);
  });
});

describe("queued prompt editing", () => {
  const queued = ["先看这个", "第二条", "第三条"];
  // Mirrors the kernel echo: a SetQueued write lands in the store's view, the
  // way `QueuedPrompts` does in production, so the strip re-renders from it.
  const liveComposer = (items: string[]) => {
    const seeded = { ...emptyView(), streaming: true, queuedPrompts: items };
    useAgentStore.setState({ workspaceRoot: "/ws", activeId: 1 });
    useAgentStore.getState().loadView("/ws", 1, seeded);
    mockCommand("agent_cmd", (args) => {
      const cmd = args?.cmd as { SetQueued?: { items: string[] } } | undefined;
      if (cmd?.SetQueued) {
        const items = cmd.SetQueued.items;
        useAgentStore.setState((s) => {
          const views = new Map(s.views);
          views.set("/ws:1", applyEvents(views.get("/ws:1")!, [{ QueuedPrompts: { items } }]));
          return { views };
        });
      }
      return undefined;
    });
    const Live = () => {
      const live = useAgentStore((s) => s.views.get("/ws:1"));
      return <ComposerBar view={live ?? seeded} sessionKey="/ws:1" />;
    };
    return renderWithProviders(<Live />);
  };
  const setQueuedCalls = () =>
    tauri.invoke.mock.calls
      .filter(([cmd]) => cmd === "agent_cmd")
      .map(([, args]) => (args as { cmd: { SetQueued?: { items: string[] } } }).cmd.SetQueued)
      .filter((c): c is { items: string[] } => c !== undefined);
  const editButton = (n: number) => screen.getAllByRole("button", { name: "编辑" })[n];

  test("编辑 pulls the row out of the queue into the composer", async () => {
    const user = userEvent.setup();
    liveComposer(queued);
    await loaded();
    await user.click(editButton(1));
    expect(setQueuedCalls().at(-1)).toEqual({ items: ["先看这个", "第三条"] });
    expect(screen.getByRole("textbox")).toHaveValue("第二条");
    expect(screen.getByText("正在编辑第 2 条排队消息")).toBeInTheDocument();
  });

  test("编辑 re-reads attachments by path and shows their chips", async () => {
    const user = userEvent.setup();
    const shot: Attachment = { path: "/ws/.husk/attachments/shot.png", name: "shot.png", kind: "image" };
    mockCommand("agent_session", () => shot, "read_attachment");
    liveComposer([buildPayload("看这张图", [shot])]);
    await loaded();
    await user.click(editButton(0));
    expect(await screen.findByText("shot.png")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveValue("看这张图");
  });

  test("取消编辑 puts the original payload back at its index", async () => {
    const user = userEvent.setup();
    liveComposer(queued);
    await loaded();
    await user.click(editButton(1));
    await user.click(screen.getByRole("button", { name: "取消编辑" }));
    expect(setQueuedCalls().at(-1)).toEqual({ items: queued });
    expect(screen.queryByText(/正在编辑/)).not.toBeInTheDocument();
  });

  test("submitting an edit while streaming reinserts it at its original index", async () => {
    const user = userEvent.setup();
    liveComposer(queued);
    await loaded();
    await user.click(editButton(1));
    await user.clear(screen.getByRole("textbox"));
    await user.type(screen.getByRole("textbox"), "改过的第二条");
    await user.click(screen.getByRole("button", { name: "排队发送" }));
    expect(setQueuedCalls().at(-1)).toEqual({ items: ["先看这个", "改过的第二条", "第三条"] });
    expect(
      tauri.invoke.mock.calls.filter(([cmd, args]) =>
        cmd === "agent_cmd" && "Enqueue" in ((args as { cmd?: object })?.cmd ?? {})),
    ).toHaveLength(0);
  });

  test("a draft typed before the edit comes back after the edit is submitted", async () => {
    const user = userEvent.setup();
    liveComposer(queued);
    await loaded();
    await user.type(screen.getByRole("textbox"), "我还没写完");
    await user.click(editButton(0));
    expect(screen.getByRole("textbox")).toHaveValue("先看这个");
    await user.click(screen.getByRole("button", { name: "排队发送" }));
    expect(screen.getByRole("textbox")).toHaveValue("我还没写完");
    expect(screen.queryByText(/正在编辑/)).not.toBeInTheDocument();
  });

  test("only one row can be in edit at a time", async () => {
    const user = userEvent.setup();
    liveComposer(queued);
    await loaded();
    await user.click(editButton(0));
    const others = screen.getAllByRole("button", { name: "编辑" });
    expect(others.every((b) => b.hasAttribute("disabled"))).toBe(true);
  });
});
