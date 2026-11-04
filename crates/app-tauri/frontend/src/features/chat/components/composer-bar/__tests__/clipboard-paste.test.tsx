import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import { ComposerBar } from '../index';
import { emptyView } from '@/features/chat/hooks/stream-view';
import { useAgentStore } from '@/stores/agent-store';
import { renderWithProviders } from '@/test/render';
import { mockCommand, tauri } from '@/test/tauri';
import type { Attachment } from '@/lib/agent-ipc/sessions';

const platform = vi.hoisted(() => ({ linux: false }));
vi.mock('@/lib/platform', async (importOriginal) => ({
	...(await importOriginal<typeof import('@/lib/platform')>()),
	get isLinux() {
		return platform.linux;
	},
}));

const image = (path: string, name = 'clipboard.png'): Attachment => ({
	path,
	name,
	kind: 'image',
	data_url: 'data:image/png;base64,aW1hZ2U=',
});
const file = (name = 'clipboard.png') =>
	new File(['image'], name, { type: 'image/png' });
const dataTransfer = (files: File[]) => ({
	files,
	items: files.map((value) => ({
		kind: 'file',
		type: value.type,
		getAsFile: () => value,
	})),
	types: ['Files'],
	getData: () => '',
});
const calls = (command: string, op?: string) =>
	tauri.invoke.mock.calls.filter(
		([name, args]) => name === command && (!op || args?.op === op)
	);
const attachments = () => screen.queryAllByRole('button', { name: /^移除 / });

beforeEach(() => {
	platform.linux = false;
	useAgentStore.setState(useAgentStore.getInitialState(), true);
	let sequence = 0;
	mockCommand('paste_clipboard', () =>
		image('/ws/.husk/attachments/native.png')
	);
	mockCommand(
		'agent_session',
		(args) =>
			image(
				`/ws/.husk/attachments/paste-${++sequence}.png`,
				(args?.payload as { name: string }).name
			),
		'attach_bytes'
	);
});

async function input() {
	renderWithProviders(<ComposerBar view={emptyView()} sessionKey="/ws:1" />);
	await screen.findByRole('button', { name: /当前模型: Model A/ });
	return screen.getByRole('textbox');
}

describe('composer clipboard images', () => {
	test.each([
		['macOS', { metaKey: true }],
		['Windows', { ctrlKey: true }],
	])(
		'%s attaches one image for one shortcut and paste event',
		async (_, modifier) => {
			const field = await input();
			fireEvent.keyDown(field, { key: 'v', ...modifier });
			fireEvent.paste(field, {
				clipboardData: dataTransfer([file('pasted.png')]),
			});
			await screen.findByRole('button', { name: '移除 pasted.png' });
			expect(attachments()).toHaveLength(1);
			expect(calls('agent_session', 'attach_bytes')).toHaveLength(1);
			expect(calls('paste_clipboard')).toHaveLength(0);
		}
	);

	test('GTK retains native image paste when the event has no clipboard data', async () => {
		platform.linux = true;
		const field = await input();
		fireEvent.keyDown(field, { key: 'v', ctrlKey: true });
		fireEvent.paste(field, { clipboardData: null });
		await waitFor(() => expect(attachments()).toHaveLength(1));
		expect(calls('paste_clipboard')).toHaveLength(1);
		expect(calls('agent_session', 'attach_bytes')).toHaveLength(0);
	});

	test('a multi-image paste uses each file once even when items and files alias', async () => {
		const field = await input();
		fireEvent.keyDown(field, { key: 'v', metaKey: true });
		fireEvent.paste(field, {
			clipboardData: dataTransfer([file('first.png'), file('second.png')]),
		});
		await waitFor(() => expect(attachments()).toHaveLength(2));
		expect(calls('agent_session', 'attach_bytes')).toHaveLength(2);
		expect(calls('paste_clipboard')).toHaveLength(0);
	});

	test('separate paste operations can each attach the same image', async () => {
		const field = await input();
		for (let count = 1; count <= 2; count++) {
			fireEvent.keyDown(field, { key: 'v', metaKey: true });
			fireEvent.paste(field, { clipboardData: dataTransfer([file()]) });
			await waitFor(() => expect(attachments()).toHaveLength(count));
		}
		expect(calls('agent_session', 'attach_bytes')).toHaveLength(2);
		expect(calls('paste_clipboard')).toHaveLength(0);
	});

	test('plain text keeps the browser paste behavior without a native image read', async () => {
		mockCommand('paste_clipboard', () => null);
		const field = await input();
		const user = userEvent.setup();
		await user.click(field);
		fireEvent.keyDown(field, { key: 'v', metaKey: true });
		await user.paste('plain clipboard text');
		expect(field).toHaveValue('plain clipboard text');
		expect(attachments()).toHaveLength(0);
		expect(calls('paste_clipboard')).toHaveLength(0);
	});
});
