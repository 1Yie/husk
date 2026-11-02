import { describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/appearance', () => ({
	useCurrencySymbol: () => '$',
	APPEARANCE_CHANGED: 'appearance://changed',
}));
vi.mock('@/components/window-controls', () => ({ WindowControls: () => null }));
vi.mock('@/features/chat/components/stream-health', () => ({
	StreamHealth: () => null,
}));
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from '@testing-library/react';
import { TooltipProvider } from '@/components/ui/tooltip';
import { TitleBar } from '../index';
import { emptyView } from '@/features/chat/hooks/stream-view';
import { mockCommand } from '@/test/tauri';
import { DEFAULT_STATUS_ITEMS } from '../strip';

function viewWith() {
	const v = emptyView();
	v.usage = {
		prompt: 100_000,
		completion: 2_000,
		contextWindow: 200_000,
		cachedTokens: 90_000,
		cacheWriteTokens: 0,
	};
	v.toksPerSec = 42;
	v.items = [
		{ kind: 'user', text: 'hi' },
		{ kind: 'tool', name: 'read', args: '{}', ok: true },
		{ kind: 'tool', name: 'bash', args: '{}', ok: false },
		{ kind: 'assistant', text: 'done', streaming: false },
	];
	return v;
}

function renderBar(
	view = viewWith(),
	gitInfo: { repo: boolean; branch: string | null; dirty: number } | null = {
		repo: true,
		branch: 'main',
		dirty: 2,
	},
	extra: { onCompact?: () => void } = {}
) {
	return render(
		<TooltipProvider>
			<TitleBar
				title="t"
				view={view}
				contextWindowHint={200_000}
				gitInfo={gitInfo}
				{...extra}
			/>
		</TooltipProvider>
	);
}

function enableModules(...ids: string[]) {
	mockCommand(
		'agent_session',
		() => ({
			status_strip: { items: [...ids, ...DEFAULT_STATUS_ITEMS], hidden: [] },
		}),
		'get_appearance'
	);
}

describe('TitleBar stat chips', () => {
	it('shows only the default four when a model price is available', () => {
		const { container } = render(
			<TooltipProvider>
				<TitleBar
					view={viewWith()}
					gitInfo={{ repo: true, branch: 'main', dirty: 2 }}
					modelCost={{ input: 0, output: 0 }}
				/>
			</TooltipProvider>
		);
		const labels = [
			...container.querySelectorAll('button[aria-haspopup=dialog]'),
		].map((button) => button.getAttribute('aria-label')?.split(' ')[0]);
		expect(labels).toEqual(['Git', 'Token', '上下文窗口', '本轮成本']);
		expect(screen.queryByRole('button', { name: '生成活动' })).toBeNull();
		expect(
			screen.queryByRole('button', { name: /任务清单|子代理/ })
		).toBeNull();
	});

	it('summarises activity, tokens and context on the bar', async () => {
		enableModules('activity');
		renderBar();
		expect(
			(await screen.findByRole('button', { name: '生成活动' })).textContent
		).toContain('1 步 · 2 调用');
		const tokens = screen.getByRole('button', {
			name: 'Token 用量',
		}).textContent;
		expect(tokens).toContain('102K tok');
		expect(tokens).toContain('缓存 90%');
		expect(
			screen.getByRole('button', { name: '上下文窗口' }).textContent
		).toContain('50%');
	});

	it('opens the detail card on click', async () => {
		renderBar();
		fireEvent.click(screen.getByRole('button', { name: 'Token 用量' }));
		expect(await screen.findByText('未缓存输入')).toBeTruthy();
		expect(screen.getByText('90,000 tok')).toBeTruthy();
	});

	it('keeps a sub-100% cache rate from rounding up to 100%', () => {
		const v = viewWith();
		v.usage = { ...v.usage, prompt: 100_000, cachedTokens: 99_999 };
		renderBar(v);
		expect(
			screen.getByRole('button', { name: 'Token 用量' }).textContent
		).toContain('缓存 99.99%');
		fireEvent.click(screen.getByRole('button', { name: 'Token 用量' }));
		expect(screen.getByText('99.99%')).toBeTruthy();
	});

	it('lists failed tool calls in the activity card', async () => {
		enableModules('activity');
		renderBar();
		fireEvent.click(await screen.findByRole('button', { name: '生成活动' }));
		expect(await screen.findByText('工具失败')).toBeTruthy();
	});
});

