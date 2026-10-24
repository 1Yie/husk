// History → view — rebuild a `SessionView` from the persisted
// `ChatMessage` snapshot when a session is reopened. Must render the
// same stream the live `applyEvent` path showed: failed tool capsules
// stay red (`is_error`), turn-end salvage markers become system lines,
// steer annotations strip to the bare text.

import * as agent from "@/lib/agent-ipc/index";
import {
  emptyView,
  type ChildSegment,
  type SessionView,
  type StreamItem,
} from "@/features/chat/hooks/stream-view";
import { recordArtifact, recordFileChanges } from "@/features/chat/hooks/apply-event";
import type { ChangeFeedRow } from "@/lib/agent-ipc/sessions";
import type { ChatMessage } from "@/types";

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
/** Recover a call's tool name when the persisted section header says
 * `(unnamed)` — older sessions have the malformed item in `calls[]` (e.g.
 * `{"function": {"name": …}}`), so the real name is still recoverable. */
function batchToolName(call: Record<string, unknown>): string | undefined {
  for (const key of ["tool", "name", "tool_name", "call", "command", "action", "function"]) {
    const v = call[key];
    if (typeof v === "string") return v;
    if (v && typeof v === "object") {
      const n = (v as { name?: unknown }).name;
      if (typeof n === "string") return n;
    }
  }
  // No name key at all — infer from the args signature the way the kernel's
  // normalize_shape does (old persisted rows carry the raw item).
  const args =
    call.args && typeof call.args === "object"
      ? (call.args as Record<string, unknown>)
      : call;
  if ("url" in args) return "web_fetch";
  if ("pattern" in args || "regex" in args || "max_hits" in args) return "smart_grep";
  if ("command" in args) return "bash";
  return undefined;
}

function batchChildren(rawArgs: string, content: string) {
  let calls: Record<string, unknown>[] = [];
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
    // `(unnamed)` = the kernel couldn't recover a label — try the raw call
    // shape once more before showing the placeholder to the user.
    const recovered =
      tool === "(unnamed)" && calls[idx] ? batchToolName(calls[idx]) : undefined;
    items.push({
      kind: "tool" as const,
      name: recovered ?? tool,
      args:
        callArgs === undefined
          ? // OpenAI wire shape carries `function.arguments`, not `args`.
            (() => {
              const f = calls[idx]?.function;
              const fa =
                f && typeof f === "object"
                  ? (f as { arguments?: unknown }).arguments
                  : undefined;
              return fa === undefined ? "" : extractArgsPreview(JSON.stringify(fa));
            })()
          : extractArgsPreview(JSON.stringify(callArgs)),
      content: body || undefined,
      ok,
      // The batch's own status word — "rejected" means it never ran; the
      // chips render 已拒绝 instead of the misleading 已中断.
      uiType: status === "rejected" ? "rejected" : undefined,
      parent: "batch_execute",
      // Every item of one batch rides the capsule's `hi`, so its place in
      // `calls[]` is the other half of its row id — without it the siblings
      // shared one id and opening one expanded them all.
      childIndex: idx,
    });
  }
  return items;
}

/** `answer_child` args → the child it answers, and the answer text. */
function answerChildArgs(rawArgs: string): { childId?: number; answer: string } {
  try {
    const parsed = JSON.parse(rawArgs);
    return {
      childId: typeof parsed?.child_id === "number" ? parsed.child_id : undefined,
      answer: typeof parsed?.answer === "string" ? parsed.answer : "",
    };
  } catch {
    return { answer: "" };
  }
}

/** The child an `answer_child` row resumed; exported for the 子代理 sidebar,
 *  which clears that child's waiting state out of the session-level feed. */
export function answerChildId(rawArgs: string): number | undefined {
  return answerChildArgs(rawArgs).childId;
}

function answerChildText(rawArgs: string): string {
  return answerChildArgs(rawArgs).answer;
}

