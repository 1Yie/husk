// History → view — rebuild a `SessionView` from the persisted
// `ChatMessage` snapshot when a session is reopened. Must render the
// same stream the live `applyEvent` path showed: failed tool capsules
// stay red (`is_error`), turn-end salvage markers become system lines,
// steer annotations strip to the bare text.

import * as agent from "../invoke/agent";
import type { ChatMessage } from "../types";
import { emptyView, type SessionView } from "./stream-view";

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

/** Rebuild a `SessionView`'s item list from persisted `ChatMessage`
 * history — used when switching to a session whose in-memory view was
 * never built (first open after launch) or dropped. Tool result messages
 * map to finished tool capsules; reasoning isn't persisted, so nothing
 * thinking-shaped is emitted. `usage` is the persisted last-turn meter —
 * seeds the header stats so a reopened session doesn't read zeros until
 * its next `Usage` event. */
export function viewFromHistory(
  history: ChatMessage[],
  usage?: agent.SessionUsage | null
): SessionView {
  const v = emptyView();
  if (usage) {
    v.usage = {
      prompt: usage.prompt,
      completion: usage.completion,
      contextWindow: usage.context_window,
    };
  }
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
    // `notice` marks the system lines the live stream actually showed —
    // replayed verbatim. Everything else (system prompt, compaction
    // note, hook injections) is internal context and stays hidden.
    if (m.role === "system") {
      if (m.notice === "system")
        v.items.push({ kind: "system", text: m.content ?? "" });
      else if (m.notice === "error")
        v.items.push({ kind: "system", text: `⚠ ${m.content ?? ""}` });
      continue;
    }
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
        // `is_error` is persisted on failed results (veto/deny/dispatch
        // error/cancelled dispatch) — replay the failed capsule the live
        // `ToolCallFinished.ok` showed instead of a green one.
        ok: m.is_error !== true,
      });
      continue;
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
      if (m.notice === "hidden") continue;
      // Mid-turn steering persists as "The user interrupted: X" — the
      // live stream echoes the bare text, so strip the annotation.
      const steer = text.match(/^The user interrupted: ([\s\S]*)$/);
      if (steer) text = steer[1].trim();
    }
    // Assistant messages that only carry `tool_calls` have no visible text —
    // the matching `tool` result already renders the capsule.
    if (!text.trim()) {
      if (markerLine) v.items.push({ kind: "system", text: markerLine });
      continue;
    }
    if (m.role === "user") {
      v.items.push({ kind: "user", text });
    } else {
      v.items.push({ kind: "assistant", text, streaming: false });
      if (markerLine) v.items.push({ kind: "system", text: markerLine });
    }
  }
  return v;
}
