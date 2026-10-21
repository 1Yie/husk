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
import { playCue } from "../notification-sound";
import { emitWindowFocus, mockCommand, tauri } from "@/test/tauri";
import type { AgentEventEnvelope, UiEvent } from "@/types";

// The cue is the point of the next tests; synthesising it needs Web Audio,
// which jsdom has no implementation of.
vi.mock("@/lib/notification-sound", () => ({ playCue: vi.fn(), initNotificationSound: vi.fn() }));

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

describe("cue sound", () => {
  const cue = vi.mocked(playCue);
  beforeEach(() => cue.mockClear());

  test("every state that stops the loop is audible, focused window or not", () => {
    // The foreground session in the window the user is looking at produces no
    // notification row — but the cue is about the loop stopping, not about
    // whether the user can see the row.
    for (const [event, expected] of [
      [{ StateChanged: "Finished" }, "turn"],
      [
        { StateChanged: { AwaitingToolConfirmation: { tool_name: "shell", diff_summary: "" } } },
        "attention",
      ],
      [{ QuestionAsked: { request_id: 7, question: "选哪个？", options: [] } }, "attention"],
      [{ StateChanged: { Failed: "连接超时" } }, "error"],
    ] as [UiEvent, string][]) {
      cue.mockClear();
      notifyForEvent(env(event), foreground);
      expect(cue).toHaveBeenCalledWith(expected);
    }
  });

  test("work in flight and the user's own cancel stay quiet", () => {
    for (const event of [
      { StateChanged: "Reasoning" },
      { TextDelta: { text: "嗯" } },
      { StateChanged: { Failed: "turn cancelled by user" } },
    ] satisfies UiEvent[]) {
      notifyForEvent(env(event), foreground);
    }
    expect(cue).not.toHaveBeenCalled();
  });

  test("a session the user cannot see still rings", () => {
    notifyForEvent(env({ StateChanged: "Finished" }, "/project", 9), foreground);
    expect(cue).toHaveBeenCalledWith("turn");
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

describe("background job failures", () => {
  const snapshot = (...jobs: [string, string, number | null][]): UiEvent => ({
    Custom: {
      kind: "husk:bg_jobs",
      payload: {
        jobs: jobs.map(([id, label, code]) => ({
          id,
          command: `cmd ${id}`,
          state:
            label === "running"
              ? { kind: "running" }
              : {
                  kind: "done",
                  label,
                  code,
                  reason: code != null ? "exited" : label.includes("timeout") ? "timeout" : "killed",
                },
          started_at_ms: 1,
          elapsed_ms: 0,
          notify: true,
          has_own_sink: false,
        })),
      },
    },
  });

  test("a failure in a session the user is not looking at notifies once", async () => {
    setSessionTitles([{ root: "/project", sessions: [{ id: 21, title: "构建" }] }]);
    vi.mocked(playCue).mockClear();
    expect(notifyForEvent(env(snapshot(["bg-1", "running", null]), "/project", 21), foreground)).toBeNull();
    const n = notifyForEvent(env(snapshot(["bg-1", "exited(101)", 101]), "/project", 21), foreground);
    expect(n).toMatchObject({ kind: "error", title: "后台任务失败", body: "cmd bg-1", session: 21 });
    expect(playCue).toHaveBeenCalledWith("error");
    await Promise.resolve();
    expect(tauri.invoke).toHaveBeenCalledWith(
      "agent_session",
      expect.objectContaining({
        op: "notify",
        payload: expect.objectContaining({
          title: "后台任务失败 — 构建",
          target: { root: "/project", session: 21 },
        }),
      }),
    );
    // Every later snapshot still lists the failure; it is not news again.
    expect(
      notifyForEvent(env(snapshot(["bg-1", "exited(101)", 101], ["bg-2", "running", null]), "/project", 21), foreground),
    ).toBeNull();
    expect(notificationsNow()).toHaveLength(1);
  });

  test("successes and the user's own kills stay quiet", () => {
    const ev = snapshot(["bg-1", "exited(0)", 0], ["bg-2", "killed(sig 9)", null]);
    expect(notifyForEvent(env(ev, "/project", 22), foreground)).toBeNull();
    expect(notificationsNow()).toHaveLength(0);
  });

  test("failures landing together are one notice; a timeout counts", () => {
    const ev = snapshot(["bg-1", "exited(1)", 1], ["bg-2", "killed(timeout)", null]);
    const n = notifyForEvent(env(ev, "/project", 23), foreground);
    expect(n).toMatchObject({ title: "2 个后台任务失败", body: "cmd bg-1\ncmd bg-2" });
  });

  test("the foreground session in a focused window leaves it to the toast", () => {
    vi.mocked(playCue).mockClear();
    expect(notifyForEvent(env(snapshot(["bg-1", "exited(1)", 1]), "/project", 1), foreground)).toBeNull();
    expect(notificationsNow()).toHaveLength(0);
    // The cue still plays: the job stopping is worth hearing either way.
    expect(playCue).toHaveBeenCalledWith("error");
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
