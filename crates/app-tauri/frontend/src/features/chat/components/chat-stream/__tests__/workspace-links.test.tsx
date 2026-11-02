import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';
import { MemoStreamdown } from '../markdown-stream';

describe('workspace links in chat Markdown', () => {
	test('opens new-root file URLs after switching the real Markdown processor context', () => {
		const open = vi.fn();
		const { rerender } = render(
			<MemoStreamdown
				text="[source](file:///first/src/a.ts#L12C3)"
				workspaceRoot="/first"
				onOpenWorkspaceFile={open}
			/>
		);
		fireEvent.click(screen.getByRole('link', { name: 'source' }));
		expect(open).toHaveBeenLastCalledWith({
			path: 'src/a.ts',
			line: 12,
			column: 3,
		});
		rerender(
			<MemoStreamdown
				text="[source](file:///second/src/a.ts#L4C2)"
				workspaceRoot="/second"
				onOpenWorkspaceFile={open}
			/>
		);
		fireEvent.click(screen.getByRole('link', { name: 'source' }));
		expect(open).toHaveBeenLastCalledWith({
			path: 'src/a.ts',
			line: 4,
			column: 2,
		});
		expect(open).toHaveBeenCalledTimes(2);
	});

	test('routes file URLs and colon positions before hardening while retaining external safety', () => {
		const open = vi.fn();
		render(
			<MemoStreamdown
				text={
					'[source](file:///repo/src/a.ts#L12C3) [relative](src/a.ts:4:2) [web](https://example.com) [unsafe](javascript:alert%281%29)'
				}
				workspaceRoot="/repo"
				onOpenWorkspaceFile={open}
			/>
		);
		fireEvent.click(screen.getByRole('link', { name: 'source' }));
		expect(open).toHaveBeenLastCalledWith({
			path: 'src/a.ts',
			line: 12,
			column: 3,
		});
		fireEvent.click(screen.getByRole('link', { name: 'relative' }));
		expect(open).toHaveBeenLastCalledWith({
			path: 'src/a.ts',
			line: 4,
			column: 2,
		});
		expect(screen.getByRole('link', { name: 'web' })).toHaveAttribute(
			'href',
			'https://example.com/'
		);
		expect(screen.queryByRole('link', { name: 'unsafe' })).toBeNull();
	});

	test('keeps the default policy without a workspace callback', () => {
		render(<MemoStreamdown text="[source](file:///repo/src/a.ts#L12)" />);
		expect(screen.queryByRole('link', { name: 'source' })).toBeNull();
		expect(screen.getByText(/source \[blocked\]/)).toBeInTheDocument();
	});
});
