// Agent IPC hooks — the React-facing boundary over the Tauri bridge.

import { useCallback, useEffect, useState } from "react";
import * as agent from "../invoke/agent";
import type { AgentEventEnvelope, AgentState, ChatMessage, SessionRow } from "../types";

export type StreamItem =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; streaming: boolean }
  | { kind: "thinking"; text: string; done: boolean }
  | {
      kind: "tool";
      name: string;
      args: string;
      content?: string;
      ok?: boolean;
      uiType?: string;
      approval?: {
        requestId: number;
        diff?: string;
        fuzzy?: boolean;
        resolved?: boolean;
        approved?: boolean;
      };
    }
  | {
      kind: "approval";
      requestId: number;
      toolName: string;
      diff: string;
      fuzzy: boolean;
      resolved?: boolean;
      approved?: boolean;
    }
  | { kind: "system"; text: string };

export interface SessionView {
  items: StreamItem[];
  state: AgentState | null;
  streaming: boolean;
  usage: { prompt: number; completion: number };
}

const emptyView = (): SessionView => ({
  items: [],
  state: null,
  streaming: false,
  usage: { prompt: 0, completion: 0 },
});

/** The pending approval a composer permission strip should surface —
 * the most recent unresolved `ApprovalRequested`. `ToolCallStarted`
 * marks it resolved/approved, `ToolDecision` (deny) resolves via state
 * transitions. Returns null when nothing is awaiting a verdict. */
export function pendingApprovalOf(view: SessionView): {
  requestId: number;
  toolName: string;
  args?: string;
  diff: string;
} | null {
  for (let i = view.items.length - 1; i >= 0; i--) {
    const it = view.items[i];
    if (it.kind === "tool" && it.approval && !it.approval.resolved) {
      return {
        requestId: it.approval.requestId,
        toolName: it.name,
        args: it.args,
        diff: it.approval.diff || "",
      };
    }
    if (it.kind === "approval" && !it.resolved) {
      return {
        requestId: it.requestId,
        toolName: it.toolName,
        diff: it.diff,
      };
    }
  }
  return null;
}

/** Strip persisted `[call: …]` text-protocol echoes from an assistant
 * message — mirrors the kernel's `strip_call_echo` so a poisoned snapshot
 * doesn't render raw call syntax as chat text. */
function stripCallEcho(text: string): string {
  let out = "";
  let rest = text;
  for (;;) {
    const start = rest.indexOf("[call:");
    if (start === -1) break;
    const before = rest.slice(0, start);
    out += before.endsWith("\n") ? before.slice(0, -1) : before;
    const after = rest.slice(start);
    const end = after.indexOf("]");
    rest = end === -1 ? "" : after.slice(end + 1);
    if (rest.startsWith("\n")) rest = rest.slice(1);
  }
  return out + rest;
}

function extractArgsPreview(rawArgs?: string): string {
  if (!rawArgs || !rawArgs.trim()) return "";
  try {
    const parsed = JSON.parse(rawArgs);
    if (typeof parsed === "string") return parsed;
    if (typeof parsed === "object" && parsed !== null) {
      const candidate =
        parsed.path ||
        parsed.file ||
        parsed.file_path ||
        parsed.command ||
        parsed.cmd ||
        parsed.pattern ||
        parsed.query ||
        parsed.action ||
        parsed.target ||
        parsed.url;
      if (typeof candidate === "string") return candidate;
      if (typeof candidate === "number") return String(candidate);
      for (const val of Object.values(parsed)) {
        if (typeof val === "string" && val.trim()) return val;
      }
    }
  } catch {
    return rawArgs.trim();
  }
  return "";
}

/** Rebuild a `SessionView`'s item list from persisted `ChatMessage`
 * history — used when switching to a session whose in-memory view was
 * never built (first open after launch) or dropped. Tool result messages
 * map to finished tool capsules; reasoning isn't persisted, so nothing
 * thinking-shaped is emitted. */
export function viewFromHistory(history: ChatMessage[]): SessionView {
  const v = emptyView();
  // call_id → tool name and args: resolve from the preceding assistant `tool_calls`.
  const idToTool = new Map<string, { name: string; args: string }>();
  for (const m of history) {
    for (const c of m.tool_calls ?? []) {
      idToTool.set(c.id, {
        name: c.function.name,
        args: extractArgsPreview(c.function.arguments),
      });
    }
  }
  for (const m of history) {
    // The system prompt is internal context — never rendered.
    if (m.role === "system") continue;
    if (m.role === "tool") {
      const toolInfo = (m.tool_call_id && idToTool.get(m.tool_call_id)) || {
        name: "tool",
        args: "",
      };
      v.items.push({
        kind: "tool",
        name: toolInfo.name,
        args: toolInfo.args,
        content: m.content ?? "",
        ok: true,
      });
      continue;
    }
    const text =
      m.role === "assistant" ? stripCallEcho(m.content ?? "") : (m.content ?? "");
    // Assistant messages that only carry `tool_calls` have no visible text —
    // the matching `tool` result already renders the capsule.
    if (!text.trim()) continue;
    if (m.role === "user") {
      v.items.push({ kind: "user", text });
    } else {
      v.items.push({ kind: "assistant", text, streaming: false });
    }
  }
  return v;
}

function closeOpenThinking(items: StreamItem[]) {
  for (let i = 0; i < items.length; i++) {
    const it = items[i];
    if (it.kind === "thinking" && !it.done) {
      items[i] = { ...it, done: true };
    }
  }
}

