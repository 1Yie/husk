// Dragging the reasoning-effort thumb past the popup's edge is part of one
// gesture: the release must not be read as a click on the page and close the
// menu, or the highest tiers become unreachable.

import { fireEvent, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';
import { ModelPicker } from '../model-picker';
import { renderWithProviders } from '@/test/render';

const models = [
	{
		provider: 'test',
		model: 'model-a',
		name: 'Model A',
		reasoning: true,
		available_levels: ['off', 'low', 'high', 'max'],
		thinking_level_map: { low: 'low', high: 'high', max: 'max' },
	},
];

function picker() {
	return (
		<ModelPicker
			activeModel="model-a"
			activeProvider="test"
			models={models}
			onSelectModel={vi.fn(async () => {})}
			hasReasoning
			activeThinkingLevel="low"
			thinkingMap={{ low: 'low', high: 'high', max: 'max' }}
			effectiveThinkingLevels={[
				{ value: 'off', token: 'off' },
				{ value: 'low', token: 'low' },
				{ value: 'high', token: 'high' },
				{ value: 'max', token: 'max' },
			]}
			onSelectThinkingLevel={vi.fn(async () => {})}
		/>
	);
}

async function openThinking(user: ReturnType<typeof userEvent.setup>) {
	await user.click(screen.getByRole('button', { name: /当前模型:/ }));
	await user.click(screen.getByRole('menuitem', { name: /^思考/ }));
}

describe('reasoning effort drag', () => {
	test('a drag that leaves the popup keeps it open', async () => {
		const user = userEvent.setup();
		renderWithProviders(picker());
		await openThinking(user);

		const slider = screen.getByRole('slider', { name: '思考等级' });
		fireEvent.pointerDown(slider, { button: 0, clientX: 10, pointerId: 1 });
		// The pointer is now outside the popup: a plain click there would dismiss it.
		fireEvent.pointerDown(document.body, {
			button: 0,
			clientX: 400,
			pointerId: 1,
		});
		expect(screen.queryByRole('slider', { name: '思考等级' })).not.toBeNull();

		fireEvent.pointerUp(document.body, { clientX: 400, pointerId: 1 });
		expect(screen.queryByRole('slider', { name: '思考等级' })).not.toBeNull();
	});

	test('an ordinary click outside still closes it', async () => {
		const user = userEvent.setup();
		renderWithProviders(picker());
		await openThinking(user);
		fireEvent.pointerDown(document.body, { button: 0, clientX: 400 });
		expect(screen.queryByRole('slider', { name: '思考等级' })).toBeNull();
	});
});
