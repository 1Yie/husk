import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { MemoryPane } from "../index";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import type { MemoryOverview } from "@/lib/agent-ipc/sessions";

/** Let real time pass inside act — for the 300ms busy-hint delay. */
const settle = (ms: number) => act(() => new Promise<void>((resolve) => setTimeout(resolve, ms)));

function overview(personaCount = 12, factCount = 13): MemoryOverview {
  return {
    enabled: true,
    distill: true,
    // The desktop build carries the encoder; the switch only exists when it does.
    semantic_available: true,
    // The row names the model once it is on, so the pane says what is scoring.
    semantic_model: "paraphrase-multilingual-MiniLM-L12-v2",
    persona: Array.from({ length: personaCount }, (_, i) => ({ key: `preference-${i + 1}`, value: `value-${i + 1}` })),
    facts: Array.from({ length: factCount }, (_, i) => ({ id: i + 1, text: `fact-${i + 1}`, confidence: 0.9, created_at: 0 })),
    episodes: 0,
  };
}

test("memory lists expand independently in batches and can return to the compact view", async () => {
  const user = userEvent.setup();
  mockCommand("agent_session", () => overview(), "memory_overview");
  renderWithProviders(<MemoryPane />);

  await screen.findByText("preference-5");
  expect(screen.queryByText("preference-6")).not.toBeInTheDocument();
  expect(screen.getByText("fact-5")).toBeInTheDocument();
  expect(screen.queryByText("fact-6")).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "显示更多个人偏好" }));
  expect(screen.getByText("preference-10")).toBeInTheDocument();
  expect(screen.queryByText("preference-11")).not.toBeInTheDocument();
  expect(screen.queryByText("fact-6")).not.toBeInTheDocument();
  expect(screen.getByText("已显示 10 / 12 条")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "显示更多个人偏好" }));
  expect(screen.getByText("preference-12")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "显示更多个人偏好" })).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "显示更多记忆事实" }));
  expect(screen.getByText("fact-10")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "收起个人偏好" }));
  expect(screen.queryByText("preference-6")).not.toBeInTheDocument();
  expect(screen.getByText("fact-10")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "收起记忆事实" }));
  expect(screen.queryByText("fact-6")).not.toBeInTheDocument();
});

test("each preference carries its scope as a tag, not as a description suffix", async () => {
  const data: MemoryOverview = {
    enabled: true,
    distill: true,
    persona: [
      { key: "language", value: "中文", scope: "global" },
      { key: "build", value: "make -j4", scope: "workspace" },
    ],
    facts: [],
    episodes: 0,
  };
  mockCommand("agent_session", () => data, "memory_overview");
  renderWithProviders(<MemoryPane />);

  // The tag says which layer the entry belongs to; the value stays the value.
  const workspaceRow = (await screen.findByText("make -j4")).closest(".group")!;
  expect(within(workspaceRow as HTMLElement).getByText("工作区")).toBeInTheDocument();
  expect(workspaceRow).not.toHaveTextContent("仅本工作区");
  const globalRow = screen.getByText("中文").closest(".group")!;
  expect(within(globalRow as HTMLElement).getByText("全局")).toBeInTheDocument();
});

test("the scope tabs count each layer and the pagination follows the filter", async () => {
  const user = userEvent.setup();
  const data: MemoryOverview = {
    enabled: true,
    distill: true,
    persona: [
      ...Array.from({ length: 12 }, (_, i) => ({
        key: `preference-${i + 1}`,
        value: `value-${i + 1}`,
        scope: "global" as const,
      })),
      { key: "build", value: "make -j4", scope: "workspace" as const },
    ],
    facts: [],
    episodes: 0,
  };
  mockCommand("agent_session", () => data, "memory_overview");
  renderWithProviders(<MemoryPane />);

  await screen.findByText("preference-5");
  // The counts are per layer, not for the list as a whole.
  const tab = (label: string) => screen.getByRole("button", { name: new RegExp(`^${label}`) });
  expect(tab("工作区")).toHaveTextContent("1");
  expect(tab("全局")).toHaveTextContent("12");
  await user.click(tab("工作区"));
  expect(screen.getByText("make -j4")).toBeInTheDocument();
  expect(screen.queryByText("preference-1")).not.toBeInTheDocument();
  // One row is under the preview cap: no footer to expand.
  expect(screen.queryByRole("button", { name: "显示更多个人偏好" })).not.toBeInTheDocument();

  // The footer counts what the filter shows, not everything the store holds.
  await user.click(tab("全局"));
  expect(screen.getByText("preference-5")).toBeInTheDocument();
  expect(screen.queryByText("build")).not.toBeInTheDocument();
  expect(screen.getByText("已显示 5 / 12 条")).toBeInTheDocument();
});

