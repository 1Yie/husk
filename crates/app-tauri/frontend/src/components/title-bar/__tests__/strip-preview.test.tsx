// The preview editor: the real chips are draggable, the tray switches modules
// back on, and Esc forgets the edit instead of half-keeping it.

import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';
import { StripPreview } from '../strip-preview';

const ORDER = [
	'git',
	'activity',
	'tokens',
	'context',
	'todos',
	'subagents',
	'cost',
];
const nodes = Object.fromEntries(
	ORDER.map((id) => [id, <span key={id}>{id}-chip</span>])
);

function pointer(element: Element, type: string, x = 10, y = 10) {
	const event = new MouseEvent(type, {
		bubbles: true,
		button: 0,
		clientX: x,
		clientY: y,
	});
	Object.defineProperties(event, {
		pointerId: { value: 1 },
		isPrimary: { value: true },
		pointerType: { value: 'mouse' },
	});
	fireEvent(element, event);
}

function dragTo(source: Element, target: Element | null) {
	const editor = document.querySelector<HTMLElement>('[data-strip-editor]')!;
	Object.defineProperty(document, 'elementFromPoint', {
		configurable: true,
		value: () => target,
	});
	pointer(source, 'pointerdown');
	pointer(editor, 'pointermove', 30, 30);
	return editor;
}

function renderPreview(
	over: Partial<React.ComponentProps<typeof StripPreview>> = {}
) {
	const props = {
		order: ORDER,
		hidden: new Set<string>(['cost']),
		nodes,
		onCommit: vi.fn(),
		onToggle: vi.fn(),
		onReset: vi.fn(),
		renderBar: (row: React.ReactNode) => <header>{row}</header>,
		...over,
	};
	const view = render(<StripPreview {...props} />);
	Object.defineProperty(document, 'elementFromPoint', {
		configurable: true,
		value: () => document.activeElement,
	});
	return { ...props, ...view };
}

