// Notification center — module-level pub/sub store feeding the sidebar bell,
// plus the rule that decides what deserves a row. Producers hand it a kernel
// event (`notifyForEvent`); the popover renders newest-first with an unread
// badge on the trigger. One row per session: it carries that session's latest
// state worth telling, not a log of every one. In-memory only — notifications
// are session-scoped signals, not history.

import { useSyncExternalStore } from "react";
import type { AgentEventEnvelope, AgentState, UiEvent } from "@/types";
import { isAppInFront, sendDesktopNotification } from "@/lib/desktop-notify";
import { playCue } from "@/lib/notification-sound";
import { jobTone } from "@/lib/bg-jobs";
import type { BgJob } from "@/features/chat/hooks/stream-view";

/** `turn` — a turn ended on its own. `attention` — the agent is blocked on
 *  the user. `error` — it gave up. Drives the row's dot colour. */
export type NotificationKind = "turn" | "attention" | "error";

export interface AppNotification {
  id: number;
  ts: number;
  kind: NotificationKind;
  title: string;
  body?: string;
  /** Session navigation target — clicking jumps to that conversation. */
  root?: string;
  session?: number;
  read: boolean;
}

export interface NotificationInput {
  kind: NotificationKind;
  title: string;
  body?: string;
  root?: string;
  session?: number;
}

const MAX_ITEMS = 50;

let items: AppNotification[] = [];
let nextId = 1;
/** `root:id` → sidebar title. The store only carries root+id; the app shell
 *  feeds the names (`setSessionTitles`) so the bell row and the desktop toast
 *  read the same label. */
let titles: ReadonlyMap<string, string> = new Map();
const listeners = new Set<() => void>();

function emit() {
  for (const fn of listeners) fn();
}

/** Session identity — the row key, and the target of "this conversation is
 *  read now". A row with no session (none today) keys on the empty one. */
function target(n: { root?: string; session?: number }) {
  return `${n.root ?? ""}:${n.session ?? 0}`;
}

export function notify(input: NotificationInput): AppNotification {
  const n: AppNotification = {
    id: nextId++,
    ts: Date.now(),
    read: false,
    ...input,
  };
  // The session's previous row is superseded, not stacked: a finish after a
  // confirmation prompt makes that prompt stale, and a session that keeps
  // failing must not bury every other conversation in the list.
  items = [n, ...items.filter((old) => target(old) !== target(n))].slice(0, MAX_ITEMS);
  emit();
  return n;
}

export function markRead(id: number) {
  if (!items.some((n) => n.id === id && !n.read)) return;
  items = items.map((n) => (n.id === id ? { ...n, read: true } : n));
  emit();
}

export function markAllRead() {
  if (!items.some((n) => !n.read)) return;
  items = items.map((n) => ({ ...n, read: true }));
  emit();
}

/** Opening a conversation reads everything it raised — the badge counts what
 *  the user has not looked at, and the conversation IS the thing. */
export function markSessionRead(root: string, session: number) {
  const key = target({ root, session });
  if (!items.some((n) => !n.read && target(n) === key)) return;
  items = items.map((n) => (!n.read && target(n) === key ? { ...n, read: true } : n));
  emit();
}

/** The session moved on from what it was blocked on — its prompt row asks
 *  for something nobody needs to give any more, so it goes, read or not. */
function dismissAttention(root: string, session: number) {
  const key = target({ root, session });
  if (!items.some((n) => n.kind === "attention" && target(n) === key)) return;
  items = items.filter((n) => !(n.kind === "attention" && target(n) === key));
  emit();
}

