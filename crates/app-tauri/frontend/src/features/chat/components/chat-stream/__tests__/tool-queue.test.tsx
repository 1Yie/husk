import { fireEvent, screen } from '@testing-library/react';
import { expect, test } from 'vitest';
import { renderWithProviders } from '@/test/render';
import { ChatStream } from '../index';
import { applyEvents } from '@/features/chat/hooks/apply-event';
import { viewFromHistory } from '@/features/chat/hooks/view-from-history';
import type { UiEvent } from '@/types';

test.each([
	{ scope: 'root', parent: null },
	{ scope: 'nested', parent: 'bundle' },
])(
	'$scope tools display queued, running, and completed states',
	({ parent }) => {
		const started = (name: string, callId: string): UiEvent => ({
			ToolCallStarted: {
				name,
				call_id: callId,
				args_preview: 'queued.txt',
				parent: name === 'smart_read' ? parent : null,
			},
		});
		let view = applyEvents(viewFromHistory([]), [
			{ UserPrompt: 'Read the queued file' },
			{ StateChanged: 'Reasoning' },
			...(parent ? [started(parent, 'batch')] : []),
			{
				ToolCallQueued: {
					name: 'smart_read',
					call_id: 'read',
					args_preview: 'queued.txt',
					parent,
				},
			},
		]);
		const { rerender } = renderWithProviders(
			<ChatStream view={view} sessionKey={`queue-${parent ?? 'root'}`} />
		);
		fireEvent.click(screen.getByRole('button', { expanded: false }));
		if (parent)
			fireEvent.click(screen.getByRole('button', { name: /^bundle/ }));
		expect(
			screen.getByRole('button', { name: /smart_read.*queued.txt.*排队中/ })
		).toBeInTheDocument();

		view = applyEvents(view, [started('smart_read', 'read')]);
		rerender(
			<ChatStream view={view} sessionKey={`queue-${parent ?? 'root'}`} />
		);
		expect(
			screen.getByRole('button', { name: /smart_read.*queued.txt.*进行中/ })
		).toBeInTheDocument();

		view = applyEvents(view, [
			{
				ToolCallFinished: {
					name: 'smart_read',
					call_id: 'read',
					content: 'read complete',
					ok: true,
					parent,
				},
			},
		]);
		rerender(
			<ChatStream view={view} sessionKey={`queue-${parent ?? 'root'}`} />
		);
		expect(
			screen.getByRole('button', { name: /smart_read.*queued.txt.*完成/ })
		).toBeInTheDocument();
	}
);
