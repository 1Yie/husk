// Event → view — the live-stream reducer. One `UiEvent` folds into the
// per-session `SessionView`; paired with `viewFromHistory` it is the
// other half of "same stream, two sources".

import type { AgentEventEnvelope } from "@/types";
import type {
  ChangeKind,
  ChangePatch,
  FileChange,
  SessionView,
  StreamItem,
} from "@/features/chat/hooks/stream-view";

// ---------------------------------------------------------------- changes

/** Write tools emit `ui_type: "diff"` with a unified diff in `content`. */
const WRITE_TOOLS = new Set(["fuzzy_patch", "apply_patch", "write_file", "fs_patch"]);

/** `fuzzy_patch` content: `patched {path} (line N)\n\n{diff}`. */
const FUZZY_HEADER = /^patched\s+(.+?)\s+\(line \d+\)/;
/** `apply_patch` emits `added|updated|deleted|moved {path}` per file. */
const PATCH_SECTION = /^(added|updated|deleted|moved)\s+(.+?)\s*$/;

function looksLikeDiff(text: string): boolean {
  return text.includes("@@ -") || (text.includes("--- ") && text.includes("+++ "));
}

function countDiff(diff: string): { adds: number; dels: number } {
  let adds = 0;
  let dels = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) adds++;
    else if (line.startsWith("-")) dels++;
  }
  return { adds, dels };
}

function sectionsFromFuzzy(content: string): { path: string; kind: ChangeKind; diff: string }[] {
  const m = content.match(FUZZY_HEADER);
  if (!m) return [];
  const diffStart = content.indexOf("\n\n");
  const diff = diffStart >= 0 ? content.slice(diffStart + 2) : "";
  if (!looksLikeDiff(diff)) return [];
  return [{ path: m[1].trim(), kind: "update", diff }];
}

/** Split an `apply_patch` result into one entry per touched file. */
export function sectionsFromPatch(content: string): { path: string; kind: ChangeKind; diff: string }[] {
  const out: { path: string; kind: ChangeKind; diff: string }[] = [];
  const lines = content.split("\n");
  let cur: { path: string; kind: ChangeKind; diff: string[] } | null = null;
  const flush = () => {
    if (!cur) return;
    const diff = cur.diff.join("\n").replace(/\n+$/, "");
    if (cur.kind === "delete" || looksLikeDiff(diff)) {
      out.push({ path: cur.path, kind: cur.kind, diff });
    }
    cur = null;
  };
  for (const line of lines) {
    const m = line.match(PATCH_SECTION);
    if (m) {
      flush();
      const kind: ChangeKind = m[1] === "added" ? "add" : m[1] === "deleted" ? "delete" : "update";
      // `moved a → b` has no diff body — flush() drops it; keep the
      // destination as the panel path.
      const path = m[1] === "moved" ? (m[2].split("→").pop() ?? m[2]).trim() : m[2].trim();
      cur = { path, kind, diff: [] };
      continue;
    }
    cur?.diff.push(line);
  }
  flush();
  return out;
}

/** Fold a write tool's diff into `view.changes`, one entry per file.
 * `pending` marks an approval-preview diff (staged, not yet committed). */
export function recordFileChanges(
  changes: FileChange[],
  tool: string,
  content: string,
  fuzzy: boolean,
  pending = false,
) {
  if (!WRITE_TOOLS.has(tool)) return;
  const sections =
    tool === "apply_patch" ? sectionsFromPatch(content) : sectionsFromFuzzy(content);
  for (const s of sections) {
    let fc = changes.find((c) => c.path === s.path);
    if (!fc) {
      fc = { path: s.path, kind: s.kind, patches: [], adds: 0, dels: 0, fuzzy: false };
      changes.push(fc);
    }
    // Latest op wins the badge; adds/dels accumulate.
    fc.kind = s.kind;
    fc.fuzzy = fc.fuzzy || fuzzy;
    // Approval → the same staged diff lands twice (ApprovalRequested,
    // then ToolCallFinished). Dedupe: the finished call flips `pending`
    // off instead of stacking a second patch.
    if (fc.patches.some((p) => p.diff === s.diff)) {
      if (!pending) fc.pending = false;
      continue;
    }
    const { adds, dels } = countDiff(s.diff);
    fc.adds += adds;
    fc.dels += dels;
    const patch: ChangePatch = { tool, diff: s.diff, n: fc.patches.length + 1 };
    fc.patches.push(patch);
    fc.pending = pending;
  }
}

