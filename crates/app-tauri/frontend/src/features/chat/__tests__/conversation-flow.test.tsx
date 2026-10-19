import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { ComposerBar } from "@/features/chat/components/composer-bar";
import { activeViewNow, initAgentStore, useActiveView, useAgentStore } from "@/stores/agent-store";
import { emptyView } from "@/features/chat/hooks/stream-view";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import { renderWithProviders } from "@/test/render";
import { emitAgentEvent, tauri } from "@/test/tauri";
import type { ChatMessage, UiEvent } from "@/types";

// Exercise real reducers/store/components. Rich Markdown/highlighting and
// browser layout have their own boundary; jsdom is not a frame-time benchmark.
vi.mock("@/features/chat/components/chat-stream/markdown-stream", () => ({
  MemoStreamdown: ({ text }: { text: string }) => <div>{text}</div>,
}));
let dispose: () => void;
function Conversation({ sessionKey = "/project:1" }: { sessionKey?: string } = {}) {
  const view = useActiveView();
  return <>
    <ChatStream view={view} sessionKey={sessionKey} hasMore={false} loading={false} />
    <ComposerBar view={view} sessionKey={sessionKey} />
  </>;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2025-01-01T00:00:00Z"));
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  dispose = initAgentStore();
});
afterEach(() => dispose());
/** Fake layout for virtualization tests: step N sits at N*1000px in a 10k
 *  column, the scroller is 800px tall, and ids are ordered as rendered.
 *  `near(id)` mirrors what lands within the bottom viewport + overscan. */
function layoutSteps() {
  const H = 10_000;
  const order = () => [...document.querySelectorAll('[id*="-step-"]')].map((el) => el.id);
  const topOf = (el: Element) => {
    const ids = order();
    const i = ids.indexOf(el.id);
    return H - (ids.length - i) * 1000;
  };
  const spies = [
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("stream-scroll")) return { top: 0, bottom: 800, height: 800, left: 0, right: 600, width: 600, x: 0, y: 0, toJSON() {} } as DOMRect;
      if (this.id.includes("-step-")) {
        const top = topOf(this);
        return { top, bottom: top + 100, height: 100, left: 0, right: 600, width: 600, x: 0, y: top, toJSON() {} } as DOMRect;
      }
      return { top: 0, bottom: 0, height: 0, left: 0, right: 0, width: 0, x: 0, y: 0, toJSON() {} } as DOMRect;
    }),
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("stream-scroll") ? H : 0;
    }),
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("stream-scroll") ? 800 : 0;
    }),
  ];
  return {
    near: (id: string) => {
      const el = document.getElementById(id);
      return !!el && topOf(el) + 100 >= H - 800 - 600;
    },
    restore: () => spies.forEach((s) => s.mockRestore()),
  };
}
async function deliver(...events: UiEvent[]) {
  await act(async () => {
    for (const event of events) emitAgentEvent({ root: "/project", session: 1, event });
    await vi.advanceTimersByTimeAsync(16);
  });
}

