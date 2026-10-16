import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { InstructionsPane } from "../index";
import { deferred, mockCommand, tauri } from "@/test/tauri";

const initial = {
  global: { path: "/config/AGENTS.md", content: "原有全局指令" },
  workspace: { path: "/project/AGENTS.md", content: "原有项目指令" },
};

describe("instructions editor saving", () => {
  test("keeps edits made during a save dirty and saves the new draft with the shortcut", async () => {
    const pending = deferred<{ success: boolean }>();
    mockCommand("agent_session", () => initial, "get_instructions");
    mockCommand("agent_session", () => pending.promise, "set_instructions");
    render(<InstructionsPane />);

    const editor = await screen.findByRole("textbox", { name: "全局指令" });
    const save = screen.getByRole("button", { name: "保存全局指令" });
    expect(save).toBeDisabled();
    editor.focus();
    fireEvent.change(editor, { target: { value: "第一版" } });
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    expect(save).toBeDisabled();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "set_instructions", payload: { scope: "global", content: "第一版" },
    });

    fireEvent.change(editor, { target: { value: "第二版" } });
    await act(async () => { pending.resolve({ success: true }); });
    expect(screen.getByRole("textbox", { name: "全局指令" })).toBe(editor);
    expect(editor).toHaveFocus();
    expect(editor).toHaveValue("第二版");
    expect(save).toBeEnabled();
    expect(screen.getByText("未保存")).toBeInTheDocument();

    mockCommand("agent_session", () => ({ success: true }), "set_instructions");
    fireEvent.keyDown(editor, { key: "s", metaKey: true });
    await waitFor(() => expect(save).toBeDisabled());
    expect(tauri.invoke).toHaveBeenLastCalledWith("agent_session", {
      op: "set_instructions", payload: { scope: "global", content: "第二版" },
    });
    await waitFor(() => expect(screen.queryByText("未保存")).not.toBeInTheDocument());
  });

  test("tracks concurrent global and project saves independently and retains a failed draft", async () => {
    const globalSave = deferred<{ success: boolean }>();
    const projectSave = deferred<{ success: boolean }>();
    mockCommand("agent_session", () => initial, "get_instructions");
    mockCommand("agent_session", (args) => {
      const payload = args?.payload as { scope: string };
      return payload.scope === "global" ? globalSave.promise : projectSave.promise;
    }, "set_instructions");
    render(<InstructionsPane />);

    const global = await screen.findByRole("textbox", { name: "全局指令" });
    const project = screen.getByRole("textbox", { name: "项目指令" });
    const saveGlobal = screen.getByRole("button", { name: "保存全局指令" });
    const saveProject = screen.getByRole("button", { name: "保存项目指令" });
    fireEvent.change(global, { target: { value: "全局改动" } });
    fireEvent.change(project, { target: { value: "项目改动" } });
    fireEvent.click(saveGlobal);
    fireEvent.click(saveProject);

    await act(async () => { projectSave.resolve({ success: true }); });
    expect(saveGlobal).toBeDisabled();
    expect(saveGlobal).toHaveTextContent("保存中…");
    expect(saveProject).toBeDisabled();
    expect(saveProject).toHaveTextContent("保存");

    await act(async () => { globalSave.reject(new Error("write failed")); });
    expect(global).toHaveValue("全局改动");
    expect(saveGlobal).toBeEnabled();
    expect(screen.getAllByText("未保存")).toHaveLength(1);
  });

  test("retries a failed read without allowing an empty draft to overwrite the file", async () => {
    mockCommand("agent_session", () => Promise.reject(new Error("read failed")), "get_instructions");
    render(<InstructionsPane />);
    expect(await screen.findByText("无法读取指令")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();

    mockCommand("agent_session", () => initial, "get_instructions");
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByRole("textbox", { name: "全局指令" })).toHaveValue(initial.global.content);
    expect(screen.getByRole("button", { name: "保存全局指令" })).toBeDisabled();
    expect(tauri.invoke.mock.calls.some(([, args]) => args?.op === "set_instructions")).toBe(false);
  });
});
