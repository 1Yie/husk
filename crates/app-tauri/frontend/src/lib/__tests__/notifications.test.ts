import { beforeEach, describe, expect, test, vi } from "vitest";
import {
  clearAll,
  markAllRead,
  markSessionRead,
  notificationFor,
  notificationsNow,
  notify,
  notifyForEvent,
  setSessionTitles,
} from "../notifications";
import { initDesktopNotify, sendDesktopNotification } from "../desktop-notify";
import { emitWindowFocus, mockCommand, tauri } from "@/test/tauri";
import type { AgentEventEnvelope, UiEvent } from "@/types";

const env = (event: UiEvent, root = "/project", session = 1): AgentEventEnvelope => ({
  root,
  session,
  event,
});
const foreground = { root: "/project", id: 1 };

/** The pref read is a promise chain — one macrotask turn settles it. */
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  clearAll();
  setSessionTitles([]);
});

describe("notification rules", () => {
  test("a finished turn is a turn notice", () => {
    expect(notificationFor({ StateChanged: "Finished" })).toEqual({
      kind: "turn",
      title: "会话已完成",
    });
  });

  test("a cancelled turn is not a failure", () => {
    expect(notificationFor({ StateChanged: { Failed: "turn cancelled by user" } })).toBeNull();
    expect(notificationFor({ StateChanged: { Failed: "连接超时" } })).toEqual({
      kind: "error",
      title: "会话出错",
      body: "连接超时",
    });
  });

  test("blocked-on-the-user states ask for attention", () => {
    expect(
      notificationFor({
        StateChanged: { AwaitingToolConfirmation: { tool_name: "shell", diff_summary: "" } },
      })
    ).toEqual({ kind: "attention", title: "等待你的确认", body: "shell" });
    expect(notificationFor({ StateChanged: { AwaitingConsent: { kind: "network" } } })).toEqual({
      kind: "attention",
      title: "等待你的确认",
      body: "network",
    });
    expect(
      notificationFor({ QuestionAsked: { request_id: 7, question: "选哪个？", options: [] } })
    ).toEqual({ kind: "attention", title: "等待你的回答", body: "选哪个？" });
  });

  test("in-flight states are not worth a bell", () => {
    expect(notificationFor({ StateChanged: "Reasoning" })).toBeNull();
    expect(notificationFor({ TextDelta: { text: "嗯" } })).toBeNull();
    expect(notificationFor({ ToolCallStarted: { name: "read", args_preview: "" } })).toBeNull();
  });
});

