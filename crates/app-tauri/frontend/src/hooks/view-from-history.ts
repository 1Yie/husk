// History → view — rebuild a `SessionView` from the persisted
// `ChatMessage` snapshot when a session is reopened. Must render the
// same stream the live `applyEvent` path showed: failed tool capsules
// stay red (`is_error`), turn-end salvage markers become system lines,
// steer annotations strip to the bare text.

import * as agent from "../invoke/agent";
import { emptyView, type SessionView } from "./stream-view";
import type { ChatMessage } from "../types";

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

/** Turn-end markers the engine appends to salvaged assistant text
 * (`engine.rs` writes `*[cancelled by user]*`, `*[interrupted — transport
 * error]*`, `*[error: …]*`; the live stream showed them as separate
 * system lines). Replay re-splits them into `system` items so a reloaded
 * session matches what was on screen — without this the marker ends up as
 * italic text inside the bubble and the system line is lost. */
const SALVAGE_MARKERS: { re: RegExp; line: (m: RegExpMatchArray) => string }[] = [
  { re: /\n?\n?\*\[cancelled by user\]\*\s*$/, line: () => "已被用户中断" },
  {
    re: /\n?\n?\*\[interrupted — transport error\]\*\s*$/,
    line: () => "流传输中断 — 已保留部分内容",
  },
  { re: /\n?\n?\*\[error: ([\s\S]*?)\]\*\s*$/, line: (m) => `⚠ ${m[1]}` },
];

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

/** `batch_execute` args → `N calls` chip text for the parent capsule. */
function batchCallCount(rawArgs: string): string {
  try {
    const parsed = JSON.parse(rawArgs);
    const n = Array.isArray(parsed?.calls) ? parsed.calls.length : 0;
    return n > 0 ? `${n} calls` : "";
  } catch {
    return "";
  }
}

/** Rebuild a batch's nested tool items from its persisted args (`calls[]`)
 * and result text (`── [i] tool (status) ──` sections — written to be
 * machine-parseable for exactly this replay). Section names/status are
 * authoritative; `calls[i].args` only supplies the chip preview. */
function batchChildren(rawArgs: string, content: string) {
  let calls: { tool?: string; args?: unknown }[] = [];
  try {
    const parsed = JSON.parse(rawArgs);
    if (Array.isArray(parsed?.calls)) calls = parsed.calls;
  } catch {
    /* malformed args — sections below still carry names/status */
  }
  // Split on section headers; parts[0] is the `[batch_execute — …]` summary.
  const parts = content.split(/\n── \[(\d+)\] /);
  const items = [];
  for (let i = 1; i + 1 < parts.length; i += 2) {
    const idx = Number(parts[i]);
    const rest = parts[i + 1];
    const m = rest.match(/^(\S+) \(([^)]*)\) ──[^\n]*\n?([\s\S]*)$/);
    if (!m) continue;
    const [, tool, status, bodyRaw] = m;
    // Drop the trailing cap-warning line — it belongs to the batch as a
    // whole, not this item's body.
    const body = bodyRaw
      .replace(/\n?⚠ batch output cap reached[^\n]*$/s, "")
      .trim();
    const ok = status.startsWith("ok");
    const callArgs = calls[idx]?.args;
    items.push({
      kind: "tool" as const,
      name: tool,
      args:
        callArgs === undefined
          ? ""
          : extractArgsPreview(JSON.stringify(callArgs)),
      content: body || undefined,
      ok,
      parent: "batch_execute",
    });
  }
  return items;
}

/** Rebuild a `SessionView`'s item list from persisted `ChatMessage`
 * history — used when switching to a session whose in-memory view was
 * never built (first open after launch) or dropped. Tool result messages
 * map to finished tool capsules; reasoning isn't persisted, so nothing
 * thinking-shaped is emitted. `usage` is the persisted last-turn meter —
 * seeds the header stats so a reopened session doesn't read zeros until
 * its next `Usage` event. */
export function viewFromHistory(
  history: ChatMessage[],
  usage?: agent.SessionUsage | null,
  baseIndex = 0,
): SessionView {
  const v = emptyView();
  if (usage) {
    v.usage = {
      prompt: usage.prompt,
      completion: usage.completion,
      contextWindow: usage.context_window,
      cachedTokens: usage.cached ?? 0,
    };
  }
  // call_id → tool name and args: resolve from the preceding assistant `tool_calls`.
  const idToTool = new Map<string, { name: string; args: string; raw: string }>();
  for (const m of history) indexToolCalls(idToTool, m);
  history.forEach((m, i) => foldMessage(v, idToTool, m, baseIndex + i));
  return v;
}

