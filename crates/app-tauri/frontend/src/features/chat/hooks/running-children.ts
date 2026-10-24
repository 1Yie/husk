import { childIdentity, type SessionView } from "@/features/chat/hooks/stream-view";
import { answerChildId, delegateChildren } from "@/features/chat/hooks/view-from-history";
import { reportBody } from "@/features/chat/hooks/report-text";
import type { ToolFeedRow } from "@/lib/agent-ipc/sessions";

/** A subagent still running in this session — a mid-run steer names its `childId`. */
export interface RunningChild {
  childId: number;
  name: string;
  task: string;
  /** The question it parked on for the parent agent, while it waits. */
  waiting?: string;
}

/** Subagents still running in this session — the ones a mid-run steer can reach.
 * A card is running until its report lands (`content` unset). */
export function extractRunningChildren(view: SessionView): RunningChild[] {
  if (!view || !view.items) return [];
  return view.items.flatMap((it) =>
    it.kind === "tool" && it.parent === "delegate" && it.childId != null && it.content === undefined
      ? [{ childId: it.childId, name: it.name, task: it.args, waiting: it.waiting }]
      : [],
  );
}

/** The same question as a boolean — a sidebar row only switches glyphs, and
 *  must not allocate a list per store batch. */
export function hasRunningChildren(view: SessionView): boolean {
  if (!view || !view.items) return false;
  return view.items.some(
    (it) => it.kind === "tool" && it.parent === "delegate" && it.childId != null && it.content === undefined,
  );
}

/** Where a delegated child stands. `aborted` is a cancel; `failed` is a child
 *  that ended in error and left a body saying why. */
export type ChildStatus = "running" | "waiting" | "done" | "failed" | "aborted";

/** Every delegated child of the session, finished ones included — the 子代理
 *  view keeps a child after its report lands. */
export interface SessionChild {
  /** Absent on a replayed parallel child whose report carried no id. */
  childId?: number;
  name: string;
  task: string;
  status: ChildStatus;
  /** The parked question while `waiting`. */
  waiting?: string;
  /** The child's report once it ended. */
  report?: string;
  /** The child's history row (global index, same coordinate as `historyStart`).
   *  Children folded out of the session-level feed carry the `delegate` row
   *  they came from, so the merge below can tell which of them the mounted
   *  page has caught up with. */
  hi?: number;
  /** The child's place in its `delegate` batch. `hi` is the PARENT row's index,
   *  so every child of one batch shares it — the position is what keeps two
   *  id-less siblings apart. */
  childIndex?: number;
}

/** The wording a cancelled card carries — live and replayed alike. */
const CANCELLED_BODY = "已中断";

/** The fields a delegated child's card carries — a `StreamItem` tool card and
 *  the cards `delegateChildren` folds out of a persisted result. */
interface DelegateCard {
  childId?: number;
  name: string;
  args: string;
  content?: string;
  ok?: boolean;
  waiting?: string;
  hi?: number;
  childIndex?: number;
}

/** One delegate card as the sidebar row it becomes — the single mapping both
 *  the mounted page and the session-level feed go through. `null` for a card
 *  that can never settle: no id means nothing in the log could close it. */
export function cardToChild(card: DelegateCard): SessionChild | null {
  const base = {
    childId: card.childId,
    name: card.name,
    task: card.args,
    hi: card.hi,
    childIndex: card.childIndex,
  };
  // A parked child is still out — on reopen its card holds the early return
  // as its body, so the question decides before the body does.
  if (card.waiting != null) return { ...base, status: "waiting", waiting: card.waiting };
  if (card.content === undefined) {
    if (card.childId == null) return null;
    return { ...base, status: "running" };
  }
  if (card.content === CANCELLED_BODY) return { ...base, status: "aborted" };
  // The kernel's header and a batch section marker are the model's metadata,
  // not the child's answer — the view shows the body under them.
  const report = reportBody(card.content) || undefined;
  return { ...base, status: card.ok === false ? "failed" : "done", report };
}

/** Every delegated child in arrival order. Finished cards come back from the
 *  persisted log on reopen, so the list survives a restart with no store of
 *  its own. */
export function extractChildren(view: SessionView): SessionChild[] {
  if (!view || !view.items) return [];
  return view.items.flatMap((it): SessionChild[] => {
    if (it.kind !== "tool" || it.parent !== "delegate") return [];
    const child = cardToChild(it);
    return child ? [child] : [];
  });
}

/** Delegated children of the WHOLE persisted log, rebuilt from `tool_rows_feed`.
 *  The mounted page is only the newest slice of a session, so without this the
 *  子代理 list would shrink to the children that happened to land in it. Every
 *  row folds, mounted or not: this is the snapshot `sessionChildren` merges the
 *  page into, so the two must describe the same children in the same
 *  coordinate (`hi` = the feed row's global index). */
