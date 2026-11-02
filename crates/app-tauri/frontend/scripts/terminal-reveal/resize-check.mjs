import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

export async function checkResizeGrip(page, handle, name) {
	handle = page.getByRole('separator', {
		name: await handle.getAttribute('aria-label'),
		exact: true,
		includeHidden: true,
	});
	const appearance = await page.evaluate(() => ({
		htmlStyle: document.documentElement.getAttribute('style'),
		bodyStyle: document.body.getAttribute('style'),
		dark: document.documentElement.classList.contains('dark'),
		height: window.workspacePanelsFixture.snapshot().terminal.panelHeight,
	}));
	const output = fileURLToPath(
		new URL(
			'../../../../../.husk/scratch/independent-workspace-panels/',
			import.meta.url
		)
	);
	const platform = await page.evaluate(() =>
		navigator.platform.startsWith('Mac') ? 'mac' : 'non-mac'
	);
	const sample = () =>
		handle.evaluate((el) => {
			const r = el.getBoundingClientRect(),
				css = getComputedStyle(el, '::after');
			const matrix = new DOMMatrix(css.transform);
			const width = Number.parseFloat(css.width),
				height = Number.parseFloat(css.height);
			const percent = (value, size) =>
				value.endsWith('%')
					? (Number.parseFloat(value) * size) / 100
					: Number.parseFloat(value);
			const x = r.left + percent(css.left, r.width) + matrix.m41;
			const y = r.top + percent(css.top, r.height) + matrix.m42;
			const neutral = document.createElement('span');
			neutral.style.color = 'var(--husk-fg-muted)';
			el.append(neutral);
			const color = getComputedStyle(neutral).color;
			neutral.remove();
			const panel = el.closest('.terminal-panel').getBoundingClientRect();
			return {
				shared: el.classList.contains('layout-resize-grip'),
				orientation: el.getAttribute('aria-orientation'),
				width,
				height,
				opacity: Number(css.opacity),
				color: css.backgroundColor,
				neutral: color,
				background: getComputedStyle(el).backgroundColor,
				transition: css.transitionDuration,
				centerError: [
					x + width / 2 - r.left - r.width / 2,
					y + height / 2 - r.top - r.height / 2,
				],
				clipped: y < panel.top || y + height > panel.bottom,
				captured: el.hasPointerCapture(1),
				dragging:
					el.hasAttribute('data-dragging') ||
					el.closest('.terminal-panel').hasAttribute('data-resizing'),
				retainedFocus: document.activeElement === window.__gripInput,
			};
		});
	const point = async () => {
		const b = await handle.boundingBox();
		return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
	};
	const input = page
		.locator('.terminal-panel__group:not([hidden]) .xterm-helper-textarea')
		.first();
	await input.focus();
	await page.evaluate(() => {
		window.__gripInput = document.activeElement;
	});
	await page.mouse.move(700, 18);
	await page.waitForTimeout(180);
	const initial = await sample();
	let p = await point();
	await page.mouse.move(p.x, p.y);
	await page.mouse.down();
	const pressed = await sample();
	await page.evaluate(() => window.dispatchEvent(new Event('blur')));
	const blurred = await sample();
	await page.mouse.up();
	console.log(
		JSON.stringify({ grip: name, platform, initial, pressed, blurred })
	);
	assert.equal(
		blurred.dragging,
		false,
		`${name}: window blur clears the drag marker`
	);
	assert.equal(
		blurred.captured,
		false,
		`${name}: window blur releases pointer capture`
	);
	assert.equal(
		pressed.retainedFocus,
		true,
		`${name}: pointer down retains terminal input focus`
	);

	for (const theme of ['light', 'dark']) {
		await page.evaluate(
			(value) => window.workspacePanelsFixture.theme(value),
			theme
		);
		await input.click();
		await page.mouse.move(700, 18);
		await page.waitForTimeout(180);
		const idle = await sample();
		assert.equal(idle.shared, true, `${name}: common paint owner`);
		assert.equal(idle.opacity, 0, `${name}: idle is hidden`);
		assert.deepEqual(
			[idle.width, idle.height],
			idle.orientation === 'vertical' ? [3, 26] : [26, 3]
		);
		assert.equal(idle.color, idle.neutral);
		assert.equal(
			idle.background,
			'rgba(0, 0, 0, 0)',
			'hitbox stays transparent'
		);
		assert.ok(
			idle.centerError.every((value) => Math.abs(value) <= 0.5),
			JSON.stringify(idle)
		);
		assert.equal(
			idle.clipped,
			false,
			'the entire pill is inside the clipped panel'
		);
		await handle.hover();
		await page.waitForTimeout(180);
		assert.equal((await sample()).opacity, 0.8);
		await page.screenshot({
			path: `${output}terminal-grip-${platform}-${name}-${theme}-hover.png`,
		});
		await page.mouse.move(700, 18);
		await page.waitForTimeout(180);
		assert.equal((await sample()).opacity, 0);
		await handle.focus();
		await page.keyboard.press('Tab');
		await page.keyboard.press('Shift+Tab');
		assert.equal(
			await handle.evaluate(
				(el) => el === document.activeElement && el.matches(':focus-visible')
			),
			true
		);
		await page.waitForTimeout(180);
		assert.equal(
			(await sample()).opacity,
			0.8,
			'real keyboard focus shows the pill'
		);
		const before = Number(await handle.getAttribute('aria-valuenow'));
		await page.keyboard.press(name === 'columns' ? 'ArrowRight' : 'ArrowUp');
		assert.notEqual(
			Number(await handle.getAttribute('aria-valuenow')),
			before,
			'keyboard resize still works'
		);
		await page.keyboard.press(name === 'columns' ? 'ArrowLeft' : 'ArrowDown');
		await input.click();
		await page.evaluate(() => {
			window.__gripInput = document.activeElement;
		});
		await page.waitForTimeout(900);
		await page.evaluate(() => {
			window.__gripNodes = [
				...document.querySelectorAll(
					'.terminal-panel__screen, .xterm-helper-textarea'
				),
			];
			const metrics = window.__terminalMetrics;
			metrics.fits ??= [];
			window.__gripStart = performance.now() - metrics.start;
		});
		p = await point();
		await page.mouse.move(p.x, p.y);
		await page.mouse.down();
		await page.mouse.move(
			p.x + (name === 'columns' ? 24 : 0),
			p.y + (name === 'columns' ? 0 : -18)
		);
		await page.waitForTimeout(180);
		assert.equal((await sample()).dragging, true);
		assert.equal((await sample()).opacity, 0.8);
		const held = await page.evaluate(() => ({
			fits: window.__terminalMetrics.fits.filter(
				(call) => call.at >= window.__gripStart
			).length,
			resizes: window.__terminalMetrics.ipc.filter(
				(call) =>
					call.command === 'terminal_resize' && call.at >= window.__gripStart
			).length,
		}));
		assert.deepEqual(
			held,
			{ fits: 0, resizes: 0 },
			'each axis freezes renderer/PTY geometry while held'
		);
		await page.screenshot({
			path: `${output}terminal-grip-${platform}-${name}-${theme}-drag.png`,
		});
		await page.mouse.up();
		await page.mouse.move(700, 18);
		await page.waitForTimeout(800);
		const released = await sample();
		assert.equal(released.dragging, false);
		assert.equal(released.captured, false);
		assert.equal(released.retainedFocus, true);
		assert.equal(
			released.opacity,
			0,
			'release and mouse leave hide the pill without artificial blur'
		);
		const settled = await page.evaluate(() => {
			const nodes = [
				...document.querySelectorAll(
					'.terminal-panel__screen, .xterm-helper-textarea'
				),
			];
			return {
				retainedNodes: window.__gripNodes.every((node, i) => node === nodes[i]),
				fits: window.__terminalMetrics.fits.filter(
					(call) => call.at >= window.__gripStart
				).length,
				resizes: window.__terminalMetrics.ipc.filter(
					(call) =>
						call.command === 'terminal_resize' && call.at >= window.__gripStart
				).length,
				panes: document.querySelectorAll(
					'.terminal-panel__group:not([hidden]) .terminal-panel__screen'
				).length,
			};
		});
		assert.equal(
			settled.retainedNodes,
			true,
			'resizing retains renderer and input nodes'
		);
		assert.ok(
			settled.fits <= settled.panes && settled.resizes <= settled.panes,
			JSON.stringify(settled)
		);
		for (const event of ['pointercancel', 'lostpointercapture']) {
			p = await point();
			await page.mouse.move(p.x, p.y);
			await page.mouse.down();
			await handle.dispatchEvent(event, { pointerId: 1 });
			assert.equal(
				(await sample()).dragging,
				false,
				`${event} clears the marker`
			);
			await page.mouse.up();
		}
		console.log(
			JSON.stringify({
				grip: name,
				platform,
				theme,
				idle,
				released,
				held,
				settled,
			})
		);
	}
	p = await point();
	await page.mouse.move(p.x, p.y);
	await page.mouse.down();
	await page.mouse.move(p.x + 10, p.y - 10);
	await page.evaluate(() => window.workspacePanelsFixture.terminalOpen(false));
	await page.waitForTimeout(350);
	const hidden = await sample();
	assert.equal(hidden.dragging, false, 'hiding clears the drag marker');
	assert.equal(hidden.captured, false, 'hiding releases pointer capture');
	await page.mouse.up();
	await page.evaluate(() => window.workspacePanelsFixture.terminalOpen(true));
	await page.waitForTimeout(900);
	await input.click();
	await page.mouse.move(700, 18);
	await page.waitForTimeout(180);
	assert.equal((await sample()).opacity, 0, 'reopening stays idle');
	console.log(JSON.stringify({ grip: name, hidden }));
	await page.emulateMedia({ reducedMotion: 'reduce' });
	assert.equal(
		(await sample()).transition,
		'0s',
		'all grips respect reduced motion'
	);
	await page.emulateMedia({ reducedMotion: 'no-preference' });
	await page.evaluate((value) => {
		document.documentElement.classList.toggle('dark', value.dark);
		for (const [element, style] of [
			[document.documentElement, value.htmlStyle],
			[document.body, value.bodyStyle],
		]) {
			if (style === null) element.removeAttribute('style');
			else element.setAttribute('style', style);
		}
		window.workspacePanelsFixture.terminalHeight(value.height);
	}, appearance);
	await page.waitForTimeout(900);
	assert.deepEqual(
		await page.evaluate(() => ({
			htmlStyle: document.documentElement.getAttribute('style'),
			bodyStyle: document.body.getAttribute('style'),
			dark: document.documentElement.classList.contains('dark'),
			height: window.workspacePanelsFixture.snapshot().terminal.panelHeight,
		})),
		appearance,
		'grip checks restore the original performance scenario inputs'
	);
}

