// Event → view — the live-stream reducer. One `UiEvent` folds into the
// per-session `SessionView`; paired with `viewFromHistory` it is the
// other half of "same stream, two sources".

import type { AgentEventEnvelope } from "@/types";
import type {
  Artifact,
  BgJob,
  ChangeKind,
  ContextBreakdown,
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
 * `pending` marks an approval-preview diff (staged, not yet committed).
 *
 * `changes` is a fresh array the caller owns, but the entries in it are the
 * previous view's: a touched entry is replaced, never edited in place, so a
 * memoized file row re-renders exactly when its own file changed. */
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
    let i = changes.findIndex((c) => c.path === s.path);
    if (i < 0) {
      changes.push({ path: s.path, kind: s.kind, patches: [], adds: 0, dels: 0, fuzzy: false });
      i = changes.length - 1;
    }
    const fc = changes[i];
    // Latest op wins the badge; adds/dels accumulate.
    const base = { ...fc, kind: s.kind, fuzzy: fc.fuzzy || fuzzy };
    // Approval → the same staged diff lands twice (ApprovalRequested,
    // then ToolCallFinished). Dedupe: the finished call flips `pending`
    // off instead of stacking a second patch.
    if (fc.patches.some((p) => p.diff === s.diff)) {
      const nextPending = pending ? fc.pending : false;
      if (base.kind !== fc.kind || base.fuzzy !== fc.fuzzy || nextPending !== fc.pending) {
        changes[i] = { ...base, pending: nextPending };
      }
      continue;
    }
    const { adds, dels } = countDiff(s.diff);
    const patch: ChangePatch = { tool, diff: s.diff, n: fc.patches.length + 1 };
    changes[i] = {
      ...base,
      adds: fc.adds + adds,
      dels: fc.dels + dels,
      patches: [...fc.patches, patch],
      pending,
    };
  }
}

/** A denied approval never commits — drop the pending preview patches
 * `ApprovalRequested` added under this tool's name. Approvals serialize,
 * so every pending entry of that tool belongs to the refused call. */
function dropPendingChanges(changes: FileChange[], tool: string) {
  for (let i = changes.length - 1; i >= 0; i--) {
    const fc = changes[i];
    if (!fc.pending) continue;
    const patches = fc.patches.filter((p) => p.tool !== tool);
    if (patches.length === 0) changes.splice(i, 1);
    else changes[i] = { ...fc, patches, pending: false };
  }
}

/** Close every approval card still waiting for a verdict. The kernel parks in
 * `AwaitingToolConfirmation` for exactly one request, so a settled turn cannot
 * still be waiting: any card left open is one it abandoned (a cancel between
 * the request and the decision). A card left open would read "待批准" and the
 * strip would send its dead request id, which the engine discards as stale. */
function settleOpenApprovals(items: StreamItem[]): StreamItem[] {
  let changed = false;
  const next = items.map((it) => {
    if (it.kind === "tool" && it.approval && !it.approval.resolved) {
      changed = true;
      return { ...it, approval: { ...it.approval, resolved: true, approved: false } };
    }
    if (it.kind === "approval" && !it.resolved) {
      changed = true;
      return { ...it, resolved: true, approved: false };
    }
    return it;
  });
  return changed ? next : items;
}

/** Fail every tool card still waiting for its result. Tools run inside the
 * turn, so a settled turn has none in flight: an open card is one it left
 * behind — typically the children of a `batch_execute` or `delegate` whose
 * dispatch a cancel dropped, which never get a `ToolCallFinished` of their
 * own. Left open they would spin forever. */
function settleOpenTools(items: StreamItem[]): StreamItem[] {
  let changed = false;
  const now = Date.now();
  const next = items.map((it) => {
    if (it.kind === "tool" && it.content === undefined) {
      changed = true;
      return { ...it, content: "已中断", ok: false, ts: now };
    }
    return it;
  });
  return changed ? next : items;
}

