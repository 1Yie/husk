import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import { MemoryPane } from "../index";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import type { MemoryOverview } from "@/lib/agent-ipc/sessions";

function overview(personaCount = 12, factCount = 13): MemoryOverview {
  return {
    enabled: true,
    distill: true,
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
