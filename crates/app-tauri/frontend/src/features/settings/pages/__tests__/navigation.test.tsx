import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, test, vi } from 'vitest';
import { renderWithProviders } from '@/test/render';
import { SettingsPage } from '..';

await vi.hoisted(async () => {
	const { installDomMocks } = await import('@/test/dom');
	installDomMocks();
});
const { agentPane } = vi.hoisted(() => ({
	agentPane: vi.fn((_props: { tab: string }) => null),
}));
vi.mock('@/features/settings/pages/agent', () => ({
	AgentSettings: agentPane,
}));

test('the connectors label keeps the MCP route for direct and sidebar navigation', async () => {
	const user = userEvent.setup();
	renderWithProviders(<SettingsPage initialTab="mcp" onClose={() => {}} />);

	expect(
		screen.getByRole('heading', { level: 1, name: '连接器' })
	).toBeInTheDocument();
	expect(screen.queryByRole('button', { name: 'MCP' })).not.toBeInTheDocument();
	expect(agentPane.mock.calls.at(-1)?.[0]).toMatchObject({ tab: 'mcp' });

	await user.click(screen.getByRole('button', { name: '插件' }));
	expect(agentPane.mock.calls.at(-1)?.[0]).toMatchObject({ tab: 'plugins' });
	await user.click(screen.getByRole('button', { name: '连接器' }));
	expect(
		screen.getByRole('heading', { level: 1, name: '连接器' })
	).toBeInTheDocument();
	expect(agentPane.mock.calls.at(-1)?.[0]).toMatchObject({ tab: 'mcp' });
});
