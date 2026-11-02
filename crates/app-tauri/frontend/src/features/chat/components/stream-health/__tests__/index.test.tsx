import { fireEvent, render, screen } from '@testing-library/react';
import { useState, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { getUiStats, onAgentEvent } from '@/lib/agent-ipc';
import { StatPopoverGroup } from '@/components/title-bar/stat-popover';
import { StreamHealth } from '../index';

vi.mock('@/lib/agent-ipc', () => ({
	getUiStats: vi.fn(),
	onAgentEvent: vi.fn(),
}));

function HealthGroup({ children }: { children: ReactNode }) {
	const [active, setActive] = useState<string | null>(null);
	return (
		<StatPopoverGroup value={{ active, setActive }}>
			{children}
		</StatPopoverGroup>
	);
}

beforeEach(() => {
	vi.mocked(getUiStats).mockResolvedValue({
		sent: 24,
		coalesced: 12,
		dropped: 0,
		depth_peak: 4,
	});
	vi.mocked(onAgentEvent).mockResolvedValue(() => {});
});

afterEach(() => vi.unstubAllEnvs());

describe('StreamHealth', () => {
	test('shows an idle state and keeps transport counters in the detail card', async () => {
		render(<StreamHealth />, { wrapper: HealthGroup });
		const chip = screen.getByRole('button', { name: '生成状态 · 就绪' });
		expect(chip.textContent).toBe('就绪');
		expect(screen.queryByText('队列新增')).toBeNull();
		fireEvent.click(chip);
		expect(await screen.findByText('24')).toBeTruthy();
		expect(screen.getByText('队列合并')).toBeTruthy();
		expect(screen.getByText('峰值深度')).toBeTruthy();
		expect(screen.getByText('文本丢失')).toBeTruthy();
	});

	test('shows an active signal and retains its accessible state in compact mode', () => {
		const { rerender, container } = render(<StreamHealth generating />, {
			wrapper: HealthGroup,
		});
		expect(
			screen.getByRole('button', { name: '生成状态 · 生成中' }).textContent
		).toBe('生成中');
		expect(
			container.querySelectorAll('[data-generation-signal] span')
		).toHaveLength(4);
		rerender(<StreamHealth generating compact />);
		expect(
			screen.getByRole('button', { name: '生成状态 · 生成中' }).textContent
		).toBe('');
	});

	test('keeps a text-loss warning visible while the session is running', async () => {
		vi.mocked(getUiStats).mockResolvedValue({
			sent: 2400,
			coalesced: 12,
			dropped: 1200,
			depth_peak: 40,
		});
		render(<StreamHealth generating />, { wrapper: HealthGroup });
		const chip = await screen.findByRole('button', {
			name: '生成状态 · 文本丢失 1.2k',
		});
		fireEvent.click(chip);
		expect(await screen.findByText(/控制事件保持完整/)).toBeTruthy();
		expect(screen.getByText('1200')).toBeTruthy();
	});

	test('retains session state when queue statistics are unavailable', async () => {
		vi.mocked(getUiStats).mockRejectedValue(new Error('offline'));
		render(<StreamHealth generating />, { wrapper: HealthGroup });
		fireEvent.click(screen.getByRole('button', { name: '生成状态 · 生成中' }));
		expect(await screen.findByText('队列统计暂不可用')).toBeTruthy();
	});

	test('renders the preview with no backend reads or event subscriptions', () => {
		render(<StreamHealth preview generating />, { wrapper: HealthGroup });
		expect(
			screen.getByRole('button', { name: '生成状态 · 生成中' })
		).toBeTruthy();
		expect(getUiStats).not.toHaveBeenCalled();
		expect(onAgentEvent).not.toHaveBeenCalled();
	});

	test('shows session state in production without adding an IPC event listener', () => {
		vi.stubEnv('DEV', false);
		render(<StreamHealth generating />, { wrapper: HealthGroup });
		expect(
			screen.getByRole('button', { name: '生成状态 · 生成中' })
		).toBeTruthy();
		expect(onAgentEvent).not.toHaveBeenCalled();
	});
});