/** A denied approval never commits — drop the pending preview patches
 * `ApprovalRequested` added under this tool's name. Approvals serialize,
 * so every pending entry of that tool belongs to the refused call. */
function dropPendingChanges(changes: FileChange[], tool: string) {
  for (let i = changes.length - 1; i >= 0; i--) {
    const fc = changes[i];
    if (!fc.pending) continue;
    fc.patches = fc.patches.filter((p) => p.tool !== tool);
    if (fc.patches.length === 0) changes.splice(i, 1);
    else fc.pending = false;
  }
}

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

/** Append a delegated child's streamed text to its own capsule (matched by the
 *  label the kernel tagged it with). Returns false when no such row is open —
 *  the text is then dropped rather than misattributed to the turn's draft. */
function appendChildText(
  items: SessionView["items"],
  parent: string,
  text: string,
  reasoning: boolean,
): boolean {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "tool" && it.content === undefined && it.name === parent) {
      items[i] = reasoning
        ? { ...it, liveReasoning: (it.liveReasoning ?? "") + text }
        : { ...it, live: (it.live ?? "") + text };
      return true;
    }
  }
  return false;
}

export function applyEvent(
  v: SessionView,
  ev: AgentEventEnvelope["event"]
): SessionView {
  const items = [...v.items];
  const last = () => items[items.length - 1];

  if (ev === "TurnRetry") {
    // The session rewound to before the last turn's user prompt — drop
    // that turn's items (user bubble included; the fresh `UserPrompt`
    // echo re-adds it). Any pending question/approval dies with the turn.
    for (let i = items.length - 1; i >= 0; i--) {
      if (items[i].kind === "user") {
        items.splice(i);
        break;
      }
    }
    return { ...v, items, pendingQuestion: undefined, turnStartedAt: undefined };
  }
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
    // A finished/failed/idle turn resolves any parked question too.
    const settled = s === "Idle" || s === "Finished" || (typeof s === "object" && "Failed" in s);
    // A pass that ends in anything but `Compacting` (failure, cancel) leaves
    // a still-pending card behind — drop it here; the failure notice draws
    // its own line. Success settles the card in place from `Compacted`.
    const compacting = s === "Compacting";
    const next = compacting
      ? items
      : items.filter((it) => !(it.kind === "compaction" && it.pending));
    return {
      ...v,
      items: next,
      state: s,
      streaming,
      ...(settled
        ? { pendingQuestion: undefined, turnStartedAt: undefined, turnOpen: false }
        : {}),
    };
  }
  if ("CompactionStarted" in ev) {
    // The running card appears the moment the pass starts — the
    // summarization sample can run for a long time on a big history, and
    // without this the user saw nothing until the finished card landed.
    // `manual` decides where it lands: a user-run `/compact` is its own
    // block between turns, an automatic pass stays inside the current turn.
    if (items.some((it) => it.kind === "compaction" && it.pending)) return v;
    return {
      ...v,
      items: [
        ...items,
        {
          kind: "compaction" as const,
          before: 0,
          after: 0,
          removed: 0,
          note: "",
          manual: ev.CompactionStarted.manual,
          pending: true,
          ts: Date.now(),
        },
      ],
    };
  }
  if ("UserPrompt" in ev) {
    items.push({ kind: "user", text: ev.UserPrompt, ts: Date.now() });
    return {
      ...v,
      items,
      turnOpen: true,
      pendingQuestion: undefined,
      turnStartedAt: Date.now(),
      toksPerSec: 0,
      rate: { chars: 0, activeMs: 0, lastAt: null },
    };
  }
  // A delegated child's deltas carry its capsule label: they belong to that
  // child's row, never to the turn's draft (three children would share one
  // buffer), and they must not move the turn's tokens/sec meter.
  if ("TextDelta" in ev && ev.TextDelta.parent) {
    const { parent, text } = ev.TextDelta;
    appendChildText(items, parent, text, false);
    return { ...v, items };
  }
  if ("ReasoningDelta" in ev && ev.ReasoningDelta.parent) {
    const { parent, text } = ev.ReasoningDelta;
    appendChildText(items, parent, text, true);
    return { ...v, items };
  }
  if ("TextDelta" in ev) {
    closeOpenThinking(items);
    const rate = bumpRate(v, ev.TextDelta.text);
    const l = last();
    if (l?.kind === "assistant")
      items[items.length - 1] = { ...l, text: l.text + ev.TextDelta.text, streaming: true };
    else items.push({ kind: "assistant", text: ev.TextDelta.text, streaming: true });
    return { ...v, items, rate, toksPerSec: rateEstimate(rate) };
  }
  if ("ReasoningDelta" in ev) {
    const rate = bumpRate(v, ev.ReasoningDelta.text);
    const l = last();
    // The current reply's thinking block is either the tail item, or sits
    // immediately before the streaming assistant bubble — some backends
    // (deepseek's Responses shim) emit reasoning deltas AFTER the answer
    // text, which used to spawn a dangling "思考过程" pill at the end of
    // the reply. Append to that block, or insert a fresh one ahead of the
    // streaming assistant; a done block reopening is harmless (the next
    // TextDelta/AssistantMessage closes it again via closeOpenThinking).
    let ti = l?.kind === "thinking" && !l.done ? items.length - 1 : -1;
    if (ti < 0 && l?.kind === "assistant" && l.streaming) {
      const p = items.length - 2;
      if (items[p]?.kind === "thinking") ti = p;
    }
    if (ti >= 0) {
      const it = items[ti];
      if (it.kind === "thinking")
        items[ti] = {
          ...it,
          text: it.text + ev.ReasoningDelta.text,
          done: false,
          // Keep the ORIGINAL start stamp on a reopen — the clock must not
          // restart when a late reasoning delta flips the block live again.
          startedAt: it.startedAt ?? Date.now(),
        };
    } else if (l?.kind === "assistant" && l.streaming) {
      items.splice(items.length - 1, 0, {
        kind: "thinking",
        text: ev.ReasoningDelta.text,
        done: false,
        startedAt: Date.now(),
      });
    } else {
      items.push({
        kind: "thinking",
        text: ev.ReasoningDelta.text,
        done: false,
        startedAt: Date.now(),
      });
    }
    return { ...v, items, rate, toksPerSec: rateEstimate(rate) };
  }
  if ("ToolCallStarted" in ev) {
    closeOpenThinking(items);
    const name = ev.ToolCallStarted.name;
    items.push({
      kind: "tool",
      name,
      args: ev.ToolCallStarted.args_preview,
      startedAt: Date.now(),
      // Rust Option<String> serializes `null`, not `undefined` —
      // normalize at the boundary so the Finished matcher's
      // `=== undefined` comparison actually works.
      parent: ev.ToolCallStarted.parent ?? undefined,
    });
    return { ...v, items };
  }
  if ("ToolCallFinished" in ev) {
    closeOpenThinking(items);
    const t = ev.ToolCallFinished;
    // Fold a committed write's diff into the changes panel.
    const changes = [...v.changes];
    if (t.ok && t.content) recordFileChanges(changes, t.name, t.content, false);
    else if (!t.ok) dropPendingChanges(changes, t.name);
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "approval" && !it.resolved && it.toolName === t.name) {
        items[i] = { ...it, resolved: true, approved: t.ok };
        break;
      }
    }
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      // Same-parent match: a batch child's Finished lands on the child
      // row, not the batch capsule itself (and vice versa).
      if (
        it.kind === "tool" &&
        it.content === undefined &&
        (it.parent ?? undefined) === (t.parent ?? undefined)
      ) {
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
    return { ...v, items, changes };
  }
  if ("ApprovalRequested" in ev) {
    closeOpenThinking(items);
    const a = ev.ApprovalRequested;
    // Mirror the staged write's diff into the panel as a preview
    // (pending). Approve re-records and dedupes; deny drops it.
    const changes = [...v.changes];
    if (a.diff) recordFileChanges(changes, a.tool_name, a.diff, a.fuzzy, true);
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
    return { ...v, items, changes };
  }
  if ("QuestionAsked" in ev) {
    const q = ev.QuestionAsked;
    return {
      ...v,
      pendingQuestion: {
        requestId: q.request_id,
        question: q.question,
        options: q.options ?? [],
      },
    };
  }
  if ("QuestionAnswered" in ev) {
    // The kernel resolved the parked card — drop it from the buffer itself.
    // (The composer's `answeredId` still hides it instantly; this is what
    // stops the card resurrecting after a session switch.) Only clear when
    // the id matches: a stale answer must not clear a newer question.
    if (v.pendingQuestion?.requestId === ev.QuestionAnswered.request_id) {
      return { ...v, pendingQuestion: undefined };
    }
    return v;
  }
  if ("AssistantMessage" in ev) {
    closeOpenThinking(items);
    // Replace the streamed draft IN PLACE — not just when it's `last()`:
    // a ReasoningDelta can land between the streamed text and this final
    // message (e.g. a second sampling pass after auto-compaction), which
    // used to leave the draft plus a duplicate final block.
    let replaced = false;
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "assistant" && it.streaming) {
        items[i] = { ...it, text: ev.AssistantMessage, streaming: false, ts: Date.now() };
        replaced = true;
        break;
      }
      // Stop at the previous turn's boundary — an interrupted turn can
      // leave a stale `streaming` draft; this message belongs to the
      // current turn and must not retro-fill that one.
      if (it.kind === "assistant" || it.kind === "user") break;
    }
    if (!replaced) {
      const l = last();
      if (!(l?.kind === "assistant" && l.text === ev.AssistantMessage)) {
        items.push({ kind: "assistant", text: ev.AssistantMessage, streaming: false, ts: Date.now() });
      }
    }
    return { ...v, items };
  }
  if ("SystemMessage" in ev) {
    closeOpenThinking(items);
    items.push({ kind: "system", text: ev.SystemMessage, standalone: !v.turnOpen });
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
        cachedTokens: ev.Usage.cached_tokens,
      },
    };
  }
  if ("Compacted" in ev) {
    // A compaction pass landed. Draw the card where it happened and re-base
    // the context meter NOW — the next `Usage` (end of this turn) would
    // otherwise be the first truthful reading, leaving the header showing
    // the pre-compaction fill for the rest of the turn.
    closeOpenThinking(items);
    const c = ev.Compacted;
    const card: StreamItem = {
      kind: "compaction",
      before: c.before_tokens,
      after: c.after_tokens,
      removed: c.removed_messages,
      note: c.note,
      manual: c.manual,
      ts: Date.now(),
    };
    // Settle the in-flight placeholder in place (same position the pass
    // started at); push a fresh card when there was none — e.g. a session
    // switched to mid-pass, or a view rebuilt while the phase was running.
    let settled = false;
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "compaction" && it.pending) {
        items[i] = card;
        settled = true;
        break;
      }
    }
    if (!settled) items.push(card);
    return {
      ...v,
      items,
      usage: {
        ...v.usage,
        prompt: c.after_tokens,
        // The pre-compaction cache reading describes a request body that no
        // longer exists — leaving it would make the uncached chip under-read
        // until the next `Usage` lands.
        cachedTokens: 0,
        ...(c.context_window > 0 ? { contextWindow: c.context_window } : {}),
      },
    };
  }
  if ("Error" in ev) {
    closeOpenThinking(items);
    items.push({ kind: "system", text: `⚠ ${ev.Error}`, standalone: !v.turnOpen });
    return { ...v, items, streaming: false };
  }
  if ("QueuedPrompts" in ev) {
    // The kernel's parked list is the single source of truth — every
    // SetQueued write and each drain pop echoes the new list here.
    return { ...v, queuedPrompts: ev.QueuedPrompts.items };
  }
  return v;
}