export function childrenFromFeed(rows: readonly ToolFeedRow[]): SessionChild[] {
  const out: SessionChild[] = [];
  // The feed ships the log's own order, but sorting is free at this size and
  // makes an `answer_child` row land after the delegate row it answers.
  const ordered = [...rows].sort((a, b) => a.index - b.index);
  for (const row of ordered) {
    if (row.tool === "delegate") {
      // Same fold the history replay uses, so a child reads the same whether
      // its page is mounted or only summarized here.
      for (const card of delegateChildren(row.args, row.content, row.is_error)) {
        const child = cardToChild({ ...card, hi: row.index });
        if (child) out.push(child);
      }
      continue;
    }
    if (row.tool === "answer_child") {
      const childId = answerChildId(row.args);
      if (childId == null) continue;
      let i = out.length - 1;
      while (i >= 0 && out[i].childId !== childId) i--;
      if (i < 0) continue;
      // `answer_child` waits for the resumed child's next outcome, so its own
      // result settles the card: a report, a cancel marker, or another park —
      // the same three shapes a `delegate` result takes, read the same way.
      const [next] = delegateChildren("", row.content, row.is_error);
      const settled = next ? cardToChild(next) : null;
      out[i] = settled
        ? {
            ...settled,
            childId,
            name: out[i].name,
            task: out[i].task,
            hi: out[i].hi,
            childIndex: out[i].childIndex,
          }
        : { ...out[i], status: "running", waiting: undefined };
    }
  }
  return out;
}

/** A child's identity across the two sources. The feed and the mounted page
 *  both stamp `hi` with the `delegate` row's global index, so the same child
 *  lands on the same key either way; a live card has no `hi` but always carries
 *  the id a steer names. Also the key a cleared card is remembered by.
 *
 *  `hi` alone is the PARENT's index: every child of one batch shares it, so a
 *  child with no id also carries its place in that batch — without it clearing
 *  one card cleared its siblings, and the merge read two children as one.
 *
 *  The id alone is not a child's identity either: ids restart with each process,
 *  so a session's old transcript can hold a child with the same id as a new one.
 *  `hi` is what tells those apart, so it leads whenever the child has one. */
export function childKey(c: SessionChild): string {
  if (c.hi != null) return `h${c.hi}-${childIdentity(c.childId, c.childIndex)}`;
  return c.childId != null ? `c${c.childId}` : `h${c.hi}-${childIdentity(undefined, c.childIndex)}`;
}

/** The children a reader has not cleared out of the 子代理 view. The list is
 *  rebuilt from the log on every event, so the dismissed keys — not the list —
 *  are what keep a cleared card gone. */
export function withoutDismissed(
  children: SessionChild[],
  dismissed: ReadonlySet<string>,
): SessionChild[] {
  if (dismissed.size === 0) return children;
  return children.filter((c) => !dismissed.has(childKey(c)));
}

/** A card nothing will move any more — the states the merge prefers over a
 *  running or parked one. */
function isSettled(c: SessionChild): boolean {
  return c.status === "done" || c.status === "failed" || c.status === "aborted";
}

/** Every delegated child of the session — for the 子代理 sidebar, which is a
 *  session-level list and must not shrink to whatever history happens to be
 *  loaded.
 *
 *  The feed is the base: it describes the whole log in one order, so a child
 *  whose `delegate` row sits above the page boundary, or whose `answer_child`
 *  settled it on a page never loaded, still shows up here. The mounted page
 *  only overrides what it knows better — a live card's state, or a settled card
 *  the feed has not caught up with (a page refetched after the report landed). */
export function sessionChildren(view: SessionView): SessionChild[] {
  const mounted = extractChildren(view);
  const feed = view.feedChildren;
  if (!feed || feed.length === 0) return mounted;
  const onPage = new Map(mounted.map((c) => [childKey(c), c]));
  const merged: SessionChild[] = [];
  const claimed = new Set<string>();
  for (const fed of feed) {
    claimed.add(childKey(fed));
    const page = onPage.get(childKey(fed)) ?? liveTwinOf(fed, mounted);
    if (!page) {
      merged.push(fed);
      continue;
    }
    claimed.add(childKey(page));
    // A settled reading wins over an open one: the page may still hold the
    // parked card whose `delegate` row it has, while the feed already saw the
    // `answer_child` that finished it. Between two settled readings the page
    // wins — it is the later window, and the feed's copy can only be staler.
    merged.push(isSettled(fed) && !isSettled(page) ? fed : page);
  }
  // Children the feed does not know at all — a live card drawn from events,
  // which the persisted log has no row for yet.
  for (const c of mounted) if (!claimed.has(childKey(c))) merged.push(c);
  return merged;
}

/** A live card (no `hi`, drawn from events) that is the same child as an
 *  unsettled feed row with the same id — the card catching up with its own row.
 *  A settled feed child is a different child from a live card reusing its id:
 *  ids restart with each process, so the old one must not absorb the new one. */
function liveTwinOf(fed: SessionChild, mounted: SessionChild[]): SessionChild | undefined {
  if (fed.childId == null || isSettled(fed)) return undefined;
  return mounted.find((c) => c.hi === undefined && c.childId === fed.childId);
}

/** Still out: a steer can reach it. */
export function childIsLive(c: SessionChild): boolean {
  return c.status === "running" || c.status === "waiting";
}
