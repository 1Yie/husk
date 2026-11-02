import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';

export async function checkWelcomeResponsive({ browser, base }) {
	const output = process.env.WELCOME_RESPONSIVE_OUTPUT;
	assert.ok(
		output && path.isAbsolute(output),
		'WELCOME_RESPONSIVE_OUTPUT must be a fresh absolute directory'
	);
	await mkdir(output);
	const report = { browser: browser.version(), cases: [], errors: [] };
	const baseline = process.argv.includes('--welcome-responsive-baseline');
	try {
		for (const mac of [true, false]) {
			for (const zero of [false, true]) {
				const page = await browser.newPage({
					viewport: { width: 700, height: 840 },
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
					await page.goto(
						`${base}terminal-reveal.html?welcome-responsive${zero ? '&zero' : ''}`
					);
					const title = page.getByText('活跃对话', { exact: true });
					const card = title.locator('../..');
					await card.waitFor({ state: 'attached' });
					assert.match(
						await card.locator('header').textContent(),
						new RegExp(`${zero ? 0 : 1} 个会话`)
					);
					assert.equal(await card.locator('p').count(), zero ? 1 : 0);
					await page.evaluate(() =>
						window.welcomeResponsiveFixture.panels(false, 0)
					);
					await page.waitForTimeout(400);
					for (const scene of ['unrooted', 'workspace']) {
						await page.evaluate(
							(scene) => window.welcomeResponsiveFixture.scene(scene),
							scene
						);
						for (const theme of ['light', 'dark']) {
							await page.evaluate(
								(theme) => window.workspacePanelsFixture.theme(theme),
								theme
							);
							for (const width of [513, 514, 600]) {
								await page.setViewportSize({ width: 700, height: 840 });
								const current = await card.evaluate(
									(el) => el.parentElement.getBoundingClientRect().width
								);
								await page.setViewportSize({
									width: Math.round(700 + width - current),
									height: 840,
								});
								await page.waitForTimeout(80);
								const name = `${mac ? 'mac' : 'nonmac'}-${zero ? 'zero' : 'loaded'}-${scene}-${theme}-${width}`;
								const geometry = await measure(card);
								report.cases.push({ name, ...geometry });
								await page.screenshot({
									path: path.join(output, `${name}.png`),
								});
								assert.ok(
									Math.abs(geometry.container.width - width) < 0.6,
									name
								);
								await checkActions(page, scene);
								assert.equal(
									geometry.card.display,
									width < 514 ? 'none' : 'block',
									`${name}: activity visibility follows welcome content width`
								);
								if (width >= 514) checkHeatmap(geometry, name);
								if (baseline) return;
							}
						}
					}
					// A wide window can still have a narrow chat after both real panels open.
					await page.setViewportSize({ width: 1100, height: 840 });
					await page.evaluate(() => {
						window.welcomeResponsiveFixture.scene('workspace');
						window.welcomeResponsiveFixture.panels(true, 400);
					});
					await page.waitForTimeout(450);
					const squeezed = await measure(card);
					report.cases.push({
						name: `${mac}-${zero}-panels-squeezed`,
						...squeezed,
					});
					await page.screenshot({
						path: path.join(output, `${mac}-${zero}-panels-squeezed.png`),
					});
					assert.ok(
						squeezed.viewport >= 1100 && squeezed.container.width < 514
					);
					assert.equal(squeezed.card.display, 'none');
					await checkActions(page, 'workspace');
					await page.evaluate(() =>
						window.welcomeResponsiveFixture.panels(false, 0)
					);
					await page.waitForTimeout(450);
					const restored = await measure(card);
					report.cases.push({
						name: `${mac}-${zero}-panels-restored`,
						...restored,
					});
					assert.equal(restored.card.display, 'block');
					checkHeatmap(restored, 'panels restored');

					await page.evaluate(() =>
						window.welcomeResponsiveFixture.scene('office')
					);
					await page
						.getByRole('button', { name: /Fixture report.pdf/ })
						.waitFor();
					await page.setViewportSize({ width: 480, height: 840 });
					await checkActions(page, 'workspace');
					assert.equal(await title.count(), 0);
					await page
						.getByRole('button', { name: /Fixture report.pdf/ })
						.click();
					await page.screenshot({
						path: path.join(output, `${mac}-${zero}-office-narrow.png`),
					});
					report.cases.push({
						name: `${mac}-${zero}-office-narrow`,
						artifactsVisible: true,
					});

					await page.evaluate(() =>
						window.welcomeResponsiveFixture.scene('greeting')
					);
					await page.locator('.stream-scroll').waitFor();
					assert.equal(await title.count(), 0);
					await page.getByText(/，今天想做点什么？/).waitFor();
					assert.ok(await page.locator('textarea').first().isVisible());
					report.cases.push({
						name: `${mac}-${zero}-existing-session`,
						greetingVisible: true,
					});

					if (!zero) {
						await page.setViewportSize({ width: 840, height: 840 });
						await page.evaluate(() => window.settingsShellFixture.open());
						await page
							.getByRole('button', { name: '统计', exact: true })
							.click();
						const heatmap = page.locator('.min-w-\\[480px\\]');
						await heatmap.waitFor();
						await heatmap.scrollIntoViewIfNeeded();
						const settings = await heatmap.evaluate((el) => {
							const parent = el.parentElement;
							parent.scrollLeft = parent.scrollWidth;
							return {
								width: el.getBoundingClientRect().width,
								display: getComputedStyle(el).display,
								overflow: getComputedStyle(parent).overflowX,
								viewport: parent.clientWidth,
								scrollWidth: parent.scrollWidth,
								scrollLeft: parent.scrollLeft,
							};
						});
						assert.ok(settings.width >= 480);
						assert.equal(settings.display, 'block');
						assert.equal(settings.overflow, 'auto');
						assert.ok(
							settings.viewport < settings.scrollWidth &&
								settings.scrollLeft > 0
						);
						report.cases.push({ name: `${mac}-settings-scroll`, ...settings });
						await page.screenshot({
							path: path.join(output, `${mac}-settings-scroll.png`),
						});
					}
				} finally {
					await page.close();
				}
			}
		}
		assert.deepEqual(report.errors, [], 'no browser runtime errors');
	} finally {
		await writeFile(
			path.join(output, 'report.json'),
			JSON.stringify(report, null, 2)
		);
		console.log(
			JSON.stringify({
				output,
				cases: report.cases.length,
				errors: report.errors,
			})
		);
	}
}

async function checkActions(page, scene) {
	const welcome = page.locator('.workspace-welcome-content');
	const cta = welcome.getByRole('button', {
		name: scene === 'unrooted' ? '打开工作区…' : '新建会话',
		exact: true,
	});
	assert.ok(await welcome.getByText(/，今天想做点什么？/).isVisible());
	assert.ok((await cta.isVisible()) && (await cta.isEnabled()));
	const key = scene === 'unrooted' ? 'open' : 'newSession';
	const before = await page.evaluate(
		(key) => window.welcomeResponsiveFixture.clicks()[key],
		key
	);
	await cta.click();
	assert.equal(
		await page.evaluate(
			(key) => window.welcomeResponsiveFixture.clicks()[key],
			key
		),
		before + 1
	);
	if (scene === 'unrooted') {
		const recent = welcome.getByRole('button', { name: /Recent fixture/ });
		assert.ok(await recent.isVisible());
		const beforeRecent = await page.evaluate(
			() => window.welcomeResponsiveFixture.clicks().recent
		);
		await recent.click();
		assert.equal(
			await page.evaluate(
				() => window.welcomeResponsiveFixture.clicks().recent
			),
			beforeRecent + 1
		);
	} else {
		assert.equal(
			await welcome.getByRole('button', { name: /Recent fixture/ }).count(),
			0
		);
	}
}

async function measure(card) {
	return card.evaluate((el) => {
		const rect = (node) => {
			const r = node.getBoundingClientRect();
			return {
				x: r.x,
				y: r.y,
				right: r.right,
				bottom: r.bottom,
				width: r.width,
				height: r.height,
			};
		};
		const container = el.parentElement;
		const css = getComputedStyle(container);
		const labels = [...el.querySelectorAll('span.absolute')].map((node) => ({
			text: node.textContent,
			...rect(node),
		}));
		return {
			viewport: innerWidth,
			container: {
				...rect(container),
				type: css.containerType,
				name: css.containerName,
			},
			card: { ...rect(el), display: getComputedStyle(el).display },
			heading: rect(el.querySelector('header')),
			headingParts: [...el.querySelector('header').children].map(rect),
			labels,
		};
	});
}

function checkHeatmap(geometry, name) {
	assert.ok(
		geometry.heading.width >= 480,
		`${name}: header and heatmap retain usable width`
	);
	assert.ok(
		geometry.headingParts[0].right <= geometry.headingParts[1].x,
		`${name}: title and totals stay separate`
	);
	assert.ok(
		geometry.headingParts.every((part) => part.right <= geometry.heading.right),
		`${name}: header text stays inside`
	);
	assert.ok(geometry.labels.length >= 11, `${name}: month labels are present`);
	for (let i = 0; i < geometry.labels.length; i++) {
		const label = geometry.labels[i];
		assert.ok(
			label.x >= geometry.card.x && label.right <= geometry.card.right,
			`${name}: ${label.text} stays in card`
		);
		if (i)
			assert.ok(
				geometry.labels[i - 1].right <= label.x,
				`${name}: month labels stay separate`
			);
	}
}
