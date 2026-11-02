// The trace viewer's first page is a fetch: while it is in flight the list shows
// placeholder rows shaped like the real ones, with the words kept for a screen
// reader — not a lone sentence.

import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import { renderWithProviders } from '@/test/render';

const state = { loading: true, page: null as unknown };

vi.mock('../use-raw-history', () => ({
	useRawHistory: () => ({
		...state,
		error: null,
		open: true,
		retry: vi.fn(),
		latest: vi.fn(),
		older: vi.fn(),
		newer: vi.fn(),
		loadPage: vi.fn(),
		atStart: true,
		atEnd: true,
	}),
	readRawHistoryJson: vi.fn(),
	RAW_PAGE_SIZE: 40,
}));

import { RawHistoryDialog } from '../index';

describe('execution trace loading', () => {
	beforeEach(() => {
		state.loading = true;
		state.page = null;
	});

	test('a skeleton stands in for the rows, with an accessible status', () => {
		renderWithProviders(
			<RawHistoryDialog
				open
				onOpenChange={() => {}}
				sessionId={1}
				workspaceRoot="/w"
			/>
		);
		const status = screen.getByRole('status');
		expect(status).toBeTruthy();
		expect(status.textContent).toContain('正在读取执行轨迹');
		// The placeholders are decoration; the status carries the meaning.
		const hidden = status.querySelector('[aria-hidden="true"]');
		expect(hidden).toBeTruthy();
		expect(hidden?.querySelectorAll('div.flex.min-h-9').length).toBe(8);
		// The old one-line-only state is gone.
		expect(screen.queryByText('正在读取…')).toBeNull();
	});

	test('the placeholders are replaced once the page lands', () => {
		state.loading = false;
		state.page = {
			history: [{ role: 'user', content: 'hello' }],
			history_start: 0,
			history_end: 1,
			history_total: 1,
			history_anchor_len: 1,
			history_anchor: 'a',
		};
		renderWithProviders(
			<RawHistoryDialog
				open
				onOpenChange={() => {}}
				sessionId={1}
				workspaceRoot="/w"
			/>
		);
		expect(screen.queryByRole('status')).toBeNull();
		expect(screen.queryByText('正在读取…')).toBeNull();
	});
});
