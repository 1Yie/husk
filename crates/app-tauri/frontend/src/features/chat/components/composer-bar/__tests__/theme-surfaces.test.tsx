import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';
import { ComposerBar } from '../index';
import { emptyView } from '@/features/chat/hooks/stream-view';
import { renderWithProviders } from '@/test/render';

describe('composer theme surfaces', () => {
	test.each(['normal', 'todo'])(
		'%s surfaces switch colour with the theme',
		async (scene) => {
			const view = emptyView();
			if (scene === 'todo') {
				view.items.push({
					kind: 'tool',
					name: 'todo',
					args: '{}',
					ok: true,
					content: '1/2 done\n[x] #1 Read code\n[ ] #2 Verify theme',
				});
			}
			const { container } = renderWithProviders(
				<ComposerBar view={view} sessionKey="/ws:1" />
			);
			await screen.findByRole('button', { name: /当前模型: Model A/ });
			const surface = container.querySelector(
				scene === 'todo'
					? '[class~="bg-panel"][class~="rounded-[24px]"]'
					: '[class~="bg-white"][class~="rounded-[22px]"]'
			);
			expect(surface).toBeInTheDocument();
			expect(surface).not.toHaveClass('transition-all');
			expect(surface).not.toHaveClass('transition-colors');

			if (scene === 'todo') {
				const list = surface!.querySelector('.grid') as HTMLElement;
				expect(list).toHaveClass(
					'transition-[grid-template-rows,opacity]',
					'duration-250'
				);
				expect(list.style.gridTemplateRows).toBe('1fr');
				fireEvent.click(surface!.querySelector('button')!);
				expect(list.style.gridTemplateRows).toBe('0fr');
				expect(list.style.opacity).toBe('0');
			}
		}
	);
});
