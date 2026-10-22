// The 子代理 view: one row per running child, and the steer each one takes.

import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import { toast } from "sonner";
import { SubagentsList } from "@/features/chat/components/subagents-panel";
import { renderWithProviders } from "@/test/render";
import { mockCommand, tauri } from "@/test/tauri";
import { WAITING_STEER_HINT } from "@/lib/subagents";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const child = (childId: number, name: string, task: string) => ({ childId, name, task });

describe("子代理 view", () => {
  test("gives each running child a row: its name, its task and 运行中", () => {
    renderWithProviders(
      <SubagentsList
        children={[child(1, "review #1", "检查鉴权"), child(2, "review #2", "看性能")]}
      />,
    );
    expect(screen.getByText("review #1")).toBeInTheDocument();
    expect(screen.getByText("检查鉴权")).toBeInTheDocument();
    expect(screen.getByText("review #2")).toBeInTheDocument();
    expect(screen.getAllByText("运行中")).toHaveLength(2);
  });

  test("a steer typed against a child reaches the kernel", async () => {
    const user = userEvent.setup();
    renderWithProviders(<SubagentsList children={[child(3, "review #1", "检查鉴权")]} />);
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    expect(tauri.invoke).toHaveBeenCalledWith("agent_cmd", {
      cmd: { SteerChild: { child_id: 3, text: "先看中间件" } },
    });
    // The kernel echoes the steer back as a `subagent:steer` custom event, so
    // the box empties and nothing is drawn locally.
    await waitFor(() => expect(box).toHaveValue(""));
  });

  test("a rejected steer is reported and the draft kept", async () => {
    mockCommand("agent_cmd", () => Promise.reject(new Error("session gone")));
    const user = userEvent.setup();
    renderWithProviders(<SubagentsList children={[child(3, "review #1", "检查鉴权")]} />);
    const box = screen.getByPlaceholderText("插话…");
    await user.type(box, "先看中间件{Enter}");
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("插话失败：Error: session gone"));
    expect(box).toHaveValue("先看中间件");
  });

  test("a parked child shows its question and says the steer waits", () => {
    renderWithProviders(
      <SubagentsList
        children={[{ ...child(4, "review #1", "检查鉴权"), waiting: "用哪条分支？" }]}
      />,
    );
    expect(screen.getByText("等待主代理回复")).toBeInTheDocument();
    expect(screen.getByText("用哪条分支？")).toBeInTheDocument();
    expect(screen.getByText(WAITING_STEER_HINT)).toBeInTheDocument();
  });
});
