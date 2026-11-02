// The strip renders what the reader asked for — in their order, without the
// modules they switched off.

import { describe, expect, test } from 'vitest';
import {
	DEFAULT_STATUS_ITEMS,
	STATUS_MODULES,
	fullStripOrder,
	resolveStripOrder,
} from '../strip';

/** What a stored list of `named` ids resolves to: the names it kept, then the
 *  default-on modules it forgot, in registry order. */
const withDefaults = (named: string[]) => [
	...named,
	...DEFAULT_STATUS_ITEMS.filter((id) => !named.includes(id)),
];

describe('resolveStripOrder', () => {
	test('no stored layout is the built-in order', () => {
		expect(DEFAULT_STATUS_ITEMS).toEqual(['git', 'tokens', 'context', 'cost']);
		expect(resolveStripOrder(null)).toEqual([...DEFAULT_STATUS_ITEMS]);
		expect(resolveStripOrder(undefined)).toEqual([...DEFAULT_STATUS_ITEMS]);
	});

	test('the editor retains every module for DIY while starting with the default four', () => {
		const order = fullStripOrder(null);
		expect(order.slice(0, 4)).toEqual(['git', 'tokens', 'context', 'cost']);
		expect(new Set(order)).toEqual(
			new Set(STATUS_MODULES.map((module) => module.id))
		);
		expect(order.filter((id) => !DEFAULT_STATUS_ITEMS.includes(id))).toEqual([
			'stream-health',
			'activity',
			'todos',
			'subagents',
		]);
	});

	test('saved visibility and order preserve explicit customization', () => {
		const items = [
			'subagents',
			'activity',
			'stream-health',
			'todos',
			'context',
			'git',
			'tokens',
			'cost',
		];
		expect(resolveStripOrder({ items, hidden: ['cost', 'tokens'] })).toEqual(
			items.slice(0, -2)
		);
		expect(fullStripOrder({ items, hidden: ['cost', 'tokens'] })).toEqual(
			items
		);
	});

	test('a stored order is honoured', () => {
		const stored = ['context', 'tokens', 'git', 'activity', 'stream-health'];
		expect(resolveStripOrder({ items: stored })).toEqual(withDefaults(stored));
	});

	test('hidden ids drop out, the rest keep their order', () => {
		// A partial list keeps what it names and appends the ids it forgot, in
		// registry order — a module added after the layout was written still shows.
		expect(
			resolveStripOrder({ items: ['context', 'git'], hidden: ['git'] })
		).toEqual(withDefaults(['context']).filter((id) => id !== 'git'));
	});

	test('an unknown id is skipped, not rendered as a gap', () => {
		expect(resolveStripOrder({ items: ['nope', 'tokens'] })).toEqual(
			withDefaults(['tokens'])
		);
	});

	test('a layout of nothing but unknown ids falls back to the built-in order', () => {
		expect(resolveStripOrder({ items: ['nope'] })).toEqual([
			...DEFAULT_STATUS_ITEMS,
		]);
	});

	test('hiding one named module keeps the omitted default-on modules', () => {
		expect(
			resolveStripOrder({ items: ['git'], hidden: ['git'] }).length
		).toBeGreaterThan(0);
	});
});
