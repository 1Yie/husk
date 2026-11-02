// The checklist card: three states, hierarchical labels (`2-1` for a subtask),
// and the progress note the agent writes beside a running task.

import { render, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';
import { parseTodos, TodoView } from '../index';

const BLOCK = `2/4 done
[ ] #1 extract the parser
  [~] #1-1 move tests — reading parser_tests.rs
  [x] #1-2 drop the old path
[x] #2 update the guide`;

describe('todo parsing', () => {
	test('the three marks, the labels and the subtask indent are read', () => {
		const { items, doneCount, totalCount } = parseTodos(BLOCK);
		expect(items.map((i) => i.status)).toEqual([
			'pending',
			'in_progress',
			'completed',
			'completed',
		]);
		expect(items.map((i) => i.label)).toEqual(['1', '1-1', '1-2', '2']);
		expect(items.map((i) => i.id)).toEqual([1, 1, 1, 2]);
		expect(items.map((i) => i.depth)).toEqual([0, 1, 1, 0]);
		expect(doneCount).toBe(2);
		expect(totalCount).toBe(4);
	});

	test('only the last ` — ` starts the progress note', () => {
		const [root, running] = parseTodos(
			`${'0/2 done'}\n[ ] #1 parse — the old switch\n  [~] #1-1 rewrite — now on the second arm`
		).items;
		expect(root.text).toBe('parse — the old switch');
		expect(root.note).toBeUndefined();
		expect(running.text).toBe('rewrite');
		expect(running.note).toBe('now on the second arm');
	});

	test('an unparseable body keeps its header and yields no rows', () => {
		const { items, headerMessage } = parseTodos('Cleared 3 todos.');
		expect(items).toEqual([]);
		expect(headerMessage).toBe('Cleared 3 todos.');
	});
});

describe('todo card', () => {
	test('a subtask prints its own label and sits one step in', () => {
		render(<TodoView content={BLOCK} />);
		expect(screen.getByText('#1-1')).toBeTruthy();
		expect(screen.getByText('#2')).toBeTruthy();
		const nested = screen.getByText('move tests').closest('div.group');
		expect(nested?.className).toContain('ml-4');
		const root = screen.getByText('extract the parser').closest('div.group');
		expect(root?.className).not.toContain('ml-4');
	});

	test('a running task shows its progress note under the title', () => {
		render(<TodoView content={BLOCK} />);
		expect(screen.getByText('reading parser_tests.rs')).toBeTruthy();
		const row = screen.getByText('move tests').closest('div.group');
		expect(row?.querySelector('.bg-amber-500')).not.toBeNull();
		const done = screen.getByText('drop the old path').closest('div.group');
		expect(done?.querySelector('.line-through')).not.toBeNull();
	});

	test('the header reports completed over total', () => {
		render(<TodoView content={BLOCK} />);
		expect(screen.getByText('2/4 (50%)')).toBeTruthy();
	});
});