describe("conversation lifecycle through the event subscription", () => {
  test("send → reasoning → answer → finish preserves the draft and commits final text once", async () => {
    renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "请分析问题" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { Prompt: { text: "请分析问题" } } });
    await deliver(
      { UserPrompt: "请分析问题" }, { StateChanged: "Reasoning" },
      { ReasoningDelta: { text: "先检查状态", parent: null } },
    );
    expect(screen.getByText("请分析问题")).toBeInTheDocument();
    fireEvent.change(input, { target: { value: "跟进草稿" } });
    await deliver({ ReasoningDelta: { text: "与边界条件", parent: null } });
    expect(input).toHaveValue("跟进草稿");
    fireEvent.click(screen.getByRole("button", { name: /思考中/ }));
    expect(screen.getAllByText("先检查状态与边界条件").length).toBeGreaterThan(0);
    await deliver(
      { TextDelta: { text: "最终", parent: null } }, { TextDelta: { text: "答案", parent: null } },
      { AssistantMessage: "最终答案" }, { StateChanged: "Finished" },
    );
    expect(screen.getByText("最终答案")).toBeInTheDocument();
    expect(activeViewNow().items.filter((item) => item.kind === "assistant")).toHaveLength(1);
    expect(input).toHaveValue("跟进草稿");
    expect(screen.queryByRole("button", { name: "中断回复" })).not.toBeInTheDocument();
  });

  test("approval and question updates remain actionable during a streamed turn", async () => {
    renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    await deliver(
      { UserPrompt: "修改文件" }, { StateChanged: "Reasoning" },
      { ToolCallStarted: { name: "fuzzy_patch", args_preview: "a.ts" } },
      { ApprovalRequested: { request_id: 6, tool_name: "fuzzy_patch", diff: "preview", fuzzy: false } },
    );
    fireEvent.click(screen.getByRole("button", { name: "允许" }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { ToolDecision: { request_id: 6, approved: true } } });
    await deliver(
      { ToolCallFinished: { name: "fuzzy_patch", ok: true, content: "写入完成" } },
      { QuestionAsked: { request_id: 8, question: "继续范围", options: [{ label: "当前文件" }] } },
    );
    fireEvent.click(screen.getByRole("button", { name: /当前文件/ }));
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", { cmd: { AnswerQuestion: { request_id: 8, answer: "当前文件" } } });
    await deliver({ QuestionAnswered: { request_id: 8 } }, { StateChanged: "Finished" });
    expect(screen.queryByText("继续范围")).not.toBeInTheDocument();
  });

  test("a loaded standalone notice cannot create an unloaded-history rail dot", async () => {
    act(() => useAgentStore.getState().loadView("/project", 1, {
      ...emptyView(), historyStart: 0, historyTotal: 3, turnTotal: 1, turnOffset: 0,
      items: [
        { kind: "system", standalone: true, text: "已切换模型", hi: 0 },
        { kind: "user", text: "你好", hi: 1, ts: 1 },
        { kind: "assistant", text: "已准备就绪", streaming: false, hi: 2 },
      ],
    }));
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    expect(screen.getByText("已切换模型")).toBeInTheDocument();
    const marks = container.querySelectorAll(".group\\/item");
    // top handle, system notice, user, answer, bottom handle — no fake dot
    expect(marks).toHaveLength(5);
  });

  test("a long live session keeps a bounded number of turns mounted", async () => {
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    for (let i = 0; i < 60; i++) {
      await deliver(
        { UserPrompt: `问题${i}` }, { StateChanged: "Reasoning" },
        { AssistantMessage: `回答${i}` }, { StateChanged: "Finished" },
      );
    }
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    const mounted = container.querySelectorAll('[id^="chat-turn-"][id$="-user"]').length;
    expect(mounted).toBeLessThanOrEqual(40);
    expect(screen.getByText("回答59")).toBeInTheDocument();
    expect(screen.queryByText("回答0")).not.toBeInTheDocument();
  });

  test("a giant turn has no intermediate scroll gate", () => {
    const history: ChatMessage[] = [{ role: "user", content: "问题" }];
    for (let i = 0; i < 40; i++) {
      history.push({
        role: "assistant",
        content: "",
        reasoning: `思考 ${i}`,
        tool_calls: [
          { id: `c${i}`, type: "function", function: { name: "bash", arguments: "{}" } },
        ],
      });
      history.push({ role: "tool", content: "ok", tool_call_id: `c${i}` });
    }
    history.push({ role: "assistant", content: "完成" });
    act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
    const { container } = renderWithProviders(<Conversation />);
    const steps = container.querySelectorAll('[id^="chat-turn-"][id*="-step-"]');
    // The jsdom observer treats every step as visible; the point here is that
    // there is no "show more" gate in the intermediate UI.
    expect(steps.length).toBeGreaterThan(24);
    expect(screen.queryByText(/前面还有/)).not.toBeInTheDocument();
    expect(screen.queryByText("过程")).not.toBeInTheDocument();
    expect(screen.getByText("完成")).toBeInTheDocument();
  });

  test("a tool-heavy history is bounded by total steps, not only turns", async () => {
    const history: ChatMessage[] = [];
    for (let i = 0; i < 40; i++) {
      history.push({ role: "user", content: `问题${i}` });
      for (let j = 0; j < 5; j++) {
        history.push({
          role: "assistant",
          content: `检查 ${i}-${j}`,
          tool_calls: [
            { id: `c${i}-${j}`, type: "function", function: { name: "bash", arguments: "{}" } },
          ],
        });
        history.push({ role: "tool", content: "ok", tool_call_id: `c${i}-${j}` });
      }
      history.push({ role: "assistant", content: `回答${i}` });
    }
    act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    const steps = container.querySelectorAll('[id^="chat-turn-"][id*="-step-"]');
    expect(steps.length).toBeLessThanOrEqual(120);
    expect(steps.length).toBeGreaterThan(0);
    // The block budget keeps roughly the last 10 tool-heavy turns (120
    // blocks / 12 per turn), not the 30-turn cap.
    expect(container.querySelectorAll('[id^="chat-turn-"][id$="-user"]').length).toBeLessThanOrEqual(12);
    expect(screen.getByText("回答39")).toBeInTheDocument();
  });

  test("steps outside the observed window are placeholders", async () => {
    const original = window.IntersectionObserver;
    class ManualIO {
      static instances: ManualIO[] = [];
      targets = new Set<Element>();
      root = null;
      rootMargin = "0px";
      thresholds = [0];
      constructor(private cb: IntersectionObserverCallback) {
        ManualIO.instances.push(this);
      }
      observe(target: Element) { this.targets.add(target); }
      unobserve(target: Element) { this.targets.delete(target); }
      disconnect() { this.targets.clear(); }
      takeRecords(): IntersectionObserverEntry[] { return []; }
      fire(visible: boolean) {
        this.cb(
          [...this.targets].map(
            (target) => ({ target, isIntersecting: visible }) as IntersectionObserverEntry,
          ),
          this as unknown as IntersectionObserver,
        );
      }
    }
    vi.stubGlobal("IntersectionObserver", ManualIO);
    try {
      const history: ChatMessage[] = [{ role: "user", content: "问题" }];
      for (let i = 0; i < 5; i++) {
        history.push({
          role: "assistant",
          content: `检查 ${i}`,
          tool_calls: [
            { id: `c${i}`, type: "function", function: { name: "bash", arguments: "{}" } },
          ],
        });
        history.push({ role: "tool", content: "ok", tool_call_id: `c${i}` });
      }
      history.push({ role: "assistant", content: "完成" });
      act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
      // Steps 1000px apart in a 10k column; the reader lands at the bottom
      // (800px viewport) — only the last steps are inside view + overscan.
      const geo = layoutSteps();
      const { container } = renderWithProviders(<Conversation />);
      await act(async () => { await Promise.resolve(); });
      const mounted = () => [...container.querySelectorAll("[data-step-mounted]")].map((el) => el.id);
      // The steps the reader will see are real in the FIRST paint…
      expect(mounted().length).toBeGreaterThan(0);
      expect(mounted().every((id) => geo.near(id))).toBe(true);
      expect(screen.getByText("完成")).toBeInTheDocument();
      // …everything further away still waits for the observer.
      expect(container.querySelectorAll("[data-step-placeholder]").length).toBeGreaterThan(0);
      act(() => { ManualIO.instances.forEach((observer) => observer.fire(true)); });
      expect(container.querySelectorAll("[data-step-placeholder]")).toHaveLength(0);
      geo.restore();
    } finally {
      vi.stubGlobal("IntersectionObserver", original);
    }
  });

  test("measured heights outlive remounts; the live tail never unmounts", async () => {
    const original = window.IntersectionObserver;
    const io: { cb?: IntersectionObserverCallback; targets: Set<Element> } = { targets: new Set() };
    class ManualIO {
      root = null; rootMargin = "0px"; thresholds = [0];
      constructor(cb: IntersectionObserverCallback) { io.cb = cb; }
      observe(t: Element) { io.targets.add(t); }
      unobserve(t: Element) { io.targets.delete(t); }
      disconnect() { io.targets.clear(); }
      takeRecords(): IntersectionObserverEntry[] { return []; }
    }
    const fire = (visible: boolean) => act(() => {
      io.cb?.([...io.targets].map((target) => ({ target, isIntersecting: visible }) as IntersectionObserverEntry),
        {} as IntersectionObserver);
    });
    // jsdom has no layout: a mounted step "measures" 333px at a 600px column.
    const h = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.hasAttribute("data-step-mounted") ? 333 : 0;
    });
    const w = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(600);
    vi.stubGlobal("IntersectionObserver", ManualIO);
    try {
      const history: ChatMessage[] = [{ role: "user", content: "问题" }];
      for (let i = 0; i < 3; i++) {
        history.push({
          role: "assistant",
          content: `检查 ${i}`,
          tool_calls: [{ id: `k${i}`, type: "function", function: { name: "bash", arguments: "{}" } }],
        });
        history.push({ role: "tool", content: "ok", tool_call_id: `k${i}` });
      }
      history.push({ role: "assistant", content: "完成" });
      act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
      const first = renderWithProviders(<Conversation />);
      await act(async () => { await Promise.resolve(); });
      fire(true);
      const step0 = () => first.container.querySelector<HTMLElement>('[id$="-step-0"]')!;
      expect(step0().hasAttribute("data-step-mounted")).toBe(true);
      first.unmount();

      // A fresh instance (trim/reveal, session switch) starts its placeholder
      // at the measured 333px, not the length estimate. (step-0 sits far
      // above the bottom viewport, so it stays a placeholder.)
      const geo = layoutSteps();
      const second = renderWithProviders(<Conversation />);
      await act(async () => { await Promise.resolve(); });
      const ph = second.container.querySelector<HTMLElement>('[id$="-step-0"]')!;
      expect(ph.hasAttribute("data-step-placeholder")).toBe(true);
      expect(ph.style.height).toBe("333px");
      second.unmount();

      // …but the cache is per SESSION. Turn ids are `t{hi}` — an index into
      // one history — so another session's `t0/step-0` must not inherit this
      // one's height and land the scroll on the wrong offset.
      const other = renderWithProviders(<Conversation sessionKey="/project:2" />);
      await act(async () => { await Promise.resolve(); });
      const otherPh = other.container.querySelector<HTMLElement>('[id$="-step-0"]')!;
      expect(otherPh.hasAttribute("data-step-placeholder")).toBe(true);
      expect(otherPh.style.height).not.toBe("333px");
      other.unmount();
      geo.restore();

      // A streaming tail stays mounted after leaving the overscan.
      act(() => useAgentStore.getState().dropView("/project", 1));
      const live = renderWithProviders(<Conversation />);
      await deliver({ UserPrompt: "继续" }, { StateChanged: "Reasoning" }, { TextDelta: { text: "正在写", parent: null } });
      fire(true);
      fire(false);
      await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
      const tail = [...live.container.querySelectorAll<HTMLElement>('[id*="-step-"]')].at(-1)!;
      expect(tail.hasAttribute("data-step-mounted")).toBe(true);
      expect(screen.getByText("正在写")).toBeInTheDocument();
    } finally {
      h.mockRestore();
      w.mockRestore();
      vi.stubGlobal("IntersectionObserver", original);
    }
  });

  test("a text block mounted as the live tail keeps its renderer once a tool lands", async () => {
    const { container } = renderWithProviders(<Conversation />);
    await deliver({ UserPrompt: "看看" }, { StateChanged: "Reasoning" }, { TextDelta: { text: "先检查一下", parent: null } });
    const block = () => screen.getByText("先检查一下");
    // Streamdown (mocked as a bare div) while it is the live tail…
    expect(block().className).not.toContain("whitespace-pre-wrap");
    await deliver({ ToolCallStarted: { name: "bash", args_preview: "{}", parent: null } });
    // …and still Streamdown after a tool makes it non-last: no swap.
    expect(block().className).not.toContain("whitespace-pre-wrap");
    expect(container.querySelectorAll('[id*="-step-"]').length).toBeGreaterThan(1);
  });

  test("numbered lists and links never take the plain-text path", () => {
    const fresh = [
      { role: "user", content: "问题" },
      { role: "assistant", content: "1. 第一步", tool_calls: [{ id: "a", type: "function", function: { name: "bash", arguments: "{}" } }] },
      { role: "tool", content: "ok", tool_call_id: "a" },
      { role: "assistant", content: "见 https://example.com", tool_calls: [{ id: "b", type: "function", function: { name: "bash", arguments: "{}" } }] },
      { role: "tool", content: "ok", tool_call_id: "b" },
      // The renderer enables single-dollar math, so `$x$` is markup too —
      // the plain path would print the delimiters verbatim.
      { role: "assistant", content: "结果是 $x^2$", tool_calls: [{ id: "c", type: "function", function: { name: "bash", arguments: "{}" } }] },
      { role: "tool", content: "ok", tool_call_id: "c" },
      { role: "assistant", content: "转义 \\*不是\* 星号", tool_calls: [{ id: "d", type: "function", function: { name: "bash", arguments: "{}" } }] },
      { role: "tool", content: "ok", tool_call_id: "d" },
      { role: "assistant", content: "普通前言", tool_calls: [{ id: "e", type: "function", function: { name: "bash", arguments: "{}" } }] },
      { role: "tool", content: "ok", tool_call_id: "e" },
      { role: "assistant", content: "完成" },
    ] as ChatMessage[];
    act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(fresh, null, 0)));
    renderWithProviders(<Conversation />);
    expect(screen.getByText("1. 第一步").className).not.toContain("whitespace-pre-wrap");
    expect(screen.getByText("见 https://example.com").className).not.toContain("whitespace-pre-wrap");
    expect(screen.getByText("结果是 $x^2$").className).not.toContain("whitespace-pre-wrap");
    expect(screen.getByText("转义 \\*不是\* 星号").className).not.toContain("whitespace-pre-wrap");
    // A short markup-free preamble still takes the cheap path.
    expect(screen.getByText("普通前言").className).toContain("whitespace-pre-wrap");
  });

  test("an older page that brings the tail turn's prompt does not jump to the bottom", async () => {
    // First page after launch: one tool-heavy turn fills it, so it holds no
    // user row at all — the prompt sits on the NEXT older page.
    const page: ChatMessage[] = [];
    for (let i = 0; i < 6; i++) {
      page.push({
        role: "assistant",
        content: `检查 ${i}`,
        tool_calls: [{ id: `p${i}`, type: "function", function: { name: "bash", arguments: "{}" } }],
      });
      page.push({ role: "tool", content: "ok", tool_call_id: `p${i}` });
    }
    page.push({ role: "assistant", content: "完成" });
    act(() => useAgentStore.getState().loadView("/project", 1, { ...viewFromHistory(page, null, 20), historyStart: 20 }));
    const into = vi.spyOn(Element.prototype, "scrollIntoView");
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    // The reader scrolls up (a real gesture, away from the bottom).
    const scroller = container.querySelector<HTMLElement>(".stream-scroll")!;
    vi.spyOn(scroller, "scrollHeight", "get").mockReturnValue(5000);
    vi.spyOn(scroller, "clientHeight", "get").mockReturnValue(800);
    fireEvent.wheel(scroller);
    scroller.scrollTop = 1000;
    fireEvent.scroll(scroller);
    into.mockClear();

    const older = viewFromHistory([
      { role: "user", content: "最早的提问" },
      { role: "assistant", content: "收到" },
    ], null, 18).items;
    await act(async () => {
      useAgentStore.getState().prependItems("/project", 1, older, 18);
      await vi.advanceTimersByTimeAsync(50);
    });
    expect(screen.getByText("最早的提问")).toBeInTheDocument();
    const toEnd = into.mock.contexts.filter((el) => (el as Element).id === "chat-stream-end");
    expect(toEnd).toHaveLength(0);

    // A genuinely new prompt still re-pins to the bottom.
    await deliver({ UserPrompt: "新的问题" });
    expect(into.mock.contexts.some((el) => (el as Element).id === "chat-stream-end")).toBe(true);
    into.mockRestore();
  });

  test("backfill stops once the reader scrolls away from the bottom", async () => {
    // Enough plain turns that the tail-first mount leaves a backfill to run.
    const history: ChatMessage[] = [];
    for (let i = 0; i < 24; i++) history.push({ role: "user", content: `问${i}` }, { role: "assistant", content: `答${i}` });
    act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
    // No idle callback in jsdom → the backfill scheduler falls back to a 0ms
    // timer; hold the clock so we control when chunks run.
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await Promise.resolve(); });
    const turnsMounted = () => container.querySelectorAll('[id^="chat-turn-"][id$="-user"]').length;
    const before = turnsMounted();
    expect(before).toBeLessThan(24);
    // A real upward gesture away from the bottom.
    const scroller = container.querySelector<HTMLElement>(".stream-scroll")!;
    vi.spyOn(scroller, "scrollHeight", "get").mockReturnValue(5000);
    vi.spyOn(scroller, "clientHeight", "get").mockReturnValue(800);
    fireEvent.wheel(scroller);
    scroller.scrollTop = 2000;
    await act(async () => { fireEvent.scroll(scroller); });
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    // Nothing more was mounted above the reader.
    expect(turnsMounted()).toBe(before);
  });

  test("keyboard paging counts as a reader gesture; typing in the composer does not", async () => {
    const history: ChatMessage[] = [];
    for (let i = 0; i < 4; i++) history.push({ role: "user", content: `问${i}` }, { role: "assistant", content: `答${i}` });
    act(() => useAgentStore.getState().loadView("/project", 1, viewFromHistory(history, null, 0)));
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    const scroller = container.querySelector<HTMLElement>(".stream-scroll")!;
    vi.spyOn(scroller, "scrollHeight", "get").mockReturnValue(5000);
    vi.spyOn(scroller, "clientHeight", "get").mockReturnValue(800);
    // The jump-to-bottom arrow's own wrapper collapses to zero width (and
    // stops taking pointers) while the follow pin is on — that is also what
    // keeps the dock pill dead centre.
    const following = () => {
      const wrap = screen.getByRole("button", { name: "回到底部" }).parentElement!;
      return wrap.classList.contains("pointer-events-none") && wrap.classList.contains("w-0");
    };
    // Move the fake clock well past any landing-time stamp.
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    // A space typed into the composer is not a scroll.
    const box = container.querySelector("textarea")!;
    fireEvent.keyDown(box, { key: " " });
    scroller.scrollTop = 1000;
    await act(async () => { fireEvent.scroll(scroller); });
    expect(following()).toBe(true);
    // PageUp on the page is.
    fireEvent.keyDown(document.body, { key: "PageUp" });
    scroller.scrollTop = 900;
    await act(async () => { fireEvent.scroll(scroller); });
    expect(following()).toBe(false);
  });

  test("a long agentic answer folds into a bounded run of rail dashes", async () => {
    const items: import("@/features/chat/hooks/stream-view").StreamItem[] = [
      { kind: "user", text: "做任务", hi: 0, ts: 1 },
    ];
    for (let i = 0; i < 20; i++) {
      items.push({ kind: "thinking", text: `想${i}`, done: true, hi: items.length });
      items.push({ kind: "tool", name: "read", args: `f${i}`, content: "ok", ok: true, hi: items.length } as never);
    }
    items.push({ kind: "assistant", text: "完成", streaming: false, hi: items.length });
    act(() => useAgentStore.getState().loadView("/project", 1, {
      ...emptyView(), historyStart: 0, historyTotal: items.length, turnTotal: 1, turnOffset: 0, items,
    }));
    const { container } = renderWithProviders(<Conversation />);
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    // top handle, prompt, at most 6 answer dashes, bottom handle
    expect(container.querySelectorAll(".group\\/item").length).toBe(9);
  });
});
