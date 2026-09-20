// Notification center — module-level pub/sub store feeding the sidebar
// bell. Producers call `notify()`; the popover renders newest-first with
// an unread badge on the trigger. In-memory only — notifications are
// session-scoped signals, not history.

import { useSyncExternalStore } from "react";

export type NotificationKind = "turn" | "update" | "info";

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
const listeners = new Set<() => void>();

function emit() {
  for (const fn of listeners) fn();
}

export function notify(input: NotificationInput): AppNotification {
  const n: AppNotification = {
    id: nextId++,
    ts: Date.now(),
    read: false,
    ...input,
  };
  items = [n, ...items].slice(0, MAX_ITEMS);
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
