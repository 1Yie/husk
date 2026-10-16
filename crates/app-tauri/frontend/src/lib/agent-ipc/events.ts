// Agent events — kernel → webview subscription (`agent://event`).
//
// The forwarder emits `{ session, event }` envelopes; this wraps `listen`
// so hooks subscribe to typed envelopes, not the raw channel name.

import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { AgentEventEnvelope } from "@/types";
import type { NotificationTarget } from "@/lib/agent-ipc/sessions";

export function onAgentEvent(
  handler: (envelope: AgentEventEnvelope) => void,
): Promise<() => void> {
  return listen<AgentEventEnvelope>("agent://event", (e) => handler(e.payload));
}

// Serialize navigation across listener replacements as well as event bursts.
// A stopped consumer leaves an unread click in the backend for its successor.
let notificationNavigation = Promise.resolve();

/** Subscribe before draining queued activations. Acknowledge each click only
 *  after navigation, so startup, remounts and concurrent clicks cannot lose or
 *  duplicate the target. An AbortSignal can stop even an unfinished subscribe. */
export async function onNotificationClick(
  handler: (target: NotificationTarget) => void | Promise<void>,
  signal?: AbortSignal,
): Promise<() => void> {
  let disposed = false;
  const active = () => !disposed && !signal?.aborted;
  const drain = () => {
    const navigation = notificationNavigation.then(async () => {
      while (active()) {
        const click = await invoke<{ id: number; target: NotificationTarget } | null>("agent_session", {
          op: "peek_notification_click",
        });
        if (!active() || !click) return;
        await handler(click.target);
        await invoke("agent_session", { op: "ack_notification_click", payload: { id: click.id } });
      }
    });
    // Keep the queue usable after a failed navigation while letting this caller
    // surface the error. Its unacknowledged target remains available to retry.
    notificationNavigation = navigation.catch(() => {});
    return navigation;
  };
  const stopListening = await listen<NotificationTarget>("husk://notification-click", () => {
    void drain().catch((e) => {
      if (active()) console.error("notification click failed:", e);
    });
  });
  const stop = () => {
    if (disposed) return;
    disposed = true;
    stopListening();
    signal?.removeEventListener("abort", stop);
  };
  signal?.addEventListener("abort", stop, { once: true });
  if (!active()) {
    stop();
    return stop;
  }
  try {
    await drain();
  } catch (e) {
    stop();
    throw e;
  }
  return stop;
}
