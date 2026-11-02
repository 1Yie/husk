import { fireEvent, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, test, vi } from 'vitest';
import { renderWithProviders } from '@/test/render';
import { MainLayout } from '@/app/layout/main-layout';
import { useLayoutStore } from '@/app/layout/layout-store';
import { useDockStore } from '@/features/chat/components/dock/state';
import { SessionSidebar } from '@/features/chat/components/session-sidebar';
import { LeftSidebarToggle, RightSidebarToggle } from '../layout-controls';
import { isMac } from '@/lib/platform';

vi.mock('@/lib/platform', async (original) => ({
	...(await original<object>()),
	isMac: true,
	framedShell: true,
}));
vi.mock('@tauri-apps/api/core', async () => ({
	invoke: (await import('@/test/tauri')).tauri.invoke,
	convertFileSrc: (path: string) => `asset://localhost/${path}`,
	isTauri: () => false,
}));

beforeEach(() => {
	useLayoutStore.setState({ sidebarOpen: true, terminalOpen: false });
	useDockStore.setState({
		open: false,
		tab: 'changes',
		browserOpen: false,
		filesTab: 'changes',
		width: 300,
	});
});

function shell() {
	return renderWithProviders(
		<MainLayout
			sidebar={<button>原有会话状态</button>}
			terminal={<div>终端保留区</div>}
		>
			<div>
				<LeftSidebarToggle />
				<RightSidebarToggle />
			</div>
			<div>主内容</div>
		</MainLayout>
	);
}

test.each([true, false])(
	'Mac header omits branding with mode controls: %s',
	(withModes) => {
		const { container } = renderWithProviders(
			<SessionSidebar
				projects={[]}
				hasWorkspace={false}
				onNew={() => {}}
				onOpenSession={() => {}}
				onSwitchMode={withModes ? vi.fn() : undefined}
			/>
		);
		const header = container.querySelector('.session-navigation-panel')!
			.firstElementChild as HTMLElement;
		expect(header.querySelector('img')).toBeNull();
		expect(within(header).queryByText('Husk', { exact: true })).toBeNull();
		expect(header.querySelector('.w-\\[55px\\]')).not.toBeNull();
		expect(header).toHaveAttribute('data-tauri-drag-region', 'deep');
		expect(within(header).queryAllByRole('tab')).toHaveLength(
			withModes ? 2 : 0
		);
	}
);

test('the left toggle stays reachable while navigation is collapsed without unmounting it', async () => {
	const user = userEvent.setup();
	shell();
	const main = document.querySelector('.main-col');
	const terminal = screen.getByText('终端保留区');
	const content = screen.getByText('主内容');
	await user.click(screen.getByRole('button', { name: '收起会话侧栏' }));
	const collapsed = screen.getByRole('button', { name: '展开会话侧栏' });
	expect(collapsed).toHaveAttribute('aria-expanded', 'false');
	const navigation = document.getElementById('session-navigation');
	expect(navigation).toHaveAttribute('aria-hidden', 'true');
	expect(navigation).toHaveAttribute('inert');
	expect(navigation).toHaveStyle({ width: '0px' });
	expect(screen.getByText('原有会话状态')).toBeInTheDocument();
	expect(screen.getByText('终端保留区')).toBeInTheDocument();
	expect(document.querySelector('.main-col')).toBe(main);
	expect(screen.getByText('终端保留区')).toBe(terminal);
	expect(screen.getByText('主内容')).toBe(content);
	await user.click(collapsed);
	expect(navigation).toHaveAttribute('aria-hidden', 'false');
	expect(navigation).not.toHaveAttribute('inert');
	expect(document.querySelector('.main-col')).toBe(main);
	expect(screen.getByText('终端保留区')).toBe(terminal);
});

test('on macOS the collapsed toggle clears the traffic lights, easing with the sidebar', async () => {
	vi.resetModules();
	vi.doMock('@/lib/platform', async (original) => ({
		...(await original<object>()),
		isMac: true,
	}));
	const { LeftSidebarToggle: MacToggle } = await import('../layout-controls');
	const { useLayoutStore: store } = await import('@/app/layout/layout-store');
	store.setState({ sidebarOpen: true });
	const view = renderWithProviders(<MacToggle />);
	const lane = screen
		.getByRole('button', { name: '收起会话侧栏' })
		.closest('div.flex-none') as HTMLElement;
	expect(lane).toHaveClass('pl-0', 'transition-[padding]', 'duration-300');
	await userEvent
		.setup()
		.click(screen.getByRole('button', { name: '收起会话侧栏' }));
	expect(lane).toHaveClass('pl-[55px]');
	view.unmount();
	vi.doUnmock('@/lib/platform');
});