test("adding a preference with 仅当前工作区 writes it to the workspace scope", async () => {
  const user = userEvent.setup();
  mockCommand("agent_session", () => overview(0, 0), "memory_overview");
  mockCommand("agent_session", () => ({ success: true }), "memory_set");
  renderWithProviders(<MemoryPane />);

  await user.click(await screen.findByRole("button", { name: "添加" }));
  await user.type(screen.getByPlaceholderText("key"), "build");
  await user.type(screen.getByPlaceholderText("value"), "make -j4");
  // A new preference is global until the dialog is told otherwise.
  expect(screen.getByRole("button", { name: "偏好范围" })).toHaveTextContent("全局");
  await user.click(screen.getByRole("button", { name: "偏好范围" }));
  await user.click(await screen.findByRole("menuitemradio", { name: "仅当前工作区" }));
  await user.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "memory_set",
      payload: { key: "build", value: "make -j4", scope: "workspace" },
    }),
  );
});

test("editing opens on the entry's own scope, and changing it forgets the old row first", async () => {
  const user = userEvent.setup();
  const data: MemoryOverview = {
    enabled: true,
    distill: true,
    persona: [{ key: "language", value: "中文", scope: "global" }],
    facts: [],
    episodes: 0,
  };
  mockCommand("agent_session", () => data, "memory_overview");
  mockCommand("agent_session", () => ({ success: true }), "memory_set");
  mockCommand("agent_session", () => ({ success: true, removed: true }), "memory_forget");
  renderWithProviders(<MemoryPane />);

  await user.click(await screen.findByRole("button", { name: "编辑" }));
  expect(screen.getByRole("button", { name: "偏好范围" })).toHaveTextContent("全局");
  await user.click(screen.getByRole("button", { name: "偏好范围" }));
  await user.click(await screen.findByRole("menuitemradio", { name: "仅当前工作区" }));
  // The move is announced — the label cannot live in both scopes at once.
  expect(screen.getByText(/移到新范围/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "memory_set",
      payload: { key: "language", value: "中文", scope: "workspace" },
    }),
  );
  // Only the row it came from: the same label may be kept in the other scope,
  // and that entry is not part of the move.
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
    op: "memory_forget",
    payload: { key: "language", scope: "global" },
  });
  const ops = tauri.invoke.mock.calls
    .map(([, args]) => args?.op)
    .filter((op) => op === "memory_forget" || op === "memory_set");
  expect(ops).toEqual(["memory_forget", "memory_set"]);
});

test("editing without a scope change overwrites the row and forgets nothing", async () => {
  const user = userEvent.setup();
  const data: MemoryOverview = {
    enabled: true,
    distill: true,
    persona: [{ key: "language", value: "中文", scope: "workspace" }],
    facts: [],
    episodes: 0,
  };
  mockCommand("agent_session", () => data, "memory_overview");
  mockCommand("agent_session", () => ({ success: true }), "memory_set");
  mockCommand("agent_session", () => ({ success: true, removed: true }), "memory_forget");
  renderWithProviders(<MemoryPane />);

  await user.click(await screen.findByRole("button", { name: "编辑" }));
  // Same scope: nothing moves, so the dialog says nothing about one.
  expect(screen.queryByText(/移到新范围/)).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "memory_set",
      payload: { key: "language", value: "中文", scope: "workspace" },
    }),
  );
  expect(tauri.invoke).not.toHaveBeenCalledWith(
    "agent_session",
    expect.objectContaining({ op: "memory_forget" }),
  );
});