// ---------------------------------------------------------------- artifacts
// Office mode's counterpart to `changes`: document/asset tools write whole
// files, never diffs. The call's args locator (`args_preview` live,
// `extractArgsPreview` on history replay) IS the file path for every
// office_*/web_download arg shape, so a string is all registration needs.

/** Tools whose `file`/`path` arg names a file the session WROTE — the
 *  first ok call registers it as an artifact, later ones bump the stamp. */
const ARTIFACT_WRITERS = new Set([
  "office_create",
  "office_set",
  "office_add",
  "office_remove",
  "office_move",
  "office_batch",
  "office_merge",
  "office_import",
  "office_save",
  "web_download",
]);
/** The escape hatches can write, but they can also just READ — they bump
 *  a tracked path's stamp but never register a new artifact (a read-only
 *  `office_exec view …` must not pop "产物已生成"). */
const ARTIFACT_TOUCHERS = new Set(["office_raw", "office_exec"]);

/** Every tool whose result the session-level summary — the changes panel and
 *  the artifacts dock — is rebuilt from. The host's `changes_feed` ships
 *  exactly these, so the list lives here, beside the parsers that read them. */
export const SUMMARY_TOOLS: readonly string[] = [
  ...WRITE_TOOLS,
  ...ARTIFACT_WRITERS,
  ...ARTIFACT_TOUCHERS,
];

/** Fold a finished tool call into `view.artifacts`. `locator` is the
 *  call's args preview — the workspace-relative file path for every
 *  office/asset tool shape. */
export function recordArtifact(
  artifacts: Artifact[],
  tool: string,
  locator?: string,
) {
  const writer = ARTIFACT_WRITERS.has(tool);
  if (!writer && !ARTIFACT_TOUCHERS.has(tool)) return;
  const rel = locator?.trim();
  // A locator that isn't a workspace-relative path (command text, a url,
  // an absolute path) can't be an artifact row.
  if (!rel || rel.startsWith("/") || rel.includes("://")) return;
  const i = artifacts.findIndex((a) => a.path === rel);
  if (i >= 0) {
    artifacts[i] = { ...artifacts[i], tool, updatedAt: Date.now() };
    return;
  }
  if (!writer) return;
  const name = rel.split("/").pop() ?? rel;
  const dot = name.lastIndexOf(".");
  artifacts.push({
    path: rel,
    name,
    ext: dot >= 0 ? name.slice(dot + 1).toLowerCase() : "",
    tool,
    updatedAt: Date.now(),
  });
}

function closeOpenThinking(items: StreamItem[]) {
  for (let i = 0; i < items.length; i++) {
    const it = items[i];
    if (it.kind === "thinking" && !it.done) {
      // Stamp the close time — the settled row's span is `ts - startedAt`,
      // so a remount/cached-view reuse doesn't lose the clock.
      items[i] = { ...it, done: true, ts: Date.now() };
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

/** The card a delegated child's own event belongs to. `child_id` names it
 *  exactly — two `delegate` calls in one turn can share a label like
 *  "subagent", so the label alone picks the wrong row. The label is only the
 *  fallback for events emitted before the wire carried ids. */
function childCardAt(
  items: SessionView["items"],
  childId: number | undefined,
  parent: string,
): number {
  if (childId != null) {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "tool" && it.parent === "delegate" && it.childId === childId) return i;
    }
    return -1;
  }
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "tool" && it.content === undefined && it.name === parent) return i;
  }
  return -1;
}

/** Append a delegated child's streamed text to its own card. Returns false when
 *  no such card exists — the text is then dropped rather than misattributed to
 *  the turn's draft. */
