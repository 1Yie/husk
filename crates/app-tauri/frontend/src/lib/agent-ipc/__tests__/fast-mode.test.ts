import { expect, test } from "vitest";
import { setFastMode } from "../sessions";
import { mockCommand, tauri } from "@/test/tauri";

test("Fast targets a workspace/session and returns the acknowledged preference", async () => {
  mockCommand("agent_session", () => ({ fast_mode: true }), "set_fast_mode");
  await expect(setFastMode(true, 7, "/repo")).resolves.toEqual({ fast_mode: true });
  expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
    op: "set_fast_mode", id: 7, path: "/repo", payload: { enabled: true },
  });
});

test("a rejected Fast toggle reaches the caller", async () => {
  mockCommand("agent_session", () => { throw new Error("Fast 未配置"); }, "set_fast_mode");
  await expect(setFastMode(true, 7, "/repo")).rejects.toThrow("Fast 未配置");
});