test("deleting a row forgets that row's scope, not the label in every scope", async () => {
  const user = userEvent.setup();
  const data: MemoryOverview = {
    enabled: true,
    distill: true,
    persona: [
      { key: "style", value: "全局值", scope: "global" },
      { key: "style", value: "工作区值", scope: "workspace" },
    ],
    facts: [],
    episodes: 0,
  };
  mockCommand("agent_session", () => structuredClone(data), "memory_overview");
  mockCommand("agent_session", () => ({ success: true, removed: true }), "memory_forget");
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("工作区值")).closest(".group")!;
  await user.click(row.querySelector<HTMLButtonElement>('button[title="删除"]')!);
  await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "删除" }));

  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "memory_forget",
      payload: { key: "style", scope: "workspace" },
    }),
  );
});

test("search includes hidden facts and changing the query resets the result preview", async () => {
  const user = userEvent.setup();
  mockCommand("agent_session", () => overview(), "memory_overview");
  renderWithProviders(<MemoryPane />);
  const search = await screen.findByPlaceholderText("搜索事实");

  await user.click(screen.getByRole("button", { name: "显示更多记忆事实" }));
  await user.type(search, "fact-");
  expect(screen.getByText("fact-5")).toBeInTheDocument();
  expect(screen.queryByText("fact-6")).not.toBeInTheDocument();
  await user.type(search, "13");
  expect(screen.getByText("fact-13")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "显示更多记忆事实" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "清除搜索" }));
  expect(screen.getByText("fact-5")).toBeInTheDocument();
  expect(screen.queryByText("fact-6")).not.toBeInTheDocument();
});

test("deleting the only search result leaves search available when the total drops to five", async () => {
  const user = userEvent.setup();
  const data = overview(2, 6);
  mockCommand("agent_session", () => structuredClone(data), "memory_overview");
  mockCommand("agent_session", (args) => {
    const { id } = args?.payload as { id: number };
    data.facts = data.facts.filter((fact) => fact.id !== id);
    return { success: true, removed: true };
  }, "memory_remove_fact");
  renderWithProviders(<MemoryPane />);
  await user.type(await screen.findByPlaceholderText("搜索事实"), "fact-6");
  const row = screen.getByText("fact-6").closest(".group")!;
  const remove = row.querySelector<HTMLButtonElement>('button[title="删除"]')!;
  await user.click(remove);
  await user.click(await screen.findByRole("button", { name: "删除" }));
  await waitFor(() => expect(screen.queryByText("fact-6")).not.toBeInTheDocument());
  expect(screen.getByPlaceholderText("搜索事实")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "清除搜索" }));
  expect(screen.getByText("fact-5")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "显示更多记忆事实" })).not.toBeInTheDocument();
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "memory_remove_fact", payload: { id: 6 } });
});

/** Turning it on is the one switch that costs a download, so it asks first,
 *  then reports bytes while the fetch runs. */
test("semantic recall asks before the download and shows its progress", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  // 50 MB of a 241 MB model, then done.
  let phase = "downloading";
  mockCommand(
    "agent_session",
    () =>
      phase === "ready"
        ? { phase: "ready", bytes: 241 * 1024 * 1024, total: 241 * 1024 * 1024 }
        : { phase, bytes: 50 * 1024 * 1024, total: 241 * 1024 * 1024 },
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  expect(row).toHaveTextContent("本地小型语义模型");
  await user.click(row.querySelector('[role="switch"]')!);

  // Nothing is written until the download is agreed to.
  expect(await screen.findByText("启用语义召回")).toBeInTheDocument();
  expect(tauri.invoke).not.toHaveBeenCalledWith("agent_session", {
    op: "set_default_prefs",
    payload: expect.objectContaining({ semantic_recall: true }),
  });

  await user.click(screen.getByRole("button", { name: "下载并启用" }));
  // While the weights arrive the row carries a bar and the percentage, and 取消
  // takes the switch's place: nothing is a choice until the model has landed.
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("21%"));
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "21");
  expect(screen.getByText("已下载 50 MB / 241 MB")).toBeInTheDocument();
  expect(within(row as HTMLElement).getByRole("button", { name: "取消" })).toBeInTheDocument();
  expect(row.querySelector('[role="switch"]')).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "卸载本地模型" })).not.toBeInTheDocument();

  // The model lands: the switch comes back, names the model, and 卸载 appears.
  phase = "ready";
  await waitFor(() => expect(row.querySelector('[role="switch"]')).not.toBeDisabled(), {
    timeout: 4000,
  });
  expect(row).toHaveTextContent("paraphrase-multilingual-MiniLM-L12-v2");
  expect(screen.getByRole("button", { name: "卸载本地模型" })).toBeInTheDocument();
});
test("a failed download keeps the reason in the row, with a retry", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "failed", bytes: 0, total: 241 * 1024 * 1024, error: "网络不可达" }),
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(await screen.findByRole("button", { name: "下载并启用" }));
  // The poll stops on failure, so the row carries the reason itself — it used
  // to keep claiming 下载中, or lose the error on the next remount.
  expect(await screen.findByText("网络不可达")).toBeInTheDocument();
  expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  // The switch reads off (the model is not in effect) and is the retry: it
  // used to be a separate button, or a dead re-save of the preference.
  const retry = row.querySelector('[role="switch"]') as HTMLElement;
  expect(retry).not.toBeDisabled();
  await user.click(retry);
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "semantic_retry" });
});

