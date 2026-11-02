// Status-strip visibility and ordering share one module registry.

import type { ReactNode } from 'react';

/** One entry per module this build knows. `defaultOn` decides what a reader who
 *  never opened the customiser sees; the order here is the built-in order. */
export const STATUS_MODULES = [
	{ id: 'stream-health', label: '生成状态', defaultOn: false },
	{ id: 'git', label: 'Git 分支', defaultOn: true },
	{ id: 'activity', label: '生成活动', defaultOn: false },
	{ id: 'tokens', label: 'Token 用量', defaultOn: true },
	{ id: 'context', label: '上下文窗口', defaultOn: true },
	{ id: 'todos', label: '任务清单', defaultOn: false },
	{ id: 'subagents', label: '子代理', defaultOn: false },
	{ id: 'cost', label: '本轮成本', defaultOn: true },
] as const;

export type StatusModuleId = (typeof STATUS_MODULES)[number]['id'];

/** Modules visible in a new layout. */
export const DEFAULT_STATUS_ITEMS: readonly string[] = STATUS_MODULES.flatMap(
	(m) => (m.defaultOn ? [m.id] : [])
);

/** The layout as stored in `appearance.status_strip`. */
export interface StatusStripLayout {
	items?: string[] | null;
	hidden?: string[] | null;
}

/** Resolve visible modules in order, preserving explicit hidden entries. */
export function resolveStripOrder(
	stored: StatusStripLayout | null | undefined,
	known: readonly string[] = STATUS_MODULES.map((m) => m.id)
): string[] {
	const hidden = new Set(stored?.hidden ?? []);
	const asked = (stored?.items ?? []).filter((id) => known.includes(id));
	// No stored layout → what the build turns on by default, not "everything".
	const order = asked.length > 0 ? asked : [...DEFAULT_STATUS_ITEMS];
	// A known id the stored list forgot is appended at the end, in registry order
	// when it is on by default: a module added after the layout was written still
	// shows up, and the reader can move it where they want it. An off-by-default
	// module is left out until the customiser asks for it.
	for (const id of known) {
		if (order.includes(id)) continue;
		const module = STATUS_MODULES.find((m) => m.id === id);
		if (module?.defaultOn) order.push(id);
	}
	return order.filter((id) => !hidden.has(id));
}

/** The full display order for a stored layout: every id this build knows, the
 *  reader's order first, the rest in registry order. Writing the full list keeps
 *  a later toggle from having to guess where a module belonged. */
export function fullStripOrder(
	stored: StatusStripLayout | null | undefined
): string[] {
	// EVERY known module, default-off ones included: the editor has to be able to
	// switch a module on, so its row cannot be missing from the list it edits.
	const known: string[] = STATUS_MODULES.map((m) => m.id);
	const named = (stored?.items ?? []).filter((id) => known.includes(id));
	const order = named.length > 0 ? named : [...DEFAULT_STATUS_ITEMS];
	for (const id of known) if (!order.includes(id)) order.push(id);
	return order;
}

/** `order` with `id` moved to `index`, clamped. */
export function moveStatusItem(
	order: readonly string[],
	id: string,
	index: number
): string[] {
	const from = order.indexOf(id);
	if (from < 0) return [...order];
	const rest = order.filter((_, i) => i !== from);
	const at = Math.max(0, Math.min(rest.length, index));
	return [...rest.slice(0, at), id, ...rest.slice(at)];
}

/** `hidden` with `id` toggled on or off. */
export function toggleStatusItem(
	hidden: readonly string[],
	id: string,
	visible: boolean
): string[] {
	const next = new Set(hidden);
	if (visible) next.delete(id);
	else next.add(id);
	return [...next];
}

/** Renders the ordered nodes. `nodes` is keyed by module id; a missing or empty
 *  node is skipped, which is how a module says "nothing to show right now". */
export function StatusStrip({
	order,
	nodes,
}: {
	order: readonly string[];
	nodes: Record<string, ReactNode>;
}) {
	return (
		<>
			{order.map((id) =>
				nodes[id] ? (
					<span key={id} className="contents">
						{nodes[id]}
					</span>
				) : null
			)}
		</>
	);
}
