import { act } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

// Throw exactly once from the notification half of the event pump. Before the
// `finally` guard this left `timer` null forever: the queue only grew and the
// stream appeared frozen.
const notify = vi.hoisted(() => ({ throws: 1 }));

vi.mock("@/lib/notifications", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/notifications")>();
  return {
    ...actual,
    notifyForEvent: (...args: Parameters<typeof actual.notifyForEvent>) => {
      if (notify.throws > 0) {
        notify.throws -= 1;
        throw new Error("notification boom");
      }
      return actual.notifyForEvent(...args);
    },
  };
});

import { activeViewNow, initAgentStore, useAgentStore } from "../agent-store";
import { clearAll } from "@/lib/notifications";
import { emitAgentEvent } from "@/test/tauri";
import type { UiEvent } from "@/types";

let dispose: (() => void) | undefined;

beforeEach(() => {
  notify.throws = 1;
  useAgentStore.setState(useAgentStore.getInitialState(), true);
  useAgentStore.setState({ workspaceRoot: "/project", activeId: 1 });
  clearAll();
  vi.useFakeTimers();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
});

function emit(event: UiEvent) {
  emitAgentEvent({ root: "/project", session: 1, event });
}

async function flush() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(16);
  });
}

test("a throwing notification leaves the event pump alive", async () => {
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  dispose = initAgentStore();

  emit({ UserPrompt: "第一" });
  await flush();
  expect(activeViewNow().items[0]).toMatchObject({ kind: "user", text: "第一" });
  expect(consoleError).toHaveBeenCalled();

  emit({ UserPrompt: "第二" });
  await flush();
  expect(activeViewNow().items.at(-1)).toMatchObject({ kind: "user", text: "第二" });
});