/** `delegate` args → `N tasks` chip text for the parent capsule. */
function delegateCallCount(rawArgs: string): string {
  try {
    const parsed = JSON.parse(rawArgs);
    if (Array.isArray(parsed?.tasks)) return `${parsed.tasks.length} tasks`;
    return parsed?.task ? "1 task" : "";
  } catch {
    return "";
  }
}

/** The label the kernel gives each child card — keep the two in step:
 * `subagent` for one unnamed child, `<agent> #n` when a manifest was named. */
function delegateChildLabel(agent: string | undefined, index: number, total: number): string {
  if (agent) return `${agent} #${index + 1}`;
  return total === 1 ? "subagent" : `subagent #${index + 1}`;
}

/** The parent's early return while a child is parked: `delegate` answers with
 *  `[subagent <label> (child_id N) is waiting for you]`, so a reloaded session
 *  can still draw the waiting card against the right child. */
const WAITING_REPORT = /^\[subagent (.+?) \(child_id (\d+)\) is waiting for you\]/;

/** A detached child still out when its parallel `delegate` returned: its section
 *  reads `[subagent <label> (child_id N) is still running …]`, and its report
 *  arrives later as a host notice. The id is the only way to name its card. */
const RUNNING_REPORT = /^\[subagent (.+?) \(child_id (\d+)\) is still running/;

/** A detached child's settlement, delivered later as a host notice: the header
 *  says how it ended, and the text after it is the report, error, or question. */
const BACKGROUND_NOTICE =
  /^\[subagent (.+?) \(child_id (\d+)\) (finished in the background|failed in the background|is waiting for you)\]/;

/** A finished child's report opens with `[subagent report — child_id N · K tool
 *  calls, Xs]` — the only place the persisted log still names the child, and
 *  the id its card needs to replay the transcript afterwards. */
const CHILD_REPORT = /\[subagent report — child_id (\d+) ·/;

/** A cancelled child's result: the report it never produced is replaced by
 *  `[subagent cancelled — child_id N]`. The id still names the child, so a
 *  reloaded card keeps its transcript; only the report is gone. */
const CHILD_CANCELLED = /\[subagent cancelled — child_id (\d+)\]/;

/** The body a cancelled child's card carries. The chips read this exact word as
 *  已中断 — the live card's own cancel wording — where any other body would read
 *  失败. */
const CANCELLED_BODY = "已中断";

/** Rebuild a delegation's nested child items from the persisted args
 * (`task`/`tasks`) and result text (`── [i] ok · 3s ──` sections for the
 * parallel form; the bare report for a single child) — live-only events, so
 * without this a reopened session would show the parent capsule alone.
 * Exported for the 子代理 sidebar, which folds the same cards out of the
 * session-level tool feed. */
export function delegateChildren(rawArgs: string, content: string, isError: boolean) {
  let tasks: string[] = [];
  let agent: string | undefined;
  try {
    const parsed = JSON.parse(rawArgs);
    if (Array.isArray(parsed?.tasks)) tasks = parsed.tasks.map(String);
    else if (typeof parsed?.task === "string") tasks = [parsed.task];
    if (typeof parsed?.agent === "string") agent = parsed.agent;
  } catch {
    /* malformed args — the report below still carries the bodies */
  }
  const total = tasks.length;
  const items: {
    kind: "tool";
    name: string;
    args: string;
    content?: string;
    ok: boolean;
    parent: string;
    childId?: number;
    /** The child's place in the batch — every child shares the parent row's
     *  `hi`, so this is what makes its card id unique (and, when the report
     *  lost its `child_id`, its only identity). */
    childIndex: number;
    waiting?: string;
  }[] = [];
  if (total <= 1) {
    // Single child: the report is the whole body, one card.
    const body = content.trim();
    if (!body) return items;
    // A parked child's early return: the card is waiting, not finished, and
    // the id in the text is the only thing that names it after a reload.
    const waiting = body.match(WAITING_REPORT);
    const finished = body.match(CHILD_REPORT);
    // A cancelled child never wrote a report — the kernel's marker is all the
    // log kept, and it is the card's only route back to the child.
    const cancelled = body.match(CHILD_CANCELLED);
    items.push({
      kind: "tool" as const,
      name: waiting ? waiting[1] : delegateChildLabel(agent, 0, 1),
      args: tasks[0] ? extractArgsPreview(JSON.stringify(tasks[0])) : "",
      content: cancelled ? CANCELLED_BODY : body,
      // The persisted `is_error` is what the live card's `ok` showed — a
      // cancelled or failed child must not reload green.
      ok: !isError && !cancelled,
      parent: "delegate",
      childIndex: 0,
      childId: waiting
        ? Number(waiting[2])
        : finished
          ? Number(finished[1])
          : cancelled
            ? Number(cancelled[1])
            : undefined,
      waiting: waiting ? body.slice(waiting[0].length).trim() : undefined,
    });
    return items;
  }
  // Parallel form: `── [i] ok · 3s ──` sections, input order. A cancelled child
  // says so in its status word and carries the marker as its body.
  const parts = content.split(/\n── \[(\d+)\] /);
  for (let i = 1; i + 1 < parts.length; i += 2) {
    const idx = Number(parts[i]);
    const rest = parts[i + 1];
    const nl = rest.indexOf("──");
    if (nl < 0) continue;
    const status = rest.slice(0, nl).trim();
    const body = rest.slice(nl + 2).replace(/^\n/, "").trim();
    // A parked child's section: the card is waiting, and its question is the body
    // — the same reading the single-child form gives its early return.
    const waiting = body.match(WAITING_REPORT);
    // A detached child still out has no report yet. Its card stays unsettled —
    // `content` unset — so it reads as running, and its report later arrives as
    // a host notice rather than a row.
    const running = body.match(RUNNING_REPORT);
    const finished = body.match(CHILD_REPORT);
    const cancelled = body.match(CHILD_CANCELLED);
    // Either signal marks it: the status word alone is enough when a log lost
    // the marker, the marker alone when the head still says `ok`.
    const interrupted = cancelled != null || status.startsWith("cancelled");
    items.push({
      kind: "tool" as const,
      name: waiting ? waiting[1] : delegateChildLabel(agent, idx, total),
      args: tasks[idx] ? extractArgsPreview(JSON.stringify(tasks[idx])) : "",
      content: running ? undefined : interrupted ? CANCELLED_BODY : body || undefined,
      ok: !interrupted && status.startsWith("ok"),
      parent: "delegate",
      childIndex: idx,
      childId: waiting
        ? Number(waiting[2])
        : running
          ? Number(running[2])
          : finished
            ? Number(finished[1])
            : cancelled
              ? Number(cancelled[1])
              : undefined,
      waiting: waiting ? body.slice(waiting[0].length).trim() : undefined,
    });
  }
  return items;
}

/** The kernel's prefix on the row a child reads a parent answer as. */
const PARENT_ANSWER_PREFIX = "[answer from your parent]";

/** A `delegate`/`answer_child` call the log never answered: the app was quit
 *  (or the actor killed) with the child still out, so no tool row will ever
 *  follow it. Nothing else in the history can draw that child's card, so it is
 *  drawn here as interrupted — cheap and honest beats no card at all. A
 *  `delegate` child's id is minted at runtime and never persisted, so it stays
 *  unknown; `answer_child` names the child it resumed in its own args. */
function danglingDelegateCards(
  tool: string,
  rawArgs: string,
): {
  kind: "tool";
  name: string;
  args: string;
  content?: string;
  ok: boolean;
  parent: string;
  childId?: number;
  childIndex: number;
}[] {
  if (tool === "delegate") {
    let tasks: string[] = [];
    let agent: string | undefined;
    try {
      const parsed = JSON.parse(rawArgs);
      if (Array.isArray(parsed?.tasks)) tasks = parsed.tasks.map(String);
      else if (typeof parsed?.task === "string") tasks = [parsed.task];
      if (typeof parsed.agent === "string") agent = parsed.agent;
    } catch {
      /* malformed args — the card still says the child never came back */
    }
    if (tasks.length === 0) tasks = [""];
    return tasks.map((task, i) => ({
      kind: "tool" as const,
      name: delegateChildLabel(agent, i, tasks.length),
      args: task ? extractArgsPreview(JSON.stringify(task)) : "",
      content: CANCELLED_BODY,
      ok: false,
      parent: "delegate",
      childId: undefined,
      // No report ever named this child — its place in the batch is all the
      // log can say, and it still keeps its card's id off its siblings'.
      childIndex: i,
    }));
  }
  return [
    {
      kind: "tool" as const,
      name: "subagent",
      args: "",
      content: CANCELLED_BODY,
      ok: false,
      parent: "delegate",
      childId: answerChildId(rawArgs),
      childIndex: 0,
    },
  ];
}

/** A finished child's own history → the body its card shows, so reopening a
 *  session restores what the live `applyEvent` path had streamed: assistant
 *  text runs, the child's own tool calls as nested rows, and the muted steer /
 *  parent-answer notes. Its system row and the task prompt are context, not
 *  body — the live card never drew them either. */
export function childTranscriptCard(
  history: ChatMessage[],
  label: string,
  childId?: number,
): { segments: ChildSegment[]; rows: StreamItem[] } {
  const idToTool = new Map<string, ToolInfo>();
  for (const m of history) indexToolCalls(idToTool, m);
  const segments: ChildSegment[] = [];
  const rows: StreamItem[] = [];
  // Consecutive text folds into the run already open — the live stream
  // appended each delta to it, so replay must not fan one run into several.
  const pushText = (text: string, muted = false) => {
    const last = segments[segments.length - 1];
    if (last?.kind === "text" && !!last.muted === muted) {
      segments[segments.length - 1] = { ...last, text: `${last.text}\n\n${text}` };
      return;
    }
    segments.push(muted ? { kind: "text", text, muted } : { kind: "text", text });
  };
  for (const m of history) {
    if (m.role === "system") continue;
    if (m.role === "user") {
      if (m.notice === "hidden") continue;
      const steer = steerTextOf(m);
      if (steer !== null) {
        pushText(`> 用户插话：${steer}`, true);
        continue;
      }
      const text = (m.content ?? "").trim();
      if (text.startsWith(PARENT_ANSWER_PREFIX)) {
        pushText(`> 主代理回复：${text.slice(PARENT_ANSWER_PREFIX.length).trim()}`, true);
      }
      // Anything else is the task it was handed — already on the card's chip.
      continue;
    }
    if (m.role === "tool") {
      const info = (m.tool_call_id && idToTool.get(m.tool_call_id)) || { name: "tool", args: "", raw: "" };
      // One row per call, and the segment indexes ITS OWN rows — the card's
      // `children` are appended in this same order.
      const index = rows.length;
      rows.push({
        kind: "tool",
        name: info.name,
        args: info.args,
        content: m.content ?? "",
        ok: m.is_error !== true,
        parent: label,
        childId,
        ts: m.ts ?? undefined,
        startedAt: m.ts != null && m.duration_ms != null ? m.ts - m.duration_ms : undefined,
      });
      segments.push({ kind: "tool", index });
      continue;
    }
    // The round's reasoning, drawn before the text and calls it produced —
    // the same order the live stream appended its runs in.
    const reasoning = (m.reasoning ?? "").trim();
    if (reasoning) segments.push({ kind: "thinking", text: reasoning });
    const text = stripCallEcho(m.content ?? "").trim();
    if (text) pushText(text);
  }
  return { segments, rows };
}

/** Every `tool_call_id` this history answered with a tool row. A call without
 *  one was never dispatched to completion: the process died mid-turn. */
function answeredCalls(history: ChatMessage[]): Set<string> {
  const out = new Set<string>();
  for (const m of history) {
    if (m.role === "tool" && m.tool_call_id) out.add(m.tool_call_id);
  }
  return out;
}

/** How a rebuilt page should read rows it cannot complete. `dangling` says the
 *  page is the session's whole state and nothing is running, so a
 *  `delegate`/`answer_child` call with no result is a child that died with the
 *  process — an older page must leave that alone (its results are in the newer
 *  pages already on screen), and a running session leaves it to the live stream. */
export interface HistoryFoldOptions {
  dangling?: boolean;
}

/** Rebuild a `SessionView`'s item list from persisted `ChatMessage`
 * history — used when switching to a session whose in-memory view was
 * never built (first open after launch) or dropped. Tool result messages
 * map to finished tool capsules; a persisted reasoning trace replays as the
 * same 思考过程 block the live stream drew (its assistant row carries it).
 * `usage` is the persisted last-turn meter —
 * seeds the header stats so a reopened session doesn't read zeros until
 * its next `Usage` event. */
export function viewFromHistory(
  history: ChatMessage[],
  usage?: agent.SessionUsage | null,
  baseIndex = 0,
  opts?: HistoryFoldOptions,
): SessionView {
  const v = emptyView();
  if (usage) {
    v.usage = {
      prompt: usage.prompt,
      completion: usage.completion,
      contextWindow: usage.context_window,
      cachedTokens: usage.cached ?? 0,
      cacheWriteTokens: usage.cache_write ?? 0,
    };
    if (usage.breakdown) v.contextBreakdown = usage.breakdown;
  }
  // call_id → tool name and args: resolve from the preceding assistant `tool_calls`.
  const idToTool = new Map<string, { name: string; args: string; raw: string }>();
  for (const m of history) indexToolCalls(idToTool, m);
  const answered = answeredCalls(history);
  history.forEach((m, i) => foldMessage(v, idToTool, m, baseIndex + i, answered, opts?.dangling === true));
  if (opts?.dangling === true) closeOrphanedChildCards(v);
  return v;
}

/** Chunked variant — folds ~24 messages per macrotask so a session
 * rebuild doesn't monopolize the main thread: sidebar/settings input
 * keeps dispatching between slices. Same output as `viewFromHistory`. */
export async function viewFromHistoryChunked(
  history: ChatMessage[],
  usage?: agent.SessionUsage | null,
  baseIndex = 0,
  opts?: HistoryFoldOptions,
): Promise<SessionView> {
  const v = emptyView();
  if (usage) {
    v.usage = {
      prompt: usage.prompt,
      completion: usage.completion,
      contextWindow: usage.context_window,
      cachedTokens: usage.cached ?? 0,
      cacheWriteTokens: usage.cache_write ?? 0,
    };
    if (usage.breakdown) v.contextBreakdown = usage.breakdown;
  }
  const idToTool = new Map<string, { name: string; args: string; raw: string }>();
  for (const m of history) indexToolCalls(idToTool, m);
  const answered = answeredCalls(history);
  for (let i = 0; i < history.length; i++) {
    foldMessage(v, idToTool, history[i], baseIndex + i, answered, opts?.dangling === true);
    if (i % 24 === 23) await new Promise((r) => setTimeout(r, 0));
  }
  if (opts?.dangling === true) closeOrphanedChildCards(v);
  return v;
}

/** Rebuild the session-level summary — the changes panel and the artifacts
 * dock — from the WHOLE persisted log.
 *
 * `viewFromHistory` only sees the mounted page (the newest messages), so a
 * session whose writes are older than that page used to reopen with fewer
 * files than it wrote: the live view had accumulated every `ToolCallFinished`
 * of the process, the rebuilt one only what fit in the page. Same parser and
 * same tool sets — only the source of rows is wider. */
export function applyChangesFeed(v: SessionView, rows: ChangeFeedRow[]) {
  v.changes = [];
  v.artifacts = [];
  for (const row of rows) {
    recordFileChanges(v.changes, row.tool, row.content, false);
    // Artifact writers are located by their call args, exactly as the live
    // event and the page replay do it.
    recordArtifact(v.artifacts, row.tool, extractArgsPreview(row.args));
  }
}

type ToolInfo = { name: string; args: string; raw: string };

/** Persisted `NoticeKind::Compacted` row → the card fields. The content is
 * a JSON payload written by `ChatMessage::compaction`; a malformed/legacy
 * row degrades to nothing rather than throwing out the whole replay. */
function parseCompactionCard(content: string):
  | {
      kind: "compaction";
      before: number;
      after: number;
      removed: number;
      manual: boolean;
      note: string;
    }
  | null {
  try {
    const p = JSON.parse(content);
    return {
      kind: "compaction",
      before: Number(p.before_tokens) || 0,
      after: Number(p.after_tokens) || 0,
      removed: Number(p.removed_messages) || 0,
      manual: p.manual === true,
      note: typeof p.note === "string" ? p.note : "",
    };
  } catch {
    return null;
  }
}

/** call_id → tool name/args index — one pass over `tool_calls` on the
 * preceding assistant messages. */
function indexToolCalls(idToTool: Map<string, ToolInfo>, m: ChatMessage) {
  for (const c of m.tool_calls ?? []) {
    const raw = c.function.arguments ?? "";
    idToTool.set(c.id, {
      name: c.function.name,
      // batch_execute's / delegate's own preview is the call count — the
      // real per-item args live inside `calls[]` / `tasks[]`.
      args:
        c.function.name === "batch_execute"
          ? batchCallCount(raw)
          : c.function.name === "delegate"
            ? delegateCallCount(raw)
            : extractArgsPreview(raw),
      raw,
    });
  }
}

/** Once nothing is running, a delegate card still reading running is a detached
 *  child whose process died — it can no longer finish, so it reads as interrupted. */
function closeOrphanedChildCards(v: SessionView) {
  for (let i = 0; i < v.items.length; i++) {
    const card = v.items[i];
    if (card.kind !== "tool" || card.parent !== "delegate") continue;
    if (card.content !== undefined || card.waiting != null) continue;
    v.items[i] = { ...card, content: CANCELLED_BODY, ok: false };
  }
}

/** A host notice → the delegate card it settles. The newest card with that id is
 *  the live one, as with `answer_child`; a notice names no card of its own. */
function settleBackgroundChild(v: SessionView, content: string) {
  const hit = content.match(BACKGROUND_NOTICE);
  if (!hit) return;
  const childId = Number(hit[2]);
  for (let i = v.items.length - 1; i >= 0; i--) {
    const card = v.items[i];
    if (card.kind !== "tool" || card.parent !== "delegate" || card.childId !== childId) continue;
    if (hit[3] === "is waiting for you") {
      const [next] = delegateChildren("", content, false);
      v.items[i] = { ...card, content: next?.content, ok: next?.ok ?? card.ok, waiting: next?.waiting };
    } else {
      v.items[i] = {
        ...card,
        content: content.slice(hit[0].length).trim(),
        ok: hit[3] === "finished in the background",
        waiting: undefined,
      };
    }
    return;
  }
}

/** Fold one persisted message into the view's item list — shared by the
 * sync and time-sliced builders. */
/** Text the model reads in front of a mid-turn steer. The kernel writes it;
 *  it is matched here only for rows persisted before the `steer` tag existed. */
const STEER_PREFIX = "The user interrupted: ";

/** The steer's own words, or `null` when the row is not a steer. A row the
 *  kernel tagged is judged by its tag alone; an untagged user row is a steer
 *  only if it carries the prefix (history written before the tag). */
export function steerTextOf(m: ChatMessage): string | null {
  if (m.role !== "user") return null;
  const text = m.content ?? "";
  if (m.notice === "steer") {
    return (text.startsWith(STEER_PREFIX) ? text.slice(STEER_PREFIX.length) : text).trim();
  }
  if (m.notice == null && text.startsWith(STEER_PREFIX)) {
    return text.slice(STEER_PREFIX.length).trim();
  }
  return null;
}

/** Whether a persisted row opens a user turn — the store's `counts_as_turn`
 *  on this side: a real prompt, not a hidden row and not a steer. */
export function opensUserTurn(m: ChatMessage): boolean {
  return m.role === "user" && m.notice !== "hidden" && steerTextOf(m) === null;
}

function foldMessage(
  v: SessionView,
  idToTool: Map<string, ToolInfo>,
  m: ChatMessage,
  hi: number,
  /** Calls that DO have a tool row somewhere in this history. */
  answered: ReadonlySet<string>,
  /** Whether an unanswered `delegate`/`answer_child` call is a child that died
   *  with the process — false for an older page whose results are in a newer
   *  one, and false while the session is still running. */
  emitDangling: boolean,
) {
    // `notice` marks the system lines the live stream actually showed —
    // replayed verbatim. Everything else (system prompt, compaction
    // note, hook injections) is internal context and stays hidden.
    if (m.role === "system") {
      if (m.notice === "system")
        // Notices only exist between turns — replay them standalone.
        v.items.push({ kind: "system", text: m.content ?? "", standalone: true, hi });
      else if (m.notice === "error")
        v.items.push({ kind: "system", text: `⚠ ${m.content ?? ""}`, standalone: true, hi });
      else if (m.notice === "compacted") {
        // The compaction card must replay exactly like the live
        // `Compacted` event drew it — numbers, summary, position.
        const card = parseCompactionCard(m.content ?? "");
        if (card) v.items.push({ ...card, ts: m.ts ?? undefined, hi });
      } else if (m.notice === "plan") {
        // The `submit_plan` payload replays as the same plan card the live
        // `plan_submitted` event drew — inside the turn it ended.
        try {
          const plan = JSON.parse(m.content ?? "");
          v.items.push({ kind: "plan", plan, ts: m.ts ?? undefined, hi });
        } catch {
          /* malformed row — skip rather than break the whole replay */
        }
      }
      // A detached child's settlement lands on its card in place — it draws no row
      // of its own, the same as the live stream.
      else if (m.notice === "host") settleBackgroundChild(v, m.content ?? "");
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
        // The persisted span (`duration_ms` = dispatch→result) rewinds the
        // row's start stamp so the step can re-derive its elapsed label —
        // `ts` alone is only the END.
        ts: m.ts ?? undefined,
        startedAt:
          m.ts != null && m.duration_ms != null
            ? m.ts - m.duration_ms
            : undefined,
      });
      // Rebuild the changes panel from the persisted write-tool result.
      if (m.is_error !== true && m.content) {
        recordFileChanges(v.changes, toolInfo.name, m.content, false);
      }
      // Same for the artifacts dock — office/asset writers register the
      // locator path on their first ok call, later writes just bump it.
      if (m.is_error !== true) {
        recordArtifact(v.artifacts, toolInfo.name, toolInfo.args);
      }
      // A batch's inner items were live-only events — they're not in
      // history. Rebuild them from the persisted call list + the
      // `── [i] tool (status) ──` result sections so the nested capsule
      // survives reload.
      if (toolInfo.name === "batch_execute") {
        for (const child of batchChildren(toolInfo.raw, m.content ?? "")) {
          v.items.push({ ...child, hi });
        }
      }
      // Same for a delegation: child cards are live-only events.
      if (toolInfo.name === "delegate") {
        for (const child of delegateChildren(toolInfo.raw, m.content ?? "", m.is_error === true)) {
          v.items.push({ ...child, hi });
        }
      }
      // The parent answering a parked child is its own tool row — replay the
      // reply onto the child's card, so the reloaded session shows it resumed.
      if (toolInfo.name === "answer_child") {
        const childId = answerChildId(toolInfo.raw);
        const answer = answerChildText(toolInfo.raw);
        if (childId != null && answer) {
          for (let i = v.items.length - 1; i >= 0; i--) {
            const card = v.items[i];
            if (card.kind !== "tool" || card.parent !== "delegate" || card.childId !== childId) continue;
            // `answer_child` does not return an acknowledgement — its result IS
            // the resumed child's next outcome: a report, a cancel marker, or
            // another park. Read it as such, or the card keeps the stale parked
            // body and reads as a finished report that was never produced.
            const [next] = delegateChildren("", m.content ?? "", m.is_error === true);
            v.items[i] = {
              ...card,
              ...(next ? { content: next.content, ok: next.ok, waiting: next.waiting } : { waiting: undefined }),
              segments: [
                ...(card.segments ?? []),
                { kind: "text" as const, text: `> 主代理回复：${answer}`, muted: true },
              ],
            };
            break;
          }
        }
      }
      return;
    }
    let text =
      m.role === "assistant" ? stripCallEcho(m.content ?? "") : (m.content ?? "");
    let markerLine: string | null = null;
    let isSteer = false;
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
      // The live stream echoes a steer as its bare words, so strip the prefix
      // the model reads.
      const steerText = steerTextOf(m);
      if (steerText !== null) {
        text = steerText;
        isSteer = true;
      }
    }
    // The round's reasoning trace, if it had one — drawn BEFORE its text and
    // tool capsules, which is the order the live stream produced. Emitted
    // even when the round contributed no visible text: a tool-calling round
    // is usually exactly that shape, and dropping it here is what made
    // reopened sessions lose their thinking blocks.
    const reasoning = (m.reasoning ?? "").trim();
    if (m.role === "assistant" && reasoning) {
      v.items.push({
        kind: "thinking",
        text: reasoning,
        done: true,
        hi,
        // Persisted span (`duration_ms` = first→last reasoning delta) rewinds
        // the start stamp — `ts - startedAt` replays the settled `· 20s`,
        // identical formula to a live-closed block.
        ts: m.ts ?? undefined,
        startedAt:
          m.ts != null && m.duration_ms != null
            ? m.ts - m.duration_ms
            : undefined,
      });
    }
    // A `delegate`/`answer_child` call this history never answered: the child
    // was still out when the process died, and no tool row will ever follow.
    // Its card is drawn here as interrupted — the only thing the log can say.
    if (emitDangling && m.role === "assistant") {
      for (const c of m.tool_calls ?? []) {
        const tool = c.function.name;
        if (tool !== "delegate" && tool !== "answer_child") continue;
        if (answered.has(c.id)) continue;
        for (const card of danglingDelegateCards(tool, c.function.arguments ?? "")) {
          // A resumed child already has the card its own `delegate` row drew —
          // that one closes in place instead of appearing twice.
          const at =
            card.childId == null
              ? -1
              : v.items.findIndex(
                  (it) =>
                    it.kind === "tool" && it.parent === "delegate" && it.childId === card.childId,
                );
          const existing = at >= 0 ? v.items[at] : undefined;
          if (existing?.kind === "tool") {
            v.items[at] = { ...existing, content: CANCELLED_BODY, ok: false, waiting: undefined };
            continue;
          }
          v.items.push({ ...card, hi });
        }
      }
    }
    // Assistant messages that only carry `tool_calls` have no visible text —
    // the matching `tool` result already renders the capsule.
    if (!text.trim()) {
      if (markerLine) v.items.push({ kind: "system", text: markerLine, hi });
      return;
    }
    if (m.role === "user") {
      v.items.push({ kind: "user", text, ts: m.ts ?? undefined, hi, ...(isSteer ? { steer: true } : {}) });
    } else {
      v.items.push({
        kind: "assistant",
        text,
        streaming: false,
        // Arrival stamp for the tools-phase end — `m.ts − duration_ms` is
        // the reasoning start ≈ first model output after the tools; without
        // it the commit `ts` is the fallback (slightly late, old snapshots).
        startedAt:
          m.ts != null && m.duration_ms != null ? m.ts - m.duration_ms : undefined,
        ts: m.ts ?? undefined,
        hi,
      });
      if (markerLine) v.items.push({ kind: "system", text: markerLine, hi });
    }
}
