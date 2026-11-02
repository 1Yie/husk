// A round's calls are announced before dispatch, and each queued card becomes the
// running one under its own id — never a second card, and never the wrong one when
// two calls share a tool name.

import { describe, expect, test } from 'vitest';
import { applyEvents } from '@/features/chat/hooks/apply-event';
import { emptyView, type StreamItem } from '@/features/chat/hooks/stream-view';
import { toolActivityPreview } from '@/features/chat/components/tool-chips/index';
import type { UiEvent } from '@/types';

const queued = (name: string, callId: string): UiEvent => ({
	ToolCallQueued: {
		name,
		args_preview: `${name} target`,
		call_id: callId,
		parent: null,
		child_id: null,
	},
});

const started = (name: string, callId: string): UiEvent => ({
	ToolCallStarted: {
		name,
		args_preview: `${name} target`,
		call_id: callId,
		parent: null,
		child_id: null,
	},
});

const finished = (name: string, content: string): UiEvent => ({
	ToolCallFinished: { name, ok: true, content, parent: null, child_id: null },
});

const toolItems = (view: ReturnType<typeof emptyView>) =>
	view.items.filter(
		(it): it is Extract<StreamItem, { kind: 'tool' }> => it.kind === 'tool'
	);

describe('a queued round', () => {
	test('every call gets a card before the first one runs', () => {
		const v = applyEvents(emptyView(), [
			{ UserPrompt: 'go' },
			queued('todo', 'call_1'),
			queued('todo', 'call_2'),
			queued('todo', 'call_3'),
		]);
		const rows = toolItems(v);
		expect(rows).toHaveLength(3);
		expect(rows.every((r) => r.queued === true)).toBe(true);
		expect(rows.map((r) => r.callId)).toEqual(['call_1', 'call_2', 'call_3']);
		// The collapsed header says work remains instead of "完成".
		expect(
			toolActivityPreview(
				rows.map((r) => ({
					id: String(r.callId),
					label: r.name,
					chip: r.args ?? '',
					status: 'queued',
				}))
			)
		).toBe('3 个调用排队中');
	});

	test('a start promotes its own card, even when the names repeat', () => {
		const v = applyEvents(emptyView(), [
			{ UserPrompt: 'go' },
			queued('todo', 'call_1'),
			queued('todo', 'call_2'),
			// The second call runs first in this stream on purpose: matching by name
			// would light up the wrong card.
			started('todo', 'call_2'),
		]);
		const rows = toolItems(v);
		expect(rows).toHaveLength(2);
		expect(rows[0].queued).toBe(true);
		expect(rows[0].callId).toBe('call_1');
		expect(rows[1].queued).toBe(false);
		expect(rows[1].callId).toBe('call_2');
		expect(rows[1].startedAt).toBeGreaterThan(0);
	});

	test('each card settles on its own result', () => {
		const v = applyEvents(emptyView(), [
			{ UserPrompt: 'go' },
			queued('todo', 'call_1'),
			started('todo', 'call_1'),
			finished('todo', 'first'),
			queued('todo', 'call_2'),
			started('todo', 'call_2'),
			finished('todo', 'second'),
		]);
		const rows = toolItems(v);
		expect(rows.filter((r) => r.queued)).toHaveLength(0);
		expect(rows.map((r) => r.content)).toEqual(['first', 'second']);
	});

	test('a call the round never dispatched does not stay queued forever', () => {
		// A cancel between dispatches leaves the rest unrun; the turn's own settle
		// pass must close their cards.
		const v = applyEvents(emptyView(), [
			{ UserPrompt: 'go' },
			queued('todo', 'call_1'),
			queued('todo', 'call_2'),
			{ StateChanged: 'Finished' },
		]);
		const rows = toolItems(v);
		expect(rows.every((r) => r.content === '已中断')).toBe(true);
	});
});