/** Chunked variant — folds ~24 messages per macrotask so a session
 * rebuild doesn't monopolize the main thread: sidebar/settings input
 * keeps dispatching between slices. Same output as `viewFromHistory`. */
export async function viewFromHistoryChunked(
  history: ChatMessage[],
  usage?: agent.SessionUsage | null,
  baseIndex = 0,
): Promise<SessionView> {
  const v = emptyView();
  if (usage) {
    v.usage = {
      prompt: usage.prompt,
      completion: usage.completion,
      contextWindow: usage.context_window,
      cachedTokens: usage.cached ?? 0,
    };
  }
  const idToTool = new Map<string, { name: string; args: string; raw: string }>();
  for (const m of history) indexToolCalls(idToTool, m);
  for (let i = 0; i < history.length; i++) {
    foldMessage(v, idToTool, history[i], baseIndex + i);
    if (i % 24 === 23) await new Promise((r) => setTimeout(r, 0));
  }
  return v;
}

type ToolInfo = { name: string; args: string; raw: string };

/** call_id → tool name/args index — one pass over `tool_calls` on the
 * preceding assistant messages. */
function indexToolCalls(idToTool: Map<string, ToolInfo>, m: ChatMessage) {
  for (const c of m.tool_calls ?? []) {
    const raw = c.function.arguments ?? "";
    idToTool.set(c.id, {
      name: c.function.name,
      // batch_execute's own preview is the call count — the real
      // per-item args live inside `calls[]`.
      args:
        c.function.name === "batch_execute"
          ? batchCallCount(raw)
          : extractArgsPreview(raw),
      raw,
    });
  }
}

/** Fold one persisted message into the view's item list — shared by the
 * sync and time-sliced builders. */
function foldMessage(
  v: SessionView,
  idToTool: Map<string, ToolInfo>,
  m: ChatMessage,
  hi: number,
) {
    // `notice` marks the system lines the live stream actually showed —
    // replayed verbatim. Everything else (system prompt, compaction
    // note, hook injections) is internal context and stays hidden.
    if (m.role === "system") {
      if (m.notice === "system")
        v.items.push({ kind: "system", text: m.content ?? "", hi });
      else if (m.notice === "error")
        v.items.push({ kind: "system", text: `⚠ ${m.content ?? ""}`, hi });
      return;
    }
    if (m.role === "tool") {
      const toolInfo = (m.tool_call_id && idToTool.get(m.tool_call_id)) || {
        name: "tool",
        args: "",
        raw: "",
      };
      v.items.push({
        kind: "tool",
        hi,
        name: toolInfo.name,
        args: toolInfo.args,
        content: m.content ?? "",
        // `is_error` is persisted on failed results (veto/deny/dispatch
        // error/cancelled dispatch) — replay the failed capsule the live
        // `ToolCallFinished.ok` showed instead of a green one.
        ok: m.is_error !== true,
      });
      // A batch's inner items were live-only events — they're not in
      // history. Rebuild them from the persisted call list + the
      // `── [i] tool (status) ──` result sections so the nested capsule
      // survives reload.
      if (toolInfo.name === "batch_execute") {
        for (const child of batchChildren(toolInfo.raw, m.content ?? "")) {
          v.items.push({ ...child, hi });
        }
      }
      return;
    }
    let text =
      m.role === "assistant" ? stripCallEcho(m.content ?? "") : (m.content ?? "");
    let markerLine: string | null = null;
    if (m.role === "assistant") {
      for (const { re, line } of SALVAGE_MARKERS) {
        const hit = text.match(re);
        if (hit) {
          text = text.slice(0, hit.index).trimEnd();
          markerLine = line(hit);
          break;
        }
      }
    }
    if (m.role === "user") {
      // `hidden` marks engine-injected instructions (tool-limit nudge,
      // synthesis prompt) — the provider sees them but the live stream
      // never drew a bubble, so replay must skip them too.
      if (m.notice === "hidden") return;
      // Mid-turn steering persists as "The user interrupted: X" — the
      // live stream echoes the bare text, so strip the annotation.
      const steer = text.match(/^The user interrupted: ([\s\S]*)$/);
      if (steer) text = steer[1].trim();
    }
    // Assistant messages that only carry `tool_calls` have no visible text —
    // the matching `tool` result already renders the capsule.
    if (!text.trim()) {
      if (markerLine) v.items.push({ kind: "system", text: markerLine, hi });
      return;
    }
    if (m.role === "user") {
      v.items.push({ kind: "user", text, ts: m.ts ?? undefined, hi });
    } else {
      v.items.push({ kind: "assistant", text, streaming: false, hi });
      if (markerLine) v.items.push({ kind: "system", text: markerLine, hi });
    }
}