/** Finished a download in THIS sitting, then switched it off and on: the pane
 *  has to remember the weights landed, or it asks to download them again — and
 *  drops 卸载 with the same mistake. */
test("a finished download is not offered again after switching it off", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  let phase = "downloading";
  mockCommand(
    "agent_session",
    () =>
      phase === "ready"
        ? { phase: "ready", bytes: 241 * 1024 * 1024, total: 241 * 1024 * 1024 }
        : { phase, bytes: 20 * 1024 * 1024, total: 241 * 1024 * 1024 },
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(await screen.findByRole("button", { name: "下载并启用" }));

  // Wait for the model to land: the switch comes back (it is replaced by 取消
  // while the fetch runs) and the bar goes away.
  phase = "ready";
  await waitFor(() => expect(row.querySelector('[role="switch"]')).not.toBeDisabled(), {
    timeout: 4000,
  });
  // 卸载 belongs to the weights, so it stays while the feature is off.
  expect(screen.getByRole("button", { name: "卸载本地模型" })).toBeInTheDocument();

  await user.click(row.querySelector('[role="switch"]')!);
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_default_prefs",
      payload: expect.objectContaining({ semantic_recall: false }),
    }),
  );
  expect(screen.getByRole("button", { name: "卸载本地模型" })).toBeInTheDocument();

  // On again: no prompt, no second download.
  await user.click(row.querySelector('[role="switch"]')!);
  expect(screen.queryByText("启用语义召回")).not.toBeInTheDocument();
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
    op: "set_default_prefs",
    payload: expect.objectContaining({ semantic_recall: true }),
  });
});

/** A model already on disk must not be offered for download again. Turning the
 *  switch off and back on is not an uninstall. */
test("a model already on disk enables without the download prompt", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false, semantic_cached: true }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "loading", bytes: 241 * 1024 * 1024, total: 241 * 1024 * 1024 }),
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);

  // No prompt: the weights are here, so the switch just writes the preference.
  expect(screen.queryByText("启用语义召回")).not.toBeInTheDocument();
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
    op: "set_default_prefs",
    payload: expect.objectContaining({ semantic_recall: true }),
  });
});

/** 卸载 deletes the weights; the switch only stops using them. The icon sits
 *  beside the switch, and only when there is something on disk to delete. */
test("uninstall deletes the model and turns the feature off", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: true, semantic_ready: true, semantic_cached: true }),
    "memory_overview",
  );
  mockCommand("agent_session", () => ({ removed: true }), "semantic_remove");
  renderWithProviders(<MemoryPane />);

  await screen.findByText("语义召回");
  await user.click(screen.getByRole("button", { name: "卸载本地模型" }));
  await user.click(await screen.findByRole("button", { name: "卸载" }));

  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "semantic_remove" });
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
    op: "set_default_prefs",
    payload: expect.objectContaining({ semantic_recall: false }),
  });
});

test("turning semantic recall off needs no confirmation", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: true, semantic_ready: true }),
    "memory_overview",
  );
  renderWithProviders(<MemoryPane />);
  const row = (await screen.findByText("语义召回")).closest(".group")!;
  expect(row).toHaveTextContent("paraphrase-multilingual-MiniLM-L12-v2");
  await user.click(row.querySelector('[role="switch"]')!);
  expect(screen.queryByText("启用语义召回")).not.toBeInTheDocument();
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_default_prefs",
      payload: expect.objectContaining({ semantic_recall: false }),
    }),
  );
});

