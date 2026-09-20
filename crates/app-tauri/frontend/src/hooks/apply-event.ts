// Event → view — the live-stream reducer. One `UiEvent` folds into the
// per-session `SessionView`; paired with `viewFromHistory` it is the
// other half of "same stream, two sources".

import type { AgentEventEnvelope } from "../types";
import type { SessionView, StreamItem } from "./stream-view";

function closeOpenThinking(items: StreamItem[]) {
  for (let i = 0; i < items.length; i++) {
    const it = items[i];
    if (it.kind === "thinking" && !it.done) {
      items[i] = { ...it, done: true };
    }
  }
}

/** Fold a streamed text delta into the rate accumulator — reasoning
 * counts toward completion tokens too, so both delta kinds feed it. */
function bumpRate(v: SessionView, text: string): SessionView["rate"] {
  const now = Date.now();
  const gap = v.rate.lastAt == null ? 0 : Math.min(now - v.rate.lastAt, 1000);
  return {
    chars: v.rate.chars + text.length,
    activeMs: v.rate.activeMs + gap,
    lastAt: now,
  };
}

/** Live tok/s estimate from streamed chars (~4 chars/token). */
function rateEstimate(rate: SessionView["rate"]): number {
  return rate.activeMs > 0 ? rate.chars / 4 / (rate.activeMs / 1000) : 0;
}

export function applyEvent(
  v: SessionView,
  ev: AgentEventEnvelope["event"]
): SessionView {
  const items = [...v.items];
  const last = () => items[items.length - 1];

  if ("StateChanged" in ev) {
    const s = ev.StateChanged;
    // Mirror the kernel's `AgentState::is_active()` — the reply-wait pill
    // and the steer composer must stay lit through EVERY mid-turn state
    // (Compacting, AwaitingConsent, Branching, ScanningWorkspace), not
    // just the four that produce deltas. The old narrow set made
    // "正在回复" vanish for seconds during mid-turn auto-compaction.
    const streaming =
      s !== "Idle" &&
      s !== "Finished" &&
      !(typeof s === "object" && "Failed" in s);
    return { ...v, state: s, streaming };
  }
  if ("UserPrompt" in ev) {
    items.push({ kind: "user", text: ev.UserPrompt, ts: Date.now() });
    return {
      ...v,
      items,
      toksPerSec: 0,
      rate: { chars: 0, activeMs: 0, lastAt: null },
    };
  }
  if ("TextDelta" in ev) {
    closeOpenThinking(items);
    const rate = bumpRate(v, ev.TextDelta);
    const l = last();
    if (l?.kind === "assistant")
      items[items.length - 1] = { ...l, text: l.text + ev.TextDelta, streaming: true };
    else items.push({ kind: "assistant", text: ev.TextDelta, streaming: true });
    return { ...v, items, rate, toksPerSec: rateEstimate(rate) };
  }
  if ("ReasoningDelta" in ev) {
    const rate = bumpRate(v, ev.ReasoningDelta);
    const l = last();
    if (l?.kind === "thinking" && !l.done)
      items[items.length - 1] = { ...l, text: l.text + ev.ReasoningDelta };
    else items.push({ kind: "thinking", text: ev.ReasoningDelta, done: false });
    return { ...v, items, rate, toksPerSec: rateEstimate(rate) };
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
    // Replace the chars/4 estimate with the exact decode rate now that
    // real completion_tokens exist.
    const secs = v.rate.activeMs / 1000;
    const toksPerSec =
      secs > 0 && ev.Usage.completion_tokens > 0
        ? ev.Usage.completion_tokens / secs
        : v.toksPerSec;
    return {
      ...v,
      toksPerSec,
      usage: {
        prompt: ev.Usage.prompt_tokens,
        completion: ev.Usage.completion_tokens,
        contextWindow: ev.Usage.context_window,
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
