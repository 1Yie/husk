// Agent events — kernel → webview subscription (`agent://event`).
//
// The forwarder emits `{ session, event }` envelopes; this wraps `listen`
// so hooks subscribe to typed envelopes, not the raw channel name.

import { listen } from "@tauri-apps/api/event";
import type { AgentEventEnvelope } from "../../types";

export function onAgentEvent(
  handler: (envelope: AgentEventEnvelope) => void,
): Promise<() => void> {
  return listen<AgentEventEnvelope>("agent://event", (e) => handler(e.payload));
}