export async function checkTerminalResize(page) {
	await page
		.locator('#session-navigation')
		.getByRole('button', { name: '打开终端', exact: true })
		.click();
	await page.waitForSelector('.xterm-rows');
	await page.waitForTimeout(1600);
	// Instrument the real project renderer, not mocked dimensions. No tracing
	// code is installed in production.
	await page.evaluate(async () => {
		const url = performance
			.getEntriesByType('resource')
			.find((entry) => entry.name.includes('/terminal-session.ts'))?.name;
		const { TerminalSession } = await import(url);
		const original = TerminalSession.prototype.fitNow;
		TerminalSession.prototype.fitNow = function () {
			const start = performance.now();
			try {
				return original.call(this);
			} finally {
				window.__terminalMetrics.fits?.push({
					at: performance.now() - window.__terminalMetrics.start,
					ms: performance.now() - start,
					cols: this.terminal.cols,
					rows: this.terminal.rows,
				});
			}
		};
	});
	const begin = () =>
		page.evaluate(() => {
			const metrics = (window.__terminalMetrics = {
				start: performance.now(),
				ipc: [],
				fits: [],
				frames: [],
				active: true,
				released: null,
				initialStreamHeight:
					document.querySelector('.stream-scroll').clientHeight,
			});
			let previous = metrics.start;
			const sample = () => {
				const now = performance.now();
				metrics.frames.push({
					at: now - metrics.start,
					gap: now - previous,
					streamHeight: document.querySelector('.stream-scroll').clientHeight,
					followGap:
						document
							.querySelector('[data-terminal-composer]')
							.getBoundingClientRect().bottom +
						(Number.parseFloat(
							getComputedStyle(
								document.querySelector('.terminal-panel')
							).getPropertyValue('--terminal-gap')
						) || 0) -
						document
							.querySelector('.terminal-panel__surface')
							.getBoundingClientRect().top,
					geometryGap: Math.abs(
						document
							.querySelector('.terminal-panel__surface')
							.getBoundingClientRect().height -
							document.querySelector('.terminal-panel').getBoundingClientRect()
								.height +
							(Number.parseFloat(
								getComputedStyle(
									document.querySelector('.terminal-panel')
								).getPropertyValue('--terminal-gap')
							) || 0)
					),
				});
				previous = now;
				if (metrics.active) requestAnimationFrame(sample);
			};
			requestAnimationFrame(sample);
		});
	const finish = async (vertical = false) => {
		await page.waitForTimeout(800);
		const metrics = await page.evaluate(() => {
			window.__terminalMetrics.active = false;
			return window.__terminalMetrics;
		});
		const requests = metrics.ipc.filter(
			(entry) => entry.command === 'terminal_resize'
		);
		const late = requests.filter((entry) => entry.at > metrics.released + 400);
		const gaps = metrics.frames
			.filter((entry) => entry.at < metrics.released)
			.map((entry) => entry.gap)
			.sort((a, b) => a - b);
		const renderer = metrics.fits.at(-1);
		const last = requests.at(-1);
		const during = metrics.frames.filter(
			(entry) => entry.at < metrics.released
		);
		let previousHeight = metrics.initialStreamHeight;
		let dragLayouts = 0;
		for (const entry of during) {
			if (Math.abs(entry.streamHeight - previousHeight) > 0.5) dragLayouts++;
			previousHeight = entry.streamHeight;
		}
		const summary = {
			durationMs: metrics.released,
			fits: metrics.fits.length,
			totalFitMs: metrics.fits.reduce((total, entry) => total + entry.ms, 0),
			p95FrameMs: gaps[Math.floor(gaps.length * 0.95)],
			maxFrameMs: Math.max(...gaps),
			requests: requests.length,
			lateRequests: late.length,
			resizeTailMs: last?.at - metrics.released,
			renderer,
			lastRequest: last,
			dragLayouts,
			fitsWhileDragging: metrics.fits.filter(
				(entry) => entry.at < metrics.released
			).length,
			maxFollowGapPx: Math.max(
				0,
				...during.map((entry) => Math.abs(entry.followGap))
			),
			maxGeometryGapPx: Math.max(
				0,
				...during.map((entry) => entry.geometryGap)
			),
		};
		console.log(JSON.stringify(summary));
		assert.equal(
			late.length,
			0,
			'release must not leave obsolete PTY sizes queued for seconds'
		);
		assert.ok(last && renderer, 'gesture must exercise real xterm resize');
		assert.equal(
			last.rows,
			renderer.rows,
			"backend must settle at the renderer's final rows"
		);
		assert.equal(
			last.cols,
			renderer.cols,
			"backend must settle at the renderer's final columns"
		);
		if (vertical) {
			assert.ok(
				summary.maxGeometryGapPx < 1,
				'dragged panel bounds must match the visible surface and menu hit area'
			);
			assert.equal(
				summary.fitsWhileDragging,
				0,
				'hold xterm/PTY geometry stable until pointer release'
			);
			assert.ok(
				summary.maxFollowGapPx < 2,
				'composer must follow the drag boundary'
			);
		}
		return summary;
	};

	const handle = page.getByRole('separator', { name: '调整终端高度' });
	await checkResizeGrip(page, handle, 'height');
	const box = await handle.boundingBox();
	const x = box.x + box.width / 2;
	const y = box.y + 3;
	await page.mouse.move(x, y);
	await begin();
	await page.evaluate(() =>
		document.addEventListener(
			'pointerdown',
			(event) => {
				window.__terminalMetrics.pointer = event.pointerId;
			},
			{ once: true, capture: true }
		)
	);
	await page.mouse.down();
	// Native press/capture; fixed-rate PointerEvents thereafter. Awaiting each
	// Playwright mouse.move lets the slow UI itself throttle the driver, hiding
	// the backlog that a real high-rate pointer creates.
	await page.evaluate(
		async ({ x, y }) => {
			const handle = document.querySelector(
				'[role="separator"][aria-label="调整终端高度"]'
			);
			const pointerId = window.__terminalMetrics.pointer;
			for (const [from, to] of [
				[y, y - 240],
				[y - 240, y + 120],
				[y + 120, y - 120],
			]) {
				for (let n = 1; n <= 60; n++) {
					handle.dispatchEvent(
						new PointerEvent('pointermove', {
							bubbles: true,
							pointerId,
							buttons: 1,
							clientX: x,
							clientY: from + ((to - from) * n) / 60,
						})
					);
					await new Promise((resolve) => setTimeout(resolve, 8));
				}
			}
			window.__terminalMetrics.released =
				performance.now() - window.__terminalMetrics.start;
			handle.dispatchEvent(
				new PointerEvent('pointerup', {
					bubbles: true,
					pointerId,
					button: 0,
					clientX: x,
					clientY: y - 120,
				})
			);
		},
		{ x, y }
	);
	await page.mouse.up();
	const height = await finish(true);

	await begin();
	await page.evaluate(async () => {
		const main = document.querySelector('#root > div');
		for (const [from, to] of [
			[1280, 680],
			[680, 1080],
			[1080, 960],
		]) {
			for (let n = 1; n <= 40; n++) {
				main.style.width = `${Math.round(from + ((to - from) * n) / 40)}px`;
				await new Promise((resolve) => setTimeout(resolve, 8));
			}
		}
		window.__terminalMetrics.released =
			performance.now() - window.__terminalMetrics.start;
	});
	const width = await finish();
	assert.ok(
		width.fits <= 3,
		'continuous width input must not reflow the entire scrollback every frame'
	);
	return { height, width };
}