describe("delivery", () => {
  test("the foreground session in a focused window stays silent", () => {
    expect(notifyForEvent(env({ StateChanged: "Finished" }), foreground)).toBeNull();
    expect(notificationsNow()).toHaveLength(0);
  });

  test("a background session notifies and reaches the shell under its name", async () => {
    setSessionTitles([{ root: "/project", sessions: [{ id: 2, title: "重构解析器" }] }]);
    const n = notifyForEvent(env({ StateChanged: "Finished" }, "/project", 2), foreground);
    expect(n).toMatchObject({ kind: "turn", title: "会话已完成", session: 2 });
    await Promise.resolve();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "notify",
      payload: {
        title: "会话已完成 — 重构解析器",
        body: undefined,
        attention: false,
        // Echoed back on a click — the toast's way into this conversation.
        target: { root: "/project", session: 2 },
      },
    });
  });

  test("a session the index does not know yet falls back to its number", async () => {
    notifyForEvent(env({ StateChanged: "Finished" }, "/project", 9), foreground);
    await Promise.resolve();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "notify",
      payload: {
        title: "会话已完成 — 会话 #9",
        body: undefined,
        attention: false,
        target: { root: "/project", session: 9 },
      },
    });
  });

  test("a prompt row goes once the session moves on, even from the foreground", () => {
    const confirm: UiEvent = {
      StateChanged: { AwaitingToolConfirmation: { tool_name: "shell", diff_summary: "" } },
    };
    notifyForEvent(env(confirm, "/project", 2), foreground);
    notifyForEvent(
      env({ QuestionAsked: { request_id: 1, question: "继续？", options: [] } }, "/b", 1),
      foreground
    );
    expect(notificationsNow().map((n) => n.kind)).toEqual(["attention", "attention"]);
    // Approved: executing is not a wait, and there is nothing new to say.
    notifyForEvent(
      env({ StateChanged: { ExecutingTool: { tool_name: "shell" } } }, "/project", 2),
      foreground
    );
    expect(notificationsNow().map((n) => n.root)).toEqual(["/b"]);
    // Answered while the user had that session open — still resolved.
    notifyForEvent(env({ QuestionAnswered: { request_id: 1 } }, "/b", 1), { root: "/b", id: 1 });
    expect(notificationsNow()).toHaveLength(0);
  });

  test("a finish replaces the session's stale prompt", () => {
    notifyForEvent(env({ StateChanged: { AwaitingConsent: { kind: "network" } } }, "/project", 2), foreground);
    notifyForEvent(env({ StateChanged: "Finished" }, "/project", 2), foreground);
    expect(notificationsNow().map((n) => [n.kind, n.read])).toEqual([["turn", false]]);
  });

  test("a hidden window notifies even for the foreground session", () => {
    const hidden = vi.spyOn(Document.prototype, "hidden", "get").mockReturnValue(true);
    expect(notifyForEvent(env({ StateChanged: "Finished" }), foreground)).not.toBeNull();
    hidden.mockRestore();
  });

  test("a window the user clicked away from notifies too", () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    expect(notifyForEvent(env({ StateChanged: "Finished" }), foreground)).not.toBeNull();
  });

  test("a minimized window notifies even when the DOM still claims focus", async () => {
    // WebKitGTK: minimizing leaves `hidden` false and `hasFocus()` true — only
    // the shell's focus report tells the truth.
    const stop = initDesktopNotify();
    await settle();
    emitWindowFocus(false);
    expect(notifyForEvent(env({ StateChanged: "Finished" }), foreground)).not.toBeNull();
    emitWindowFocus(true);
    clearAll();
    expect(notifyForEvent(env({ StateChanged: "Finished" }), foreground)).toBeNull();
    stop();
  });

  test("an enabled switch reaches the shell", async () => {
    sendDesktopNotification({ title: "构建完成", body: "3 个文件" });
    await settle();
    expect(tauri.invoke).toHaveBeenCalledWith("agent_session", {
      op: "notify",
      payload: { title: "构建完成", body: "3 个文件", attention: false, target: undefined },
    });
  });

  test("the stored switch silences the shell, not the bell", async () => {
    mockCommand("agent_session", () => ({ desktop_notifications: false }), "get_default_prefs");
    const stop = initDesktopNotify();
    await settle();
    stop();

    notifyForEvent(env({ StateChanged: "Finished" }, "/parked", 4), foreground);
    // The in-app row is there either way — only the shell is spared.
    expect(notificationsNow()).toHaveLength(1);
    expect(tauri.invoke).not.toHaveBeenCalledWith(
      "agent_session",
      expect.objectContaining({ op: "notify" })
    );

    // Back to the default so the module flag does not leak into later tests.
    mockCommand("agent_session", () => ({ desktop_notifications: true }), "get_default_prefs");
    const back = initDesktopNotify();
    await settle();
    back();
  });
});

describe("store", () => {
  test("one row per session, carrying its latest state", () => {
    notify({ kind: "turn", title: "会话已完成", root: "/a", session: 1 });
    notify({ kind: "error", title: "会话出错", root: "/a", session: 1 });
    notify({ kind: "turn", title: "会话已完成", root: "/a", session: 2 });
    notify({ kind: "attention", title: "等待你的确认", root: "/b", session: 1 });
    notify({ kind: "attention", title: "等待你的回答", root: "/b", session: 1 });
    expect(notificationsNow().map((n) => [n.kind, n.title, n.session])).toEqual([
      ["attention", "等待你的回答", 1],
      ["turn", "会话已完成", 2],
      ["error", "会话出错", 1],
    ]);
  });

  test("reading a conversation reads everything it raised", () => {
    notify({ kind: "turn", title: "会话已完成", root: "/a", session: 2 });
    notify({ kind: "error", title: "会话出错", root: "/a", session: 1 });
    notify({ kind: "turn", title: "会话已完成", root: "/b", session: 1 });
    markSessionRead("/a", 1);
    expect(notificationsNow().map((n) => [n.root, n.session, n.read])).toEqual([
      ["/b", 1, false],
      ["/a", 1, true],
      ["/a", 2, false],
    ]);
    markAllRead();
    expect(notificationsNow().every((n) => n.read)).toBe(true);
    clearAll();
    expect(notificationsNow()).toHaveLength(0);
  });
});