function appendChildText(
  items: SessionView["items"],
  childId: number | undefined,
  parent: string,
  text: string,
  reasoning: boolean,
): boolean {
  const i = childCardAt(items, childId, parent);
  if (i < 0) return false;
  const it = items[i];
  if (it.kind !== "tool") return false;
  // Reasoning is its own run, drawn by the same thinking row the main turn
  // uses — in the body, where it happened, not above the whole card.
  if (reasoning) {
    const segments = [...(it.segments ?? [])];
    const last = segments[segments.length - 1];
    if (last?.kind === "thinking") {
      segments[segments.length - 1] = { kind: "thinking", text: last.text + text };
    } else {
      segments.push({ kind: "thinking", text });
    }
    items[i] = { ...it, segments };
    return true;
  }
  const segments = [...(it.segments ?? [])];
  const last = segments[segments.length - 1];
  // Answer text extends the open text run, or opens a new one after a tool
  // run, so the body keeps its arrival order.
  if (last?.kind === "text") {
    segments[segments.length - 1] = { kind: "text", text: last.text + text };
  } else {
    segments.push({ kind: "text", text });
  }
  items[i] = { ...it, live: (it.live ?? "") + text, segments };
  return true;
}

/** Record a child tool run on its card: a tool segment whose index is the
 *  card's OWN tool-run count, matching the nested rows the card renders — a
 *  session-wide count would point past them. */
function appendChildTool(
  items: SessionView["items"],
  childId: number | undefined,
  parent: string,
) {
  const i = childCardAt(items, childId, parent);
  if (i < 0) return;
  const it = items[i];
  if (it.kind !== "tool") return;
  const segments = it.segments ?? [];
  const index = segments.filter((s) => s.kind === "tool").length;
  items[i] = { ...it, segments: [...segments, { kind: "tool", index }] };
}

/** Append a side note to a child's card — a steer it was sent, or a kernel
 *  notice about it. Drawn muted, in arrival order. */
function appendChildNote(
  items: SessionView["items"],
  childId: number | undefined,
  label: string,
  text: string,
) {
  const i = childCardAt(items, childId, label);
  if (i < 0) return;
  const it = items[i];
  if (it.kind !== "tool") return;
  items[i] = { ...it, segments: [...(it.segments ?? []), { kind: "text", text, muted: true }] };
}

/** The host echoes a steer only once it has a home — injected mid-turn
 *  (`UserSteer`) or run as the next prompt (`UserPrompt`). Either echo retires
 *  the FIRST matching「等待插入」row; duplicates queue up one row each, so one
 *  echo pops exactly one. Untouched arrays keep their identity. */
function retirePendingSteer(pending: string[], text: string): string[] {
  const want = text.trim();
  const at = pending.findIndex((p) => p.trim() === want);
  if (at < 0) return pending;
  return [...pending.slice(0, at), ...pending.slice(at + 1)];
}

export function applyEvent(
  v: SessionView,
  ev: AgentEventEnvelope["event"]
): SessionView {
  return applyEventInto(v, ev, [...v.items]);
}

/** Fold a whole batch into one view with a single `items` copy — the
 *  per-event `[...v.items]` made a burst cost O(events × history). Branches
 *  that return a different array (or the untouched `v.items`) hand it back
 *  un-owned, so it is re-copied before the next event may mutate it. */
export function applyEvents(
  v: SessionView,
  events: AgentEventEnvelope["event"][]
): SessionView {
  if (events.length === 0) return v;
  let owned = [...v.items];
  let cur = v;
  for (const ev of events) {
    cur = applyEventInto(cur, ev, owned);
    if (cur.items !== owned) owned = [...cur.items];
  }
  return cur.items === owned ? cur : { ...cur, items: owned };
}