/** A build without the encoder cannot honour the switch, so it must not draw
 *  one — a toggle whose only effect is a stuck progress bar is worse than no
 *  toggle at all. */
test("a build without the encoder draws no semantic switch", async () => {
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_available: false }),
    "memory_overview",
  );
  renderWithProviders(<MemoryPane />);

  await screen.findByText("模型蒸馏");
  expect(screen.queryByText("语义召回")).not.toBeInTheDocument();
});

test("a failed fetch offers an explicit 重试 that asks the backend to fetch again", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "failed", bytes: 0, total: 241 * 1024 * 1024, error: "网络不可达" }),
    "semantic_progress",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "downloading", bytes: 0, total: 241 * 1024 * 1024 }),
    "semantic_retry",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(await screen.findByRole("button", { name: "下载并启用" }));
  await user.click(await within(row as HTMLElement).findByRole("button", { name: "重试" }));

  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "semantic_retry" });
});

/** Off, then on, after the model finished loading in this sitting: the encoder
 *  is still in memory, so the switch must come back at once with no busy state. */
test("switching off and back on after a finished load shows no busy state", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "ready", bytes: 241 * 1024 * 1024, total: 241 * 1024 * 1024 }),
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(await screen.findByRole("button", { name: "下载并启用" }));
  await waitFor(() => expect(row).toHaveTextContent("paraphrase-multilingual-MiniLM-L12-v2"));

  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(row.querySelector('[role="switch"]')!);

  // Synchronously after the click, and again once a slow load would have shown.
  expect(screen.queryByText("启用语义召回")).not.toBeInTheDocument();
  expect(screen.queryByText(/下载中|加载中/)).not.toBeInTheDocument();
  expect(row.querySelector('[role="switch"]')).not.toBeDisabled();
  expect(row.querySelector('[role="switch"]')).toHaveAttribute("aria-checked", "true");
  await settle(400);
  expect(screen.queryByText(/下载中|加载中/)).not.toBeInTheDocument();
  expect(row.querySelector('[role="switch"]')).not.toBeDisabled();
  expect(row.querySelector('[role="switch"]')).toHaveAttribute("aria-checked", "true");
});

test("a cached model that is not loaded yet shows 正在加载模型… and never 下载中", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false, semantic_cached: true }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "loading", bytes: 241 * 1024 * 1024, total: 241 * 1024 * 1024 }),
    "semantic_progress",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  expect(screen.queryByText(/下载中/)).not.toBeInTheDocument();
  expect(screen.queryByText("正在加载模型…")).not.toBeInTheDocument();

  // Only once the wait is real does the row say so, and the switch locks.
  await settle(400);
  expect(await screen.findByText("正在加载模型…")).toBeInTheDocument();
  expect(screen.queryByText(/下载中/)).not.toBeInTheDocument();
  expect(row.querySelector('[role="switch"]')).toBeDisabled();
});

test("取消 during the download stops the fetch and writes the preference off", async () => {
  const user = userEvent.setup();
  mockCommand(
    "agent_session",
    () => ({ ...overview(0, 0), semantic: false, semantic_ready: false }),
    "memory_overview",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "downloading", bytes: 20 * 1024 * 1024, total: 241 * 1024 * 1024 }),
    "semantic_progress",
  );
  mockCommand(
    "agent_session",
    () => ({ phase: "idle", bytes: 20 * 1024 * 1024, total: 241 * 1024 * 1024 }),
    "semantic_cancel",
  );
  renderWithProviders(<MemoryPane />);

  const row = (await screen.findByText("语义召回")).closest(".group")!;
  await user.click(row.querySelector('[role="switch"]')!);
  await user.click(await screen.findByRole("button", { name: "下载并启用" }));
  await user.click(await within(row as HTMLElement).findByRole("button", { name: "取消" }));

  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", { op: "semantic_cancel" });
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_default_prefs",
      payload: expect.objectContaining({ semantic_recall: false }),
    }),
  );
  expect(row.querySelector('[role="switch"]')).toHaveAttribute("aria-checked", "false");
});