describe('StripPreview', () => {
	test('places supplied live chips inside the embedded title bar', () => {
		const { container } = renderPreview({
			renderBar: (row) => <header data-preview-bar>{row}</header>,
		});
		expect(
			container.querySelector('[data-status-preview-window] [data-preview-bar]')
		).toBeTruthy();
		expect(screen.getByText('git-chip')).toBeTruthy();
		expect(screen.queryByText('改动')).toBeNull();
	});

	test('draws the real chips and keeps hidden modules in the tray', () => {
		renderPreview();
		expect(screen.getByText('git-chip')).toBeTruthy();
		expect(screen.getByRole('button', { name: '本轮成本' })).toBeTruthy();
		expect(
			screen.getByText('cost-chip').closest('[data-strip-zone=hidden]')
		).toBeTruthy();
	});

	test('× on a chip switches it off, a tray click switches it back on', async () => {
		const user = userEvent.setup();
		const { onToggle } = renderPreview();
		await user.click(screen.getByRole('button', { name: '隐藏Git 分支' }));
		expect(onToggle).toHaveBeenCalledWith('git', false);
		await user.click(screen.getByRole('button', { name: '本轮成本' }));
		expect(onToggle).toHaveBeenCalledWith('cost', true);
	});

	test('Delete removes the focused chip', () => {
		const { onToggle } = renderPreview();
		const chip = screen
			.getByText('git-chip')
			.closest('[data-strip-item]') as HTMLElement;
		fireEvent.keyDown(chip, { key: 'Delete' });
		expect(onToggle).toHaveBeenCalledWith('git', false);
	});

	test('Alt+→ commits the moved order once', () => {
		const onCommit = vi.fn<(order: string[]) => void>();
		renderPreview({ onCommit });
		const chip = screen
			.getByText('git-chip')
			.closest('[data-strip-item]') as HTMLElement;
		fireEvent.keyDown(chip, { key: 'ArrowRight', altKey: true });
		expect(onCommit).toHaveBeenCalledTimes(1);
		expect(onCommit.mock.calls[0][0].slice(0, 2)).toEqual(['activity', 'git']);
	});

	test('keeps pointer drop targets stable and commits the new order once on release', () => {
		const onCommit = vi.fn<(order: string[], shownId?: string) => void>();
		const { container } = renderPreview({ onCommit });
		const editor = dragTo(
			screen.getByText('git-chip').closest('[data-strip-item]')!,
			screen.getByText('tokens-chip')
		);
		expect(
			container
				.querySelector('[data-strip-zone=shown] [data-strip-item]')
				?.getAttribute('aria-label')
		).toBe('Git 分支');
		expect(onCommit).not.toHaveBeenCalled();
		pointer(editor, 'pointerup', 30, 30);
		expect(onCommit).toHaveBeenCalledTimes(1);
		expect(onCommit.mock.calls[0][0].slice(0, 3)).toEqual([
			'activity',
			'tokens',
			'git',
		]);
	});

	test('equivalent parent layout updates keep an in-flight drag alive until release', () => {
		const view = renderPreview();
		const editor = dragTo(
			screen.getByText('git-chip').closest('[data-strip-item]')!,
			screen.getByText('tokens-chip')
		);
		expect(document.querySelector('[data-strip-drag-ghost]')).not.toBeNull();

		for (let i = 0; i < 3; i++) {
			view.rerender(
				<StripPreview
					order={[...ORDER]}
					hidden={new Set(['cost'])}
					nodes={nodes}
					onCommit={view.onCommit}
					onToggle={view.onToggle}
					onReset={view.onReset}
					renderBar={view.renderBar}
				/>
			);
			expect(document.querySelector('[data-strip-drag-ghost]')).not.toBeNull();
			expect(view.onCommit).not.toHaveBeenCalled();
			expect(view.onToggle).not.toHaveBeenCalled();
			pointer(editor, 'pointermove', 40 + i * 10, 30);
		}

		pointer(editor, 'pointerup', 70, 30);
		expect(view.onCommit).toHaveBeenCalledExactlyOnceWith([
			'activity',
			'tokens',
			'git',
			'context',
			'todos',
			'subagents',
			'cost',
		]);
		expect(document.querySelector('[data-strip-drag-ghost]')).toBeNull();
	});

	test.each(['order', 'visibility'])(
		'a real %s update cancels a stale in-flight gesture',
		(change) => {
			const view = renderPreview();
			dragTo(
				screen.getByText('git-chip').closest('[data-strip-item]')!,
				screen.getByText('tokens-chip')
			);
			expect(document.querySelector('[data-strip-drag-ghost]')).not.toBeNull();
			view.rerender(
				<StripPreview
					order={
						change === 'order'
							? ['activity', 'git', ...ORDER.slice(2)]
							: [...ORDER]
					}
					hidden={
						new Set(change === 'visibility' ? ['cost', 'activity'] : ['cost'])
					}
					nodes={nodes}
					onCommit={view.onCommit}
					onToggle={view.onToggle}
					onReset={view.onReset}
					renderBar={view.renderBar}
				/>
			);
			expect(document.querySelector('[data-strip-drag-ghost]')).toBeNull();
			expect(view.onCommit).not.toHaveBeenCalled();
			expect(view.onToggle).not.toHaveBeenCalled();
		}
	);

	test.each(['pointercancel', 'lostpointercapture', 'Escape'])(
		'a %s cancels dragging and removes the drag ghost',
		(end) => {
			const { onCommit, container } = renderPreview();
			const editor = dragTo(
				screen.getByText('git-chip').closest('[data-strip-item]')!,
				screen.getByText('tokens-chip')
			);
			expect(document.querySelector('[data-strip-drag-ghost]')).toBeTruthy();
			if (end === 'Escape') fireEvent.keyDown(window, { key: end });
			else pointer(editor, end);
			expect(document.querySelector('[data-strip-drag-ghost]')).toBeNull();
			expect(
				container
					.querySelector('[data-strip-zone=shown] [data-strip-item]')
					?.getAttribute('aria-label')
			).toBe('Git 分支');
			expect(onCommit).not.toHaveBeenCalled();
		}
	);

	test('dropping a hidden module commits its position and visibility together', () => {
		const { onCommit, onToggle } = renderPreview();
		const editor = dragTo(
			screen.getByRole('button', { name: '本轮成本' }),
			screen.getByText('git-chip')
		);
		pointer(editor, 'pointerup', 30, 30);
		expect(onCommit).toHaveBeenCalledTimes(1);
		expect(onCommit).toHaveBeenCalledWith(
			['cost', ...ORDER.filter((id) => id !== 'cost')],
			'cost'
		);
		expect(onToggle).not.toHaveBeenCalled();
	});

	test('a pointer drop in the hidden strip switches the module off once', () => {
		const { onCommit, onToggle } = renderPreview();
		const editor = dragTo(
			screen.getByText('git-chip').closest('[data-strip-item]')!,
			document.querySelector('[data-strip-zone=hidden]')
		);
		pointer(editor, 'pointerup', 30, 30);
		expect(onToggle).toHaveBeenCalledExactlyOnceWith('git', false);
		expect(onCommit).not.toHaveBeenCalled();
	});

	test('the unoccupied title-bar area accepts a hidden module in one commit', () => {
		const { container, onCommit, onToggle } = renderPreview();
		const header = container.querySelector(
			'[data-status-preview-window]'
		)!.firstElementChild!;
		const editor = dragTo(
			screen.getByRole('button', { name: '本轮成本' }),
			header
		);
		pointer(editor, 'pointerup', 30, 30);
		expect(onCommit).toHaveBeenCalledExactlyOnceWith(ORDER, 'cost');
		expect(onToggle).not.toHaveBeenCalled();
	});

	test('lifting an item into the preview content hides it once', () => {
		const { container, onCommit, onToggle } = renderPreview();
		const body = container.querySelector(
			'[data-status-preview-window] > [aria-hidden]'
		)!;
		const editor = dragTo(
			screen.getByText('git-chip').closest('[data-strip-item]')!,
			body
		);
		pointer(editor, 'pointerup', 30, 30);
		expect(onToggle).toHaveBeenCalledExactlyOnceWith('git', false);
		expect(onCommit).not.toHaveBeenCalled();
	});

	test('cancelling a hidden chip drag does not turn the release click into a restore', () => {
		const { onCommit, onToggle } = renderPreview();
		const chip = screen.getByRole('button', { name: '本轮成本' });
		dragTo(chip, screen.getByText('git-chip'));
		fireEvent.keyDown(window, { key: 'Escape' });
		fireEvent.click(chip, { detail: 1 });
		expect(onToggle).not.toHaveBeenCalled();
		expect(onCommit).not.toHaveBeenCalled();
	});

	test('restore-defaults asks the bar, it does not write by itself', async () => {
		const user = userEvent.setup();
		const { onReset } = renderPreview();
		await user.click(screen.getByRole('button', { name: '恢复默认' }));
		expect(onReset).toHaveBeenCalled();
	});
});