export function clearAll() {
  if (items.length === 0) return;
  items = [];
  emit();
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function useNotifications() {
  return useSyncExternalStore(subscribe, () => items, () => items);
}

export function useUnreadCount() {
  const list = useNotifications();
  return list.reduce((n, i) => n + (i.read ? 0 : 1), 0);
}

/** The list outside React — tests and callbacks that run after a render. */
export function notificationsNow(): readonly AppNotification[] {
  return items;
}

/** Feed the session names — called by the app shell whenever its project
 *  list changes. A no-op when nothing was renamed, added or removed. */
export function setSessionTitles(
  projects: readonly { root: string; sessions: readonly { id: number; title: string }[] }[],
  partial = false,
) {
  // A sidebar page is not an exhaustive catalog. Retain names already learned
  // from other pages so a poll cannot erase a notification's known title.
  const next = partial ? new Map(titles) : new Map<string, string>();
  for (const p of projects) {
    for (const s of p.sessions) next.set(target({ root: p.root, session: s.id }), s.title);
  }
  if (next.size === titles.size && [...next].every(([k, v]) => titles.get(k) === v)) return;
  titles = next;
  emit();
}

export function useSessionTitles() {
  return useSyncExternalStore(subscribe, () => titles, () => titles);
}

/** What happened, and to which conversation — the bell row's title and the
 *  desktop toast's. A session the index does not know (yet) falls back to
 *  its number. */
export function notificationHeadline(
  n: { title: string; root?: string; session?: number },
  index: ReadonlyMap<string, string> = titles
): string {
  if (n.root === undefined || n.session === undefined) return n.title;
  return `${n.title} — ${index.get(target(n)) || `会话 #${n.session}`}`;
}

// ------------------------------------------------------------------- rules

/** Kernel's cancel sentinel — every cooperative-cancel exit path sets
 *  `Failed(CANCEL_ERR)`. Cancelling is the user's own doing, so it must not
 *  ring the bell (nor print the raw English sentinel as a body). */
const CANCEL_REASON = "turn cancelled by user";

/** What a kernel event is worth telling the user about, or `null`. Pure: the
 *  caller owns the "can the user already see it" half. */
export function notificationFor(ev: UiEvent): NotificationInput | null {
  if (typeof ev === "object" && "StateChanged" in ev) return forState(ev.StateChanged);
  if (typeof ev === "object" && "QuestionAsked" in ev) {
    return { kind: "attention", title: "等待你的回答", body: ev.QuestionAsked.question };
  }
  return null;
}

function forState(st: AgentState): NotificationInput | null {
  if (st === "Finished") return { kind: "turn", title: "会话已完成" };
  if (typeof st !== "object") return null;
  if ("Failed" in st) {
    return st.Failed === CANCEL_REASON
      ? null
      : { kind: "error", title: "会话出错", body: st.Failed };
  }
  // Blocked on the user: nothing moves until they answer, and a parked
  // workspace's prompt is exactly what the bell exists for.
  if ("AwaitingToolConfirmation" in st) {
    return { kind: "attention", title: "等待你的确认", body: st.AwaitingToolConfirmation.tool_name };
  }
  if ("AwaitingPluginConsent" in st) {
    return { kind: "attention", title: "等待插件授权", body: st.AwaitingPluginConsent.plugin_id };
  }
  if ("AwaitingConsent" in st) {
    return { kind: "attention", title: "等待你的确认", body: st.AwaitingConsent.kind };
  }
  return null;
}

/** Failed background jobs already told, per session. `husk:bg_jobs` is the
 *  whole list on every change, so a failure stays in every later snapshot —
 *  remembering the ids is what makes it one notice, not one per snapshot.
 *  Jobs die with their session's actor and the feed only speaks on a change,
 *  so the live stream never replays an old failure; a reopened session's
 *  `bg_list` seed does not pass through here at all. */
const toldFailures = new Map<string, Set<string>>();

/** Forget a session's bookkeeping. Session ids are never reused, so nothing in
 *  the map can be read again — leaving the entry would only grow it. */
export function forgetToldFailures(root: string, session: number) {
  toldFailures.delete(target({ root, session }));
}

/** A background job of this session failed since the last snapshot. */
function bgFailureFor(root: string, session: number, ev: UiEvent): NotificationInput | null {
  if (typeof ev !== "object" || !("Custom" in ev) || ev.Custom.kind !== "husk:bg_jobs") return null;
  const jobs = (ev.Custom.payload as { jobs?: BgJob[] } | null)?.jobs;
  if (!Array.isArray(jobs)) return null;
  const key = target({ root, session });
  const told = toldFailures.get(key) ?? new Set<string>();
  toldFailures.set(key, told);
  // A restored row is the record of an earlier run, already on the pill when
  // the session opened — it rides every snapshot now, but it is not news.
  const fresh = jobs.filter((j) => !j.restored && jobTone(j) === "failed" && !told.has(j.id));
  if (fresh.length === 0) return null;
  for (const j of fresh) told.add(j.id);
  return {
    kind: "error",
    title: fresh.length === 1 ? "后台任务失败" : `${fresh.length} 个后台任务失败`,
    body: fresh.map((j) => j.command).join("\n"),
  };
}

/** Whether the session left whatever it was blocked on: an answered question,
 *  or any state change that is not itself a wait (approved → executing,
 *  denied → finished, cancelled → failed). */
function resolvesAttention(ev: UiEvent): boolean {
  if (typeof ev !== "object") return false;
  if ("QuestionAnswered" in ev) return true;
  if (!("StateChanged" in ev)) return false;
  const st = ev.StateChanged;
  return !(
    typeof st === "object" &&
    ("AwaitingToolConfirmation" in st || "AwaitingPluginConsent" in st || "AwaitingConsent" in st)
  );
}

/** Deliver one kernel event. The rule is "the user cannot see it": a session
 *  that is not the foreground one, or a window that is not in front. Husk
 *  closes to the tray, so the window is routinely off screen — "is it the
 *  foreground session" alone under-notifies, while the foreground session's
 *  own turn end is already on screen. Resolving a prompt is not gated on
 *  visibility: the stale row goes wherever the answer came from.
 *
 *  The cue is NOT gated on that: what it announces is the agent loop stopping
 *  (finished, blocked on an approval or a question, failed), and that is worth
 *  hearing whether or not the window happens to be in front. The store's own
 *  throttle keeps a busy session from turning it into a drumbeat. */
export function notifyForEvent(
  env: AgentEventEnvelope,
  foreground: { root: string; id: number }
): AppNotification | null {
  const root = env.root ?? "";
  if (resolvesAttention(env.event)) dismissAttention(root, env.session);
  const input = notificationFor(env.event) ?? bgFailureFor(root, env.session, env.event);
  if (input) playCue(input.kind);
  if (root === foreground.root && env.session === foreground.id && isAppInFront()) {
    return null;
  }
  if (!input) return null;
  const n = notify({ ...input, root, session: env.session });
  sendDesktopNotification({
    title: notificationHeadline(n),
    body: n.body,
    target: { root, session: env.session },
  });
  return n;
}