describe('Git chip', () => {
	it('keeps the branch up while the tree is clean', async () => {
		renderBar(viewWith(), { repo: true, branch: 'main', dirty: 0 });
		const chip = screen.getByRole('button', { name: 'Git 分支 main' });
		expect(chip.textContent).toContain('main');
		// The cleanliness is detail, not a display gate.
		fireEvent.click(chip);
		expect(await screen.findByText('未提交修改')).toBeTruthy();
		expect(screen.getByText('无')).toBeTruthy();
	});

	it('stays off outside a repository', () => {
		renderBar(viewWith(), { repo: false, branch: null, dirty: 0 });
		expect(screen.queryByRole('button', { name: /Git 分支/ })).toBeNull();
	});
});

/** jsdom's PointerEvent drops `pointerType`; React's enter/leave derive
 * from over/out, so dispatch those with the field set by hand. */
function mousePointer(type: 'pointerover' | 'pointerout') {
	const e = new MouseEvent(type, {
		bubbles: true,
		relatedTarget: document.body,
	});
	Object.defineProperty(e, 'pointerType', { value: 'mouse' });
	return e;
}

describe('context card', () => {
	it('scales the estimated breakdown to the reported prompt', async () => {
		const v = viewWith();
		v.contextBreakdown = {
			system: 10,
			tools: 10,
			user: 5,
			memory: 0,
			assistant: 5,
			reasoning: 10,
			tool_calls: 10,
			tool_results: 50,
			other: 0,
		};
		renderBar(v);
		fireEvent.click(screen.getByRole('button', { name: '上下文窗口' }));
		expect(await screen.findByText('工具结果')).toBeTruthy();
		// 50% of 100K reported prompt tokens.
		expect(screen.getByText('~50K')).toBeTruthy();
		expect(screen.getByText('思考过程')).toBeTruthy();
		expect(screen.queryByText('记忆召回')).toBeNull();
	});

	it('falls back to the cache split without a breakdown', async () => {
		renderBar();
		fireEvent.click(screen.getByRole('button', { name: '上下文窗口' }));
		expect(await screen.findByText('缓存前缀')).toBeTruthy();
	});

	it('offers a compact button that calls onCompact', async () => {
		const onCompact = vi.fn();
		renderBar(viewWith(), undefined, { onCompact });
		fireEvent.click(screen.getByRole('button', { name: '上下文窗口' }));
		fireEvent.click(await screen.findByRole('button', { name: '压缩上下文' }));
		expect(onCompact).toHaveBeenCalledTimes(1);
	});

	it('disables compact while a turn is streaming', async () => {
		const v = viewWith();
		v.streaming = true;
		renderBar(v, undefined, { onCompact: vi.fn() });
		fireEvent.click(screen.getByRole('button', { name: '上下文窗口' }));
		const button = (await screen.findByRole('button', {
			name: '压缩上下文',
		})) as HTMLButtonElement;
		expect(button.disabled).toBe(true);
	});

	it('hides the compact button without an onCompact handler', async () => {
		renderBar();
		fireEvent.click(screen.getByRole('button', { name: '上下文窗口' }));
		expect(await screen.findByText('缓存前缀')).toBeTruthy();
		expect(screen.queryByRole('button', { name: '压缩上下文' })).toBeNull();
	});
});

describe('StatPopover focus', () => {
	it('replaces a pinned card when another status chip is hovered', async () => {
		enableModules('activity');
		renderBar();
		fireEvent.click(screen.getByRole('button', { name: 'Git 分支 main' }));
		expect(await screen.findByText('未提交修改')).toBeTruthy();
		fireEvent(
			await screen.findByRole('button', { name: '生成活动' }),
			mousePointer('pointerover')
		);
		expect(await screen.findByText('用户轮次')).toBeTruthy();
		await waitFor(() => expect(screen.getAllByRole('dialog')).toHaveLength(1));
		expect(screen.queryByText('未提交修改')).toBeNull();
	});

	it('does not ring the chip when a hover card closes', async () => {
		renderBar();
		const chip = screen.getByRole('button', { name: 'Token 用量' });
		fireEvent(chip, mousePointer('pointerover'));
		await screen.findByText('未缓存输入');
		fireEvent(chip, mousePointer('pointerout'));
		await waitFor(() => expect(screen.queryByText('未缓存输入')).toBeNull());
		expect(document.activeElement).not.toBe(chip);
	});

	it('returns focus on Esc after a hover card is pinned by clicking its trigger', async () => {
		renderBar();
		const chip = screen.getByRole('button', { name: 'Token 用量' });
		fireEvent(chip, mousePointer('pointerover'));
		await screen.findByText('未缓存输入');
		fireEvent.pointerDown(chip, { button: 0 });
		fireEvent.click(chip);
		fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
		await waitFor(() => expect(screen.queryByText('未缓存输入')).toBeNull());
		await waitFor(() => expect(document.activeElement).toBe(chip));
	});
});