test('the right toggle changes visibility without selecting a different tab', async () => {
	const user = userEvent.setup();
	shell();
	await user.click(screen.getByRole('button', { name: '展开工具侧栏' }));
	expect(useDockStore.getState()).toMatchObject({ open: true, tab: 'changes' });
	await user.click(screen.getByRole('button', { name: '收起工具侧栏' }));
	expect(useDockStore.getState().open).toBe(false);
	expect(useDockStore.getState().tab).toBe('changes');
	expect(
		screen.getByRole('button', { name: '展开工具侧栏' })
	).toBeInTheDocument();
});

test('terminal shortcut toggles the panel once and ignores key repeats', () => {
	shell();
	// Bindings use the platform's primary modifier, so the dispatch must too.
	const mod = isMac ? { metaKey: true } : { ctrlKey: true };
	fireEvent.keyDown(window, { key: 'j', ...mod, shiftKey: true });
	expect(useLayoutStore.getState().terminalOpen).toBe(true);
	fireEvent.keyDown(window, { key: 'j', ...mod, shiftKey: true, repeat: true });
	expect(useLayoutStore.getState().terminalOpen).toBe(true);
	fireEvent.keyDown(window, { key: 'j', ...mod, shiftKey: true });
	expect(useLayoutStore.getState().terminalOpen).toBe(false);
});

test('sidebar keyboard widths remain window-local and survive collapse without replacing content', async () => {
	shell();
	const root = document.querySelector('.main-layout') as HTMLElement;
	Object.defineProperty(root, 'clientWidth', { value: 1280 });
	const handle = screen.getByRole('separator', { name: '调整会话侧栏宽度' });
	const main = document.querySelector('.main-col');
	const stored = JSON.stringify(localStorage);
	fireEvent.keyDown(handle, { key: 'ArrowRight' });
	expect(root.style.getPropertyValue('--session-sidebar-width')).toBe('264px');
	expect(handle).toHaveAttribute('aria-valuenow', '264');
	expect(JSON.stringify(localStorage)).toBe(stored);
	await userEvent
		.setup()
		.click(screen.getByRole('button', { name: '收起会话侧栏' }));
	expect(handle).not.toBeVisible();
	const mod = isMac ? { metaKey: true } : { ctrlKey: true };
	fireEvent.keyDown(window, { key: 'j', ...mod, shiftKey: true });
	expect(useLayoutStore.getState().terminalOpen).toBe(true);
	await userEvent
		.setup()
		.click(screen.getByRole('button', { name: '展开会话侧栏' }));
	expect(root.style.getPropertyValue('--session-sidebar-width')).toBe('264px');
	expect(document.querySelector('.main-col')).toBe(main);
	fireEvent.keyDown(handle, { key: 'End' });
	expect(root.style.getPropertyValue('--session-sidebar-width')).toBe('480px');
	fireEvent.keyDown(handle, { key: 'Home' });
	fireEvent.keyDown(handle, { key: 'ArrowLeft' });
	expect(root.style.getPropertyValue('--session-sidebar-width')).toBe('200px');
	expect(handle).toHaveAttribute('aria-valuemin', '200');
});

test('the terminal entry sits apart at the far end of the sidebar footer', async () => {
	const user = userEvent.setup();
	const toggle = vi.fn();
	renderWithProviders(
		<SessionSidebar
			projects={[]}
			version="0.3.11"
			hasWorkspace={false}
			onNew={() => {}}
			onOpenSession={() => {}}
			onToggleTerminal={toggle}
		/>
	);
	const notifications = screen.getByRole('button', { name: '通知' });
	const terminal = screen.getByRole('button', { name: '打开终端' });
	expect(
		notifications.compareDocumentPosition(terminal) &
			Node.DOCUMENT_POSITION_FOLLOWING
	).not.toBe(0);
	// Not in the settings/notifications group: its own group, pushed right.
	expect(terminal.parentElement?.contains(notifications)).toBe(false);
	expect(terminal.closest('.ml-auto')).not.toBeNull();
	await user.click(terminal);
	expect(toggle).toHaveBeenCalledOnce();
});

test('the terminal button reads live state without its parent subscribing', async () => {
	renderWithProviders(
		<SessionSidebar
			projects={[]}
			version="0.3.11"
			hasWorkspace={false}
			onNew={() => {}}
			onOpenSession={() => {}}
			onToggleTerminal={useLayoutStore.getState().toggleTerminal}
		/>
	);
	await userEvent
		.setup()
		.click(screen.getByRole('button', { name: '打开终端' }));
	expect(screen.getByRole('button', { name: '隐藏终端' })).toHaveAttribute(
		'aria-pressed',
		'true'
	);
	await userEvent
		.setup()
		.click(screen.getByRole('button', { name: '隐藏终端' }));
	expect(screen.getByRole('button', { name: '打开终端' })).toHaveAttribute(
		'aria-pressed',
		'false'
	);
});