function applyEventInto(
  v: SessionView,
  ev: AgentEventEnvelope["event"],
  items: SessionView["items"]
): SessionView {
  const last = () => items[items.length - 1];

  if (ev === "TurnRetry") {
    // The session rewound to before the last turn's user prompt — drop
    // that turn's items (user bubble included; the fresh `UserPrompt`
    // echo re-adds it). Any pending question/approval dies with the turn.
    // A steer bubble is not the turn's prompt — the backend rewinds past it
    // to the prompt that opened the turn, so the view must too.
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "user" && !it.steer) {
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
    // A settled turn is not waiting on anything — see `settleOpenApprovals`
    // and `settleOpenTools`.
    const settledItems = settled ? settleOpenTools(settleOpenApprovals(next)) : next;
    // A turn that no user prompt opened — the host waking the model on a
    // finished background job. It is still a turn: the clock starts here and
    // the stream treats what follows as inside one, exactly as if the user
    // had typed. `Compacting` is excluded: a `/compact` between turns is its
    // own block, and it already carries its own clock.
    const woke = streaming && !compacting && !v.turnOpen;
    return {
      ...v,
      items: settledItems,
      state: s,
      streaming,
      ...(settled
        ? { pendingQuestion: undefined, turnStartedAt: undefined, turnOpen: false }
        : woke
          ? { turnOpen: true, turnStartedAt: Date.now() }
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
  if ("UserSteer" in ev) {
    // A steer joins the running turn: a bubble, but no new turn, clock or draft.
    items.push({ kind: "user", text: ev.UserSteer, ts: Date.now(), steer: true });
    return { ...v, items, pendingSteers: retirePendingSteer(v.pendingSteers, ev.UserSteer) };
  }
  if ("UserPrompt" in ev) {
    items.push({ kind: "user", text: ev.UserPrompt, ts: Date.now() });
    return {
      ...v,
      items,
      // A steer that lost the race is run as this prompt — same retirement.
      pendingSteers: retirePendingSteer(v.pendingSteers, ev.UserPrompt),
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
    const { parent, text, child_id } = ev.TextDelta;
    appendChildText(items, child_id ?? undefined, parent, text, false);
    return { ...v, items };
  }
  if ("ReasoningDelta" in ev && ev.ReasoningDelta.parent) {
    const { parent, text, child_id } = ev.ReasoningDelta;
    // A child's reasoning joins the child's own body as a thinking run — never
    // its prose, and never the turn's thinking row.
    appendChildText(items, child_id ?? undefined, parent, text, true);
    return { ...v, items };
  }
  if ("TextDelta" in ev) {
    closeOpenThinking(items);
    const rate = bumpRate(v, ev.TextDelta.text);
    // The answer row is normally the tail. Auto-compaction mid-turn lands its
    // card in the MIDDLE of a streamed answer, right behind the draft; skip it
    // so the rest of the answer extends the same row instead of opening a
    // second bubble.
    let tail = items.length - 1;
    while (tail >= 0 && items[tail].kind === "compaction") tail--;
    const row = tail >= 0 ? items[tail] : undefined;
    if (row?.kind === "assistant")
      items[tail] = { ...row, text: row.text + ev.TextDelta.text, streaming: true };
    else items.push({ kind: "assistant", text: ev.TextDelta.text, streaming: true, startedAt: Date.now() });
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
    const name = ev.ToolCallStarted.name;
    const parent = ev.ToolCallStarted.parent ?? undefined;
    const childId = ev.ToolCallStarted.child_id ?? undefined;
    // A child's own call runs in the child's stream — it is not a step of this
    // turn, so it must not close the parent's thinking block.
    const internal = parent !== undefined && parent !== "delegate" && childId != null;
    if (!internal) closeOpenThinking(items);
    items.push({
      kind: "tool",
      name,
      args: ev.ToolCallStarted.args_preview,
      startedAt: Date.now(),
      // Rust Option<String> serializes `null`, not `undefined` —
      // normalize at the boundary so the Finished matcher's
      // `=== undefined` comparison actually works.
      parent,
      childId,
    });
    // A child's own tool call also lands on its card's body, in arrival order.
    // Its index is how many of that card's own calls came before it.
    if (parent && parent !== "delegate") appendChildTool(items, childId, parent);
    return { ...v, items };
  }
  if ("ToolCallFinished" in ev) {
    const t = ev.ToolCallFinished;
    const parent = t.parent ?? undefined;
    const childId = t.child_id ?? undefined;
    // Internal = a child's own call. It settles that child's card body only:
    // the parent's thinking block, approval cards and session summary (the
    // changes panel now reads git) all belong to the parent's stream.
    const internal = parent !== undefined && parent !== "delegate" && childId != null;
    let changes = v.changes;
    let artifacts = v.artifacts;
    if (!internal) {
      closeOpenThinking(items);
      // Fold a committed write's diff into the changes panel.
      changes = [...v.changes];
      if (t.ok && t.content) recordFileChanges(changes, t.name, t.content, false);
      else if (!t.ok) dropPendingChanges(changes, t.name);
      artifacts = [...v.artifacts];
      for (let i = items.length - 1; i >= 0; i--) {
        const it = items[i];
        if (it.kind === "approval" && !it.resolved && it.toolName === t.name) {
          items[i] = { ...it, resolved: true, approved: t.ok };
          break;
        }
      }
    }
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      // Same-parent, same-child match: a batch child's Finished lands on the
      // child row, not the batch capsule itself (and vice versa), and two
      // delegated children sharing a label stay apart by `childId`.
      if (
        it.kind === "tool" &&
        it.name === t.name &&
        it.content === undefined &&
        it.parent === parent &&
        it.childId === childId
      ) {
        items[i] = {
          ...it,
          content: t.content,
          ok: t.ok,
          uiType: t.ui_type,
          // Result stamp — the step's settled span is `ts - startedAt`,
          // same shape a replayed row carries.
          ts: Date.now(),
          approval: it.approval
            ? { ...it.approval, resolved: true, approved: t.ok }
            : undefined,
          waiting: undefined,
        };
        // Only the matched tool's locator is a real file path — read it
        // off the started row's args before the loop ends.
        if (t.ok && !internal) recordArtifact(artifacts, t.name, it.args);
        break;
      }
    }
    // A top-level result is one history Tool row — advance the live count
    // (re-based by `husk:session_counts` when the turn commits). Delegate
    // children run in their own history and do not count.
    const taskTotal =
      v.taskTotal != null && parent === undefined && childId == null ? v.taskTotal + 1 : v.taskTotal;
    return { ...v, items, changes, artifacts, taskTotal };
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
            actions: a.actions,
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
        actions: a.actions,
        resolved: false,
      });
    }
    return { ...v, items, changes };
  }
  if ("ApprovalTimedOut" in ev) {
    // The card expired unanswered and the kernel auto-denied it. Marking it
    // resolved keeps the composer strip off a dead request id; `timedOut` is
    // what the card reads, since 已拒绝 would hide that nobody decided. The
    // `ToolCallFinished(ok:false)` that follows spreads this approval object,
    // so the mark survives the result landing.
    const id = ev.ApprovalTimedOut.request_id;
    let hit = false;
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "tool" && it.approval?.requestId === id) {
        items[i] = {
          ...it,
          approval: { ...it.approval, resolved: true, approved: false, timedOut: true },
        };
        hit = true;
      } else if (it.kind === "approval" && it.requestId === id) {
        items[i] = { ...it, resolved: true, approved: false, timedOut: true };
        hit = true;
      }
    }
    return hit ? { ...v, items } : v;
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
  if ("QuestionTimedOut" in ev) {
    // Nobody answered in time, so the agent decides the rest on its own. The
    // card goes away exactly as an answer would clear it — a stale id must not
    // drop a newer question — and the stream says what happened, since the
    // turn carries on without a user reply.
    if (v.pendingQuestion?.requestId !== ev.QuestionTimedOut.request_id) return v;
    closeOpenThinking(items);
    items.push({
      kind: "system",
      text: "提问已超时，代理将自行决定后续",
      standalone: !v.turnOpen,
      ts: Date.now(),
    });
    return { ...v, pendingQuestion: undefined, items };
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
    items.push({ kind: "system", text: ev.SystemMessage, standalone: !v.turnOpen, ts: Date.now() });
    return { ...v, items };
  }
  if ("Usage" in ev) {
    // Some wires emit an all-zero Usage on a degraded/failed round. That is
    // not a measurement; letting it replace the last real one blanked the
    // header until the next successful turn. (Cache writes are not on this
    // event — they follow as `husk:usage_cache_write`.)
    const u = ev.Usage;
    // One Usage per top-level model request (delegate children don't forward
    // theirs) — the live stand-in for "one more assistant row". Re-based by
    // `husk:session_counts` when the turn commits.
    //
    // It is an UPPER BOUND, on purpose. The engine pushes exactly one
    // assistant row per round, tool-only rounds included (a round with no
    // text still lands `{role: assistant, tool_calls: …}`), so `Usage` is
    // 1:1 with rows for every normal round. The exception is a quiet round
    // whose `final_text` came back empty — the turn policy nudges and loops
    // instead of pushing a row, and that round's `Usage` has no row behind
    // it. There is no per-round "a row landed" signal on the wire, and
    // counting assistant *items* instead would under-count every text-less
    // tool round (the common case in agentic work), which is far worse.
    // So: tick up per round, let the commit's `husk:session_counts` snap the
    // figure down to the rows actually persisted. The header can step down by
    // one at turn end; it never shows a stale maximum after `/clear`.
    const runTotal = v.runTotal != null ? v.runTotal + 1 : v.runTotal;
    if (
      u.prompt_tokens === 0 &&
      u.completion_tokens === 0 &&
      u.cached_tokens === 0 &&
      v.usage.prompt > 0
    ) {
      return {
        ...v,
        runTotal,
        usage: {
          ...v.usage,
          contextWindow: u.context_window || v.usage.contextWindow,
        },
      };
    }
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
      runTotal,
      usage: {
        prompt: ev.Usage.prompt_tokens,
        completion: ev.Usage.completion_tokens,
        contextWindow: ev.Usage.context_window,
        cachedTokens: ev.Usage.cached_tokens,
        // Cache writes ride the product event that follows this Usage; clear
        // the previous round's reading first so a provider switch cannot
        // inherit it.
        cacheWriteTokens: 0,
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
      // The split described the pre-compaction request, like the cache figures.
      contextBreakdown: undefined,
      usage: {
        ...v.usage,
        prompt: c.after_tokens,
        // The pre-compaction cache reading describes a request body that no
        // longer exists — leaving it would make the uncached chip under-read
        // until the next `Usage` lands.
        cachedTokens: 0,
        cacheWriteTokens: 0,
        ...(c.context_window > 0 ? { contextWindow: c.context_window } : {}),
      },
    };
  }
  if ("Error" in ev) {
    closeOpenThinking(items);
    items.push({ kind: "system", text: `⚠ ${ev.Error}`, standalone: !v.turnOpen, ts: Date.now() });
    return { ...v, items, streaming: false };
  }
  if ("Custom" in ev && ev.Custom.kind === "husk:session_counts") {
    // Authoritative counts of the just-persisted history.
    const p = ev.Custom.payload as { runs?: number; tasks?: number; turns?: number } | null;
    if (typeof p?.runs !== "number" || typeof p?.tasks !== "number") return v;
    return {
      ...v,
      runTotal: p.runs,
      taskTotal: p.tasks,
      // `turns` is optional: a payload without it leaves the opening
      // snapshot's figure alone.
      ...(typeof p.turns === "number" ? { turnTotal: p.turns } : {}),
    };
  }
  if ("Custom" in ev && ev.Custom.kind === "husk:bg_jobs") {
    // The host's whole list — replace, never merge: a removed job is simply
    // absent from the next snapshot.
    const jobs = (ev.Custom.payload as { jobs?: unknown } | null)?.jobs;
    if (!Array.isArray(jobs)) return v;
    return { ...v, bgJobs: jobs as BgJob[], bgJobsAt: Date.now() };
  }
  if ("Custom" in ev && ev.Custom.kind === "husk:usage_cache_write") {
    // Product extension accompanying the preceding `Usage`: the shared Usage
    // shape has no cache-write slot. Payload is the round's token count.
    const payload = ev.Custom.payload;
    const tokens = typeof payload === "number" ? payload : Number(payload) || 0;
    return { ...v, usage: { ...v.usage, cacheWriteTokens: tokens } };
  }
  if ("Custom" in ev && ev.Custom.kind === "husk:context_breakdown") {
    // Product extension accompanying the preceding `Usage`: the request's
    // estimated prompt composition.
    const p = ev.Custom.payload;
    if (p && typeof p === "object") {
      return { ...v, contextBreakdown: p as ContextBreakdown };
    }
    return v;
  }
  if ("Custom" in ev && ev.Custom.kind === "plan_submitted") {
    // The plan card IS the deliverable — it lands after the turn's final
    // assistant text so it folds into this turn and closes it.
    closeOpenThinking(items);
    try {
      const plan = JSON.parse((ev.Custom.payload as { plan: string }).plan);
      items.push({ kind: "plan", plan, ts: Date.now() });
    } catch {
      // A malformed payload degrades to a system line, not a dropped turn.
      items.push({ kind: "system", text: "计划已提交（解析失败）", hi: undefined, ts: Date.now() });
    }
    return { ...v, items };
  }
  if ("Custom" in ev && ev.Custom.kind === "blueprint_report") {
    // Same shape the persisted notice row replays as: errors draw as a
    // `⚠` line, warnings-only as a plain system line.
    closeOpenThinking(items);
    const { errors, text } = ev.Custom.payload as { errors: number; text: string };
    items.push({ kind: "system", text: errors > 0 ? `⚠ ${text}` : text, standalone: !v.turnOpen, ts: Date.now() });
    return { ...v, items };
  }
  if ("Custom" in ev && typeof ev.Custom.kind === "string" && ev.Custom.kind.startsWith("subagent:")) {
    // A delegated child's own lifecycle notes — they belong on its card, never
    // on the turn's stream.
    const p = (ev.Custom.payload ?? {}) as {
      child_id?: number | null;
      label?: string;
      text?: string;
      question?: string;
      answer?: string;
    };
    const childId = p.child_id ?? undefined;
    const label = p.label ?? "";
    const at = childCardAt(items, childId, label);
    if (at < 0) return v;
    const card = items[at];
    if (card.kind !== "tool") return v;
    switch (ev.Custom.kind) {
      case "subagent:waiting":
        items[at] = { ...card, waiting: p.question ?? "" };
        return { ...v, items };
      case "subagent:resumed":
        items[at] = {
          ...card,
          waiting: undefined,
          segments: [
            ...(card.segments ?? []),
            { kind: "text", text: `> 主代理回复：${p.answer ?? ""}`, muted: true },
          ],
        };
        return { ...v, items };
      case "subagent:steer":
        items[at] = {
          ...card,
          segments: [
            ...(card.segments ?? []),
            { kind: "text", text: `> 用户插话：${p.text ?? ""}`, muted: true },
          ],
        };
        return { ...v, items };
      default:
        items[at] = {
          ...card,
          segments: [...(card.segments ?? []), { kind: "text", text: p.text ?? "", muted: true }],
        };
        return { ...v, items };
    }
  }
  if ("QueuedPrompts" in ev) {
    // The kernel's parked list is the single source of truth — every
    // SetQueued write and each drain pop echoes the new list here.
    return { ...v, queuedPrompts: ev.QueuedPrompts.items };
  }
  return v;
}