function applyEvent(
  v: SessionView,
  ev: AgentEventEnvelope["event"]
): SessionView {
  const items = [...v.items];
  const last = () => items[items.length - 1];

  if ("StateChanged" in ev) {
    const s = ev.StateChanged;
    const streaming =
      s === "StreamingToken" ||
      s === "Reasoning" ||
      (typeof s === "object" &&
        ("ExecutingTool" in s || "AwaitingToolConfirmation" in s));
    return { ...v, state: s, streaming };
  }
  if ("UserPrompt" in ev) {
    items.push({ kind: "user", text: ev.UserPrompt });
    return { ...v, items };
  }
  if ("TextDelta" in ev) {
    closeOpenThinking(items);
    const l = last();
    if (l?.kind === "assistant")
      items[items.length - 1] = { ...l, text: l.text + ev.TextDelta, streaming: true };
    else items.push({ kind: "assistant", text: ev.TextDelta, streaming: true });
    return { ...v, items };
  }
  if ("ReasoningDelta" in ev) {
    const l = last();
    if (l?.kind === "thinking" && !l.done)
      items[items.length - 1] = { ...l, text: l.text + ev.ReasoningDelta };
    else items.push({ kind: "thinking", text: ev.ReasoningDelta, done: false });
    return { ...v, items };
  }
  if ("ToolCallStarted" in ev) {
    closeOpenThinking(items);
    const name = ev.ToolCallStarted.name;
    items.push({
      kind: "tool",
      name,
      args: ev.ToolCallStarted.args_preview,
    });
    return { ...v, items };
  }
  if ("ToolCallFinished" in ev) {
    closeOpenThinking(items);
    const t = ev.ToolCallFinished;
    // Resolve any standalone approval item first
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "approval" && !it.resolved && it.toolName === t.name) {
        items[i] = { ...it, resolved: true, approved: t.ok };
        break;
      }
    }
    // Update the running tool call and resolve its attached approval
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "tool" && it.content === undefined) {
        items[i] = {
          ...it,
          content: t.content,
          ok: t.ok,
          uiType: t.ui_type,
          approval: it.approval
            ? { ...it.approval, resolved: true, approved: t.ok }
            : undefined,
        };
        break;
      }
    }
    return { ...v, items };
  }
  if ("ApprovalRequested" in ev) {
    closeOpenThinking(items);
    const a = ev.ApprovalRequested;
    // Attach approval directly to the matching active tool call
    let attached = false;
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "tool" && it.name === a.tool_name && it.content === undefined) {
        items[i] = {
          ...it,
          approval: {
            requestId: a.request_id,
            diff: a.diff,
            fuzzy: a.fuzzy,
            resolved: false,
          },
        };
        attached = true;
        break;
      }
    }
    // Fallback: if no active tool found, push standalone approval item
    if (!attached) {
      items.push({
        kind: "approval",
        requestId: a.request_id,
        toolName: a.tool_name,
        diff: a.diff,
        fuzzy: a.fuzzy,
        resolved: false,
      });
    }
    return { ...v, items };
  }
  if ("AssistantMessage" in ev) {
    closeOpenThinking(items);
    const l = last();
    if (l?.kind === "assistant" && l.streaming) {
      items[items.length - 1] = { ...l, text: ev.AssistantMessage, streaming: false };
    } else {
      items.push({ kind: "assistant", text: ev.AssistantMessage, streaming: false });
    }
    return { ...v, items };
  }
  if ("SystemMessage" in ev) {
    closeOpenThinking(items);
    items.push({ kind: "system", text: ev.SystemMessage });
    return { ...v, items };
  }
  if ("Usage" in ev) {
    return {
      ...v,
      usage: {
        prompt: ev.Usage.prompt_tokens,
        completion: ev.Usage.completion_tokens,
      },
    };
  }
  if ("Error" in ev) {
    closeOpenThinking(items);
    items.push({ kind: "system", text: `⚠ ${ev.Error}` });
    return { ...v, items, streaming: false };
  }
  return v;
}

export function useAgentEvents() {
  const [views, setViews] = useState<Map<number, SessionView>>(new Map());
  const [activeId, setActiveId] = useState(0);

  /** Install a rebuilt view for `id` — used by `openSession` when the
   * session has no live event buffer. Never overwrites an existing view:
   * a live buffer is newer than the last persisted snapshot. */
  const loadView = useCallback((id: number, view: SessionView) => {
    setViews((m) => {
      if (m.has(id)) return m;
      const next = new Map(m);
      next.set(id, view);
      return next;
    });
  }, []);

  useEffect(() => {
    const un = agent.onAgentEvent(({ session, event }) => {
      setViews((m) => {
        const next = new Map(m);
        const v = next.get(session) ?? emptyView();
        next.set(session, applyEvent(v, event));
        return next;
      });
      setActiveId(session);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const active = views.get(activeId) ?? emptyView();
  return { views, activeId, active, setActiveId, loadView };
}

export function useAgentSession() {
  const [sessions, setSessions] = useState<SessionRow[]>([]);

  const refresh = useCallback(async () => {
    const rows = await agent.listSessions();
    setSessions(rows);
    return rows;
  }, []);

  const newSession = useCallback(async () => {
    const newId = await agent.newSession();
    await refresh();
    return newId;
  }, [refresh]);

  const openSession = useCallback(
    async (id: number) => {
      const r = await agent.openSession(id);
      await refresh();
      return r;
    },
    [refresh]
  );

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh]);

  return { sessions, refresh, newSession, openSession };
}