describe('TitleBar status strip order', () => {
	it('updates mounted chips when a saved appearance layout changes', async () => {
		const { emitTauriEvent, listenerCount, mockCommand } =
			await import('@/test/tauri');
		let config = {
			status_strip: {
				items: ['context', 'git', 'activity', 'tokens'],
				hidden: [] as string[],
			},
		};
		mockCommand('agent_session', () => config, 'get_appearance');
		const { container, unmount } = renderBar();
		const labels = () =>
			[...container.querySelectorAll('button[aria-label]')]
				.map((button) => button.getAttribute('aria-label') ?? '')
				.filter((label) =>
					/^(上下文窗口|Git 分支|生成活动|Token 用量)/.test(label)
				);
		await waitFor(() => expect(labels()[0]).toBe('上下文窗口'));
		config = {
			status_strip: {
				items: ['tokens', 'activity', 'context', 'git'],
				hidden: ['git', 'activity'],
			},
		};
		await act(async () => {
			emitTauriEvent('appearance://changed', {});
		});
		await waitFor(() =>
			expect(screen.queryByRole('button', { name: 'Git 分支 main' })).toBeNull()
		);
		expect(screen.queryByRole('button', { name: '生成活动' })).toBeNull();
		expect(labels()).toEqual(['Token 用量', '上下文窗口']);

		config = { status_strip: { ...config.status_strip, hidden: [] } };
		await act(async () => {
			emitTauriEvent('appearance://changed', {});
		});
		expect(
			await screen.findByRole('button', { name: 'Git 分支 main' })
		).toBeTruthy();
		expect(labels()[0]).toBe('Token 用量');
		await act(async () => {
			unmount();
		});
		expect(listenerCount('appearance://changed')).toBe(0);
	});

	it('respects an explicitly empty strip instead of restoring default chips', async () => {
		const { STATUS_MODULES } = await import('../strip');
		const { mockCommand } = await import('@/test/tauri');
		const ids = STATUS_MODULES.map((module) => module.id);
		mockCommand(
			'agent_session',
			() => ({
				status_strip: { items: ids, hidden: ids },
			}),
			'get_appearance'
		);
		renderBar();
		await waitFor(() =>
			expect(screen.queryByRole('button', { name: 'Git 分支 main' })).toBeNull()
		);
		expect(screen.queryByRole('button', { name: 'Token 用量' })).toBeNull();
		expect(screen.queryByRole('button', { name: '上下文窗口' })).toBeNull();
	});

	it('keeps a newer saved layout when the initial read resolves late', async () => {
		const { emitTauriEvent, listenerCount, mockCommand } =
			await import('@/test/tauri');
		const oldConfig = {
			status_strip: { items: ['git', 'tokens'], hidden: [] as string[] },
		};
		const newConfig = {
			status_strip: { items: ['tokens', 'git'], hidden: ['git'] },
		};
		let finishInitial!: (value: typeof oldConfig) => void;
		const initial = new Promise<typeof oldConfig>((resolve) => {
			finishInitial = resolve;
		});
		let calls = 0;
		mockCommand(
			'agent_session',
			() => (calls++ === 0 ? initial : newConfig),
			'get_appearance'
		);
		renderBar();
		await waitFor(() => expect(listenerCount('appearance://changed')).toBe(1));
		await act(async () => {
			emitTauriEvent('appearance://changed', {});
		});
		await waitFor(() =>
			expect(screen.queryByRole('button', { name: 'Git 分支 main' })).toBeNull()
		);
		await act(async () => {
			finishInitial(oldConfig);
		});
		expect(screen.queryByRole('button', { name: 'Git 分支 main' })).toBeNull();
	});

	it('renders the stored order and drops a hidden module', async () => {
		const { mockCommand } = await import('@/test/tauri');
		mockCommand(
			'agent_session',
			() => ({
				theme_mode: 'system',
				accent: '#339CFF',
				background: '#FFFFFF',
				foreground: '#1A1C1F',
				ui_font: 'system-ui',
				code_font: 'monospace',
				contrast: 45,
				status_strip: {
					items: ['context', 'git', 'activity', 'tokens', 'stream-health'],
					hidden: ['tokens'],
				},
			}),
			'get_appearance'
		);
		renderBar();
		await waitFor(() => {
			const chips = [...document.querySelectorAll('button[aria-label]')]
				.map((b) => b.getAttribute('aria-label') ?? '')
				.filter((l) => /^(上下文窗口|Git 分支|生成活动|Token 用量)/.test(l));
			expect(chips[0]).toMatch(/^上下文窗口/);
			expect(chips[1]).toMatch(/^Git 分支/);
			expect(chips.some((l) => l.startsWith('Token 用量'))).toBe(false);
		});
	});
});

