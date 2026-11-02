import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';

/** Sample the real CSS transitions with their clocks paused at known times. */
export async function checkPanelClip({ browser, base }) {
	const output = process.env.PANEL_CLIP_OUTPUT;
	assert.ok(
		output && path.isAbsolute(output),
		'PANEL_CLIP_OUTPUT must be a fresh absolute directory'
	);
	await mkdir(output);
	const baseline = process.argv.includes('--panel-clip-baseline');
	const report = {
		browser: browser.version(),
		method: 'paused CSS transition clocks; painted PNG corner comparison',
		cases: [],
		errors: [],
	};
	const failures = [];
	try {
		for (const mac of baseline ? [true] : [true, false]) {
			for (const width of baseline ? [1280] : [1280, 960]) {
				const page = await browser.newPage({
					viewport: { width, height: 840 },
				});
				page.on('pageerror', (error) => report.errors.push(error.message));
				await page.addInitScript((mac) => {
					Object.defineProperty(navigator, 'platform', {
						value: mac ? 'MacIntel' : 'Linux x86_64',
					});
					Object.defineProperty(navigator, 'userAgent', {
						value: mac
							? 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'
							: 'Mozilla/5.0 (X11; Linux x86_64)',
					});
				}, mac);
				try {
					await page.goto(`${base}terminal-reveal.html`);
					await page.locator('#workspace-tools-sidebar').waitFor();
					await page.waitForTimeout(600);
					for (const theme of baseline ? ['light'] : ['light', 'dark']) {
						await page.evaluate(
							(theme) => window.workspacePanelsFixture.theme(theme),
							theme
						);
						await page.waitForTimeout(400);
						for (const kind of ['dock', 'terminal']) {
							const root =
								kind === 'dock'
									? page.locator('#workspace-tools-sidebar').locator('..')
									: page.locator('.terminal-panel');
							const setOpen = async (open) => {
								if (kind === 'terminal') {
									await page.evaluate(
										(open) => window.workspacePanelsFixture.terminalOpen(open),
										open
									);
								} else {
									await page
										.getByRole('button', {
											name: open ? '展开工具侧栏' : '收起工具侧栏',
											exact: true,
										})
										.click();
								}
							};
							if (kind === 'dock') await setOpen(false);
							await page.waitForTimeout(400);
							const run = { mac, width, theme, kind, frames: [] };
							report.cases.push(run);
							for (const opening of [true, false]) {
								await setOpen(opening);
								const clocks = await root.evaluate((el, kind) => {
									const animations = el
										.getAnimations({ subtree: kind === 'dock' })
										.filter((animation) =>
											['width', 'transform', 'height'].includes(
												animation.transitionProperty
											)
										);
									window.__panelClipClocks = animations;
									for (const animation of animations) {
										animation.pause();
										animation.currentTime = 0;
									}
									return animations.map((animation) => ({
										property: animation.transitionProperty,
										duration: animation.effect.getTiming().duration,
									}));
								}, kind);
								assert.equal(
									clocks.length,
									kind === 'dock' ? 2 : 1,
									'the real width/transform or height transitions run'
								);
								assert.ok(
									clocks.every(
										(clock) => clock.duration === (kind === 'dock' ? 300 : 240)
									)
								);
								for (const time of kind === 'dock'
									? [20, 60, 120, 220]
									: [20, 60, 120, 200]) {
									await page.evaluate((time) => {
										for (const animation of window.__panelClipClocks)
											animation.currentTime = time;
									}, time);
									const geometry = await root.evaluate((el, kind) => {
										const card =
											kind === 'dock'
												? el.querySelector('aside')
												: el.querySelector('.terminal-panel__surface');
										const rect = (node) =>
											node.getBoundingClientRect().toJSON();
										const main = el.closest('.main-col');
										const chat = document.querySelector('.chat-workspace-card');
										const bounds = rect(el),
											inner = rect(card);
										const corner = document.elementFromPoint(
											bounds.right - 1,
											bounds.top + 1
										);
										return {
											root: bounds,
											card: inner,
											radius: getComputedStyle(el).borderTopRightRadius,
											cardRadius: getComputedStyle(card).borderTopRightRadius,
											background: getComputedStyle(el).backgroundColor,
											overflow: getComputedStyle(el).overflow,
											cornerHit: !!corner && el.contains(corner),
											main: {
												rect: rect(main),
												radius: getComputedStyle(main).borderRadius,
												overflow: getComputedStyle(main).overflow,
												isolation: getComputedStyle(main).isolation,
											},
											followGap:
												kind === 'terminal'
													? inner.top - rect(chat).bottom
													: bounds.left - rect(chat).right,
											cardWidth: kind === 'dock' ? inner.width : undefined,
											cardMargin:
												kind === 'dock'
													? getComputedStyle(card).marginLeft
													: undefined,
										};
									}, kind);
									const name = `${mac ? 'mac' : 'nonmac'}-${width}-${theme}-${kind}-${opening ? 'opening' : 'closing'}-${time}`;
									const actual = await page.screenshot({
										path: path.join(output, `${name}.png`),
										caret: 'hide',
									});
									const expectedRadius = geometry.cardRadius;
									// The alternate radius changes only the clip, keeping the same
									// paused content, layout and paint for the pixel comparison.
									const alternate =
										geometry.radius === '0px' ? expectedRadius : '0px';
									await root.evaluate((el, value) => {
										el.style.borderRadius = value;
									}, alternate);
									const control = await page.screenshot({
										path: path.join(output, `${name}-clip-control.png`),
										caret: 'hide',
									});
									await root.evaluate((el, value) => {
										el.style.borderRadius = value;
									}, expectedRadius);
									const expected = await page.screenshot({
										path: path.join(output, `${name}-expected.png`),
										caret: 'hide',
									});
									await root.evaluate((el) => {
										el.style.removeProperty('border-radius');
									});
									const pixels = await compareCorners(
										page,
										actual,
										control,
										geometry.root
									);
									const expectedPixels = await compareCorners(
										page,
										actual,
										expected,
										geometry.root
									);
									run.frames.push({
										opening,
										time,
										clocks,
										...geometry,
										alternate,
										pixels,
										expectedPixels,
										screenshot: `${name}.png`,
									});
									if (geometry.radius !== expectedRadius)
										failures.push(
											`${name}: clip ${geometry.radius}, card ${expectedRadius}`
										);
									if (expectedPixels.changed > 0)
										failures.push(
											`${name}: ${expectedPixels.changed} corner pixels differ from the matching-radius reference`
										);
									assert.equal(
										geometry.background,
										'rgba(0, 0, 0, 0)',
										'the animation wrapper remains transparent'
									);
									assert.equal(geometry.overflow, 'hidden');
									assert.equal(
										geometry.main.radius,
										'0px',
										'main-col remains the existing rectangular layout boundary'
									);
									assert.equal(geometry.main.isolation, 'isolate');
									assert.ok(
										Math.abs(
											geometry.followGap - (mac && kind === 'terminal' ? 8 : 0)
										) < 0.6,
										`${name}: gap tracks the real layout`
									);
									if (kind === 'dock') {
										assert.equal(
											geometry.cardWidth,
											300,
											'the inner card keeps its full visible width'
										);
										assert.equal(geometry.cardMargin, mac ? '8px' : '0px');
									}
									if (!mac)
										assert.equal(
											pixels.changed,
											0,
											'flat panels retain the same paint'
										);
								}
								await page.evaluate(() => {
									for (const animation of window.__panelClipClocks)
										animation.finish();
									delete window.__panelClipClocks;
								});
								await page.waitForTimeout(400);
								const extent = await root.evaluate((el, kind) => {
									const r = el.getBoundingClientRect();
									return kind === 'dock' ? r.width : r.height;
								}, kind);
								if (!opening)
									assert.equal(
										extent,
										0,
										'closed panels remove the whole clip and gap'
									);
							}
							if (mac && kind === 'dock') {
								for (const opening of [true, false]) {
									assert.ok(
										run.frames.some(
											(frame) =>
												frame.opening === opening && frame.pixels.changed > 8
										),
										'painted corner pixels distinguish square and rounded clips in each direction'
									);
								}
							}
							if (kind === 'terminal') {
								// Its surface follows the changing height; the control captures
								// whether the outer clip additionally contributes to the paint.
								run.maxClipPixelDifference = Math.max(
									...run.frames.map((frame) => frame.pixels.changed)
								);
							}
							if (kind === 'dock') {
								await setOpen(true);
								await page.waitForTimeout(400);
							}
						}
					}
				} finally {
					await page.close();
				}
			}
		}
		assert.deepEqual(report.errors, []);
		assert.deepEqual(
			failures,
			[],
			'animation clip roots match their visible cards'
		);
	} finally {
		report.failures = failures;
		await writeFile(
			path.join(output, 'report.json'),
			JSON.stringify(report, null, 2)
		);
		console.log(
			JSON.stringify({
				output,
				cases: report.cases.length,
				failures: failures.length,
				errors: report.errors,
			})
		);
	}
}

