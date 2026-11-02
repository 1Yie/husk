import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';
import { ReasoningEffort } from '../reasoning-effort';
import { THINKING_LEVELS, thinkingTone } from '@/lib/thinking';

function luminance(channels: readonly number[]) {
	const linear = channels.map((channel) => {
		const value = channel / 255;
		return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
	});
	return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}

describe('thinking tier overlay contrast', () => {
	const css = readFileSync(
		resolve(dirname(fileURLToPath(import.meta.url)), '../../../index.css'),
		'utf8'
	);
	const colors = [...css.matchAll(/--husk-think-([\w-]+):\s*(#[\da-f]{6});/gi)];
	const levels = THINKING_LEVELS.map((level) => level.value);

	// Frosted surfaces retain the opposite-color wallpaper behind the theme.
	test.each([
		{ mode: 'light', offset: 0, background: [209, 209, 209] },
		{ mode: 'dark', offset: levels.length, background: [94, 94, 100] },
	])('$mode tiers retain AA contrast', ({ mode, offset, background }) => {
		const palette = colors.slice(offset, offset + levels.length);
		expect(palette.map((color) => color[1])).toEqual(levels);
		for (const color of palette) {
			const foreground = [1, 3, 5].map((start) =>
				Number.parseInt(color[2].slice(start, start + 2), 16)
			);
			const light = Math.max(luminance(foreground), luminance(background));
			const dark = Math.min(luminance(foreground), luminance(background));
			expect(
				(light + 0.05) / (dark + 0.05),
				`${mode} ${color[1]}`
			).toBeGreaterThanOrEqual(4.5);
		}
	});
});

describe('reasoning effort tier colour', () => {
	test.each([0, 1, 2])(
		'the tier word keeps its full tone at stop %i',
		(value) => {
			const values = ['low', 'medium', 'max'];
			const stops = ['Low', 'Medium', 'Max'];
			render(
				<ReasoningEffort
					stops={stops}
					levelTones={values.map(thinkingTone)}
					value={value}
					label="Claude Opus 5.5"
				/>
			);
			expect(screen.getByText(stops[value])).toHaveStyle({
				color: thinkingTone(values[value]),
			});
			// The model name stays the caption colour — only the tier is toned.
			expect(
				screen.getByText('Claude Opus 5.5').getAttribute('style')
			).toBeNull();
		}
	);

	test('without tones the tier keeps the dim caption colour', () => {
		render(<ReasoningEffort stops={['Low', 'High']} value={1} label="m" />);
		expect(screen.getByText('High').getAttribute('style')).toBeNull();
	});

	test('an unknown tier falls back to the muted ink', () => {
		expect(thinkingTone('turbo')).toBe(
			'var(--husk-think-turbo, var(--husk-fg-muted))'
		);
	});

	test('Fast keeps front sparkles during the reveal, then starts background points only after transitionend', () => {
		const onChange = vi.fn();
		const { container, rerender } = render(
			<ReasoningEffort
				stops={['Low', 'High']}
				value={1}
				accent="blue"
				onChange={onChange}
			/>
		);
		const path = container.querySelector('svg path')!.getAttribute('d');
		expect(container.querySelector('[data-fast-fill]')).toBeInTheDocument();
		expect(
			container.querySelector('[data-fast-points]')
		).not.toBeInTheDocument();
		rerender(
			<ReasoningEffort
				stops={['Low', 'High']}
				value={1}
				accent="purple"
				onChange={onChange}
			/>
		);
		expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '1');
		expect(container.querySelector('svg path')).toHaveAttribute('d', path);
		expect(container.querySelector('[data-fast-front]')).toBeInTheDocument();
		expect(
			container.querySelector('[data-fast-points]')
		).not.toBeInTheDocument();
		expect(
			container.querySelector('[data-fast-ambient]')
		).not.toBeInTheDocument();
		const event = new Event('transitionend', { bubbles: true });
		Object.defineProperty(event, 'propertyName', { value: 'transform' });
		fireEvent(container.querySelector('[data-fast-fill]')!, event);
		expect(
			container.querySelector('[data-fast-front]')
		).not.toBeInTheDocument();
		expect(container.querySelector('[data-fast-points]')).toBeInTheDocument();
		expect(
			container.querySelector('[data-fast-background]')!.parentElement
		).toBe(screen.getByRole('slider'));
		expect(onChange).not.toHaveBeenCalled();
	});

	test("a restored Fast state skips the reveal's front sparkles", () => {
		const { container } = render(<ReasoningEffort accent="purple" />);
		expect(container.firstChild).toHaveAttribute('data-accent', 'purple');
		expect(
			container.querySelector('[data-fast-front]')
		).not.toBeInTheDocument();
	});

	test('opening completes before points appear, meteors follow later, and disabling or unmounting stops them', () => {
		vi.useFakeTimers();
		try {
			const { container, rerender, unmount } = render(
				<ReasoningEffort accent="purple" effectsEnabled={false} />
			);
			act(() => vi.advanceTimersByTime(5000));
			expect(
				container.querySelector('[data-fast-points]')
			).not.toBeInTheDocument();
			expect(
				container.querySelector('[data-fast-ambient]')
			).not.toBeInTheDocument();
			rerender(<ReasoningEffort accent="purple" effectsEnabled />);
			expect(
				container.querySelector('[data-fast-points]')!.childElementCount
			).toBe(10);
			expect(
				container.querySelector('[data-fast-ambient]')
			).not.toBeInTheDocument();
			act(() => vi.advanceTimersByTime(2201));
			expect(container.querySelectorAll('[data-fast-ambient]')).toHaveLength(1);
			fireEvent.animationEnd(container.querySelector('[data-fast-ambient]')!);
			expect(
				container.querySelector('[data-fast-ambient]')
			).not.toBeInTheDocument();
			rerender(<ReasoningEffort accent="blue" />);
			act(() => vi.advanceTimersByTime(10000));
			expect(
				container.querySelector('[data-fast-points]')
			).not.toBeInTheDocument();
			expect(
				container.querySelector('[data-fast-ambient]')
			).not.toBeInTheDocument();
			unmount();
			expect(vi.getTimerCount()).toBe(0);
		} finally {
			vi.useRealTimers();
		}
	});
});