describe('TitleBar new status modules', () => {
	it('shows the task list only when the session has one', async () => {
		enableModules('todos');
		const withTodos = viewWith();
		withTodos.items = [
			...withTodos.items,
			{
				kind: 'tool',
				name: 'todo',
				args: '{}',
				ok: true,
				content: '1/2 done\n[ ] #1 a\n[x] #2 b',
			},
		];
		const { unmount } = renderBar(withTodos);
		expect(
			await screen.findByRole('button', { name: /任务清单 1\/2/ })
		).toBeTruthy();
		unmount();
		// No list in the view → no chip at all, not an empty one.
		await act(async () => {
			renderBar(viewWith());
		});
		expect(screen.queryByRole('button', { name: /任务清单/ })).toBeNull();
	});

	it('counts the running subagents, and only while one is out', async () => {
		enableModules('subagents');
		const withChild = viewWith();
		withChild.items = [
			...withChild.items,
			{
				kind: 'tool',
				name: 'subagent',
				args: '检查鉴权',
				parent: 'delegate',
				childId: 7,
			},
		];
		const { unmount } = renderBar(withChild);
		expect(
			await screen.findByRole('button', { name: /子代理 1 个/ })
		).toBeTruthy();
		unmount();
		await act(async () => {
			renderBar(viewWith());
		});
		expect(screen.queryByRole('button', { name: /子代理/ })).toBeNull();
	});

	it('keeps the cost chip hidden when the model declares no price', () => {
		const { unmount } = renderBar(viewWith());
		expect(screen.queryByRole('button', { name: /本轮成本/ })).toBeNull();
		unmount();
		renderBar(viewWith(), { repo: true, branch: 'main', dirty: 0 }, {});
		// `modelCost` absent is the same as a model without a cost block.
		expect(screen.queryByRole('button', { name: /本轮成本/ })).toBeNull();
	});
});

describe('TitleBar settings preview', () => {
	it('keeps customization in settings rather than on the workspace status bar', () => {
		renderBar();
		expect(screen.queryByRole('button', { name: '自定义状态栏' })).toBeNull();
		fireEvent.contextMenu(
			screen.getByRole('button', { name: 'Git 分支 main' })
		);
		expect(screen.queryByRole('dialog', { name: '自定义状态栏' })).toBeNull();
		expect(screen.queryByText('恢复默认')).toBeNull();
	});

	it('embeds the real stat chips in an editable window without opening their cards', () => {
		const onCommit = vi.fn();
		const onToggle = vi.fn();
		const { container } = render(
			<TooltipProvider>
				<TitleBar
					title="Husk"
					view={viewWith()}
					contextWindowHint={200_000}
					gitInfo={{ repo: true, branch: 'main', dirty: 2 }}
					preview={{
						order: ['git', 'activity', 'tokens', 'context'],
						hidden: new Set(),
						onCommit,
						onToggle,
						onReset: vi.fn(),
					}}
				/>
			</TooltipProvider>
		);
		expect(
			container.querySelector('[data-status-preview-desktop]')
		).toBeTruthy();
		expect(
			container.querySelector('[data-status-preview-window]')
		).toBeTruthy();
		expect(screen.queryByText('Husk')).toBeNull();
		expect(screen.getByText('102K tok')).toBeTruthy();
		const chip = screen.getByRole('group', { name: 'Git 分支' });
		fireEvent.keyDown(chip, { key: 'ArrowRight', altKey: true });
		expect(onCommit.mock.calls[0][0]).toEqual([
			'activity',
			'git',
			'tokens',
			'context',
		]);
		fireEvent.click(screen.getByRole('button', { name: '隐藏Git 分支' }));
		expect(onToggle).toHaveBeenCalledWith('git', false);
		expect(screen.queryByText('未缓存输入')).toBeNull();
	});
});