async function compareCorners(page, actual, control, rect) {
	return page.evaluate(
		async ({ actual, control, rect }) => {
			const decode = async (encoded) => {
				const image = new Image();
				image.src = `data:image/png;base64,${encoded}`;
				await image.decode();
				const canvas = document.createElement('canvas');
				canvas.width = image.width;
				canvas.height = image.height;
				const context = canvas.getContext('2d');
				context.drawImage(image, 0, 0);
				return context.getImageData(0, 0, canvas.width, canvas.height);
			};
			const [a, b] = await Promise.all([decode(actual), decode(control)]);
			let changed = 0,
				maxDifference = 0;
			for (
				let y = Math.max(0, Math.floor(rect.top));
				y < Math.min(a.height, Math.ceil(rect.bottom));
				y++
			) {
				if (y >= rect.top + 14 && y < rect.bottom - 14) continue;
				for (
					let x = Math.max(0, Math.floor(rect.left));
					x < Math.min(a.width, Math.ceil(rect.right));
					x++
				) {
					if (x >= rect.left + 14 && x < rect.right - 14) continue;
					const index = (y * a.width + x) * 4;
					const difference = Math.max(
						...[0, 1, 2].map((channel) =>
							Math.abs(a.data[index + channel] - b.data[index + channel])
						)
					);
					if (difference > 2) changed++;
					maxDifference = Math.max(maxDifference, difference);
				}
			}
			return { changed, maxDifference };
		},
		{
			actual: actual.toString('base64'),
			control: control.toString('base64'),
			rect,
		}
	);
}
