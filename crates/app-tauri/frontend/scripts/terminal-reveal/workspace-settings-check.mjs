import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';

export async function checkSettingsShell({ browser, base, engine, mac }) {
	const output = process.env.SETTINGS_SHELL_OUTPUT;
	assert.ok(
		output && path.isAbsolute(output),
		'SETTINGS_SHELL_OUTPUT is an explicit absolute fresh directory'
	);
	await mkdir(output);
	const report = { engine, mac, cases: [], errors: [] };
	const page = await browser.newPage({
		viewport: { width: 1280, height: 840 },
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
		await page.goto(`${base}terminal-reveal.html?settings-shell`);
		await page.locator('.main-layout textarea').first().waitFor();
		await page.locator('.xterm-helper-textarea').waitFor({ state: 'attached' });
		await page.locator('.xterm-rows').waitFor({ state: 'attached' });
		if (mac)
			await page.waitForFunction(() =>
				document.documentElement.hasAttribute('data-sidebar-glass')
			);
		await page.getByRole('separator', { name: '调整会话侧栏宽度' }).focus();
		await page.keyboard.press('ArrowRight');
		await page.waitForTimeout(400);
		await page
			.locator('.main-layout textarea')
			.first()
			.fill('settings lifecycle draft');
		await page.locator('.stream-scroll').hover();
		await page.mouse.wheel(0, -800);
		await page.waitForTimeout(300);
		await page.evaluate(() => {
			const shell = document.querySelector('.main-layout');
			const textarea = shell.querySelector('textarea');
			const stream = shell.querySelector('.stream-scroll');
			window.settingsIdentity = {
				shell,
				textarea,
				stream,
				scroll: stream.scrollTop,
				terminalInput: shell.querySelector('.xterm-helper-textarea'),
				terminalRenderer: shell.querySelector('.xterm-rows'),
				renderer: shell.querySelector('.stream-scroll > *'),
				width: document
					.querySelector('#session-navigation')
					.getBoundingClientRect().width,
				rect: JSON.stringify(shell.getBoundingClientRect()),
				glass: window.settingsShellMetrics.glass.length,
				focused: document.activeElement,
			};
			window.settingsShellFixture.open();
		});
		await page.getByRole('heading', { name: '常规', exact: true }).waitFor();
		await page.getByRole('button', { name: '启动模式', exact: true }).waitFor();
		await page.waitForTimeout(120);
		report.opened = await page.evaluate(() => {
			const before = window.settingsIdentity,
				shell = document.querySelector('.main-layout');
			return {
				rect: JSON.stringify(shell.getBoundingClientRect()),
				beforeRect: before.rect,
				glass: window.settingsShellMetrics.glass.length,
				beforeGlass: before.glass,
				sameFocus: document.activeElement === before.focused,
				scroll: before.stream.scrollTop,
				beforeScroll: before.scroll,
			};
		});
		assert.equal(report.opened.rect, report.opened.beforeRect);
		assert.equal(report.opened.glass, report.opened.beforeGlass);
		assert.equal(report.opened.sameFocus, true);
		assert.equal(report.opened.scroll, report.opened.beforeScroll);
		for (const width of [1280, 1120]) {
			await page.setViewportSize({ width, height: 840 });
			for (const theme of ['light', 'dark', 'custom']) {
				await page.evaluate(
					(theme) => window.workspacePanelsFixture.theme(theme),
					theme
				);
				await page.waitForTimeout(120);
				const geometry = await page.evaluate(() => {
					const root =
						document.querySelector('.settings-layout') ??
						document.querySelector('.fixed.inset-0 > div');
					const aside = root.querySelector('aside');
					const content = root.lastElementChild;
					const top = root.firstElementChild;
					const css = (el) => {
						const s = getComputedStyle(el);
						return {
							bg: s.backgroundColor,
							opacity: s.opacity,
							radius: s.borderRadius,
							border: s.borderRightWidth,
							pointer: s.pointerEvents,
							font: s.fontFamily,
						};
					};
					return {
						root: css(root),
						nav: { ...css(aside), width: aside.getBoundingClientRect().width },
						title: css(aside.firstElementChild.firstElementChild),
						content: {
							...css(content),
							x: content.getBoundingClientRect().x,
							y: content.getBoundingClientRect().y,
							right: innerWidth - content.getBoundingClientRect().right,
							bottom: innerHeight - content.getBoundingClientRect().bottom,
						},
						top: {
							height: top.getBoundingClientRect().height,
							center:
								top.getBoundingClientRect().y +
								top.getBoundingClientRect().height / 2,
							drag: top.getAttribute('data-tauri-drag-region'),
							firstControlTop: content
								.querySelector('button')
								.getBoundingClientRect().top,
						},
						main: css(document.querySelector('.main-layout')),
						native: document.documentElement.hasAttribute('data-sidebar-glass'),
					};
				});
				report.cases.push({ width, theme, geometry });
				await page.screenshot({
					path: path.join(
						output,
						`${engine}-${mac ? 'mac' : 'nonmac'}-${width}-${theme}.png`
					),
				});
				if (!process.argv.includes('--settings-shell-baseline')) {
					assert.equal(geometry.nav.width, 240);
					assert.equal(geometry.top.height, 36);
					assert.equal(geometry.top.center, 18);
					assert.equal(geometry.top.drag, 'deep');
					assert.ok(geometry.top.firstControlTop >= geometry.top.height);
					if (mac) {
						assert.equal(geometry.content.radius, '14px');
						assert.equal(geometry.content.x, 248);
						assert.equal(geometry.content.y, 8);
						assert.equal(geometry.content.right, 8);
						assert.equal(geometry.content.bottom, 8);
						assert.equal(geometry.main.opacity, '0');
						assert.equal(geometry.main.pointer, 'none');
						assert.equal(geometry.nav.bg, 'rgba(0, 0, 0, 0)');
						assert.equal(geometry.title.bg, 'rgba(0, 0, 0, 0)');
						assert.equal(geometry.nav.border, '0px');
						assert.match(geometry.root.bg, /0\.68/);
					} else {
						assert.equal(geometry.content.radius, '0px');
						assert.equal(geometry.content.x, 240);
						assert.equal(geometry.content.y, 0);
						assert.equal(geometry.main.opacity, '1');
						assert.equal(geometry.nav.border, '1px');
					}
				}
			}
		}
		if (process.argv.includes('--settings-shell-baseline')) return;
		// The navigation rail scrolls on its own when the window is shorter than the
		// list; the title strip and the return button stay in place.
		report.navScroll = [];
		const readRail = () =>
			page.evaluate(() => {
				const root = document.querySelector('.settings-layout');
				const aside = root.querySelector('aside');
				const rail = aside.querySelector('[data-settings-nav]');
				const back = aside.querySelector('button');
				const title = aside.firstElementChild.firstElementChild;
				const content = root.lastElementChild;
				const items = [...rail.querySelectorAll('button')];
				const rect = rail.getBoundingClientRect();
				const lastItem = items.at(-1).getBoundingClientRect();
				const itemWidths = items.map(
					(item) => item.getBoundingClientRect().width
				);
				return {
					viewport: innerHeight,
					overflowY: getComputedStyle(rail).overflowY,
					backWidth: back.getBoundingClientRect().width,
					itemWidth: itemWidths.length ? Math.max(...itemWidths) : 0,
					clientHeight: rail.clientHeight,
					scrollHeight: rail.scrollHeight,
					clientWidth: rail.clientWidth,
					scrollWidth: rail.scrollWidth,
					railTop: rect.top,
					railBottom: rect.bottom,
					items: items.length,
					lastTop: lastItem.top,
					lastBottom: lastItem.bottom,
					backTop: back.getBoundingClientRect().top,
					titleTop: title.getBoundingClientRect().top,
					railScrollTop: rail.scrollTop,
					contentScrollTop: content.scrollTop,
				};
			});
		for (const height of [840, 600, 480]) {
			await page.setViewportSize({ width: 1280, height });
			await page.evaluate(() => window.workspacePanelsFixture.theme('light'));
			await page.waitForTimeout(150);
			const before = await readRail();
			assert.equal(before.viewport, height);
			assert.ok(before.items > 0, 'settings navigation renders items');
			assert.equal(before.overflowY, 'auto');
			// A drag region here would turn scrollbar drags into window drags.
			const drag = await page.evaluate(() => {
				const rail = document.querySelector('[data-settings-nav]');
				return {
					rail: rail.getAttribute('data-tauri-drag-region'),
					inner:
						rail.firstElementChild?.getAttribute('data-tauri-drag-region') ??
						null,
				};
			});
			assert.equal(drag.rail, 'false');
			assert.equal(drag.inner, 'deep');
			assert.ok(
				before.scrollWidth <= before.clientWidth + 1,
				'navigation has no horizontal overflow'
			);
			assert.ok(
				Math.abs(before.backWidth - before.itemWidth) <= 1,
				'return button spans the navigation list width'
			);
			if (height === 480)
				assert.ok(
					before.scrollHeight > before.clientHeight + 1,
					'short window overflows the rail'
				);
			if (before.scrollHeight > before.clientHeight + 1) {
				await page.evaluate(() => {
					const rail = document.querySelector('[data-settings-nav]');
					rail.scrollTop = rail.scrollHeight;
				});
				await page.waitForTimeout(120);
				const scrolled = await readRail();
				assert.ok(scrolled.railScrollTop > 0, 'rail scrolls');
				assert.equal(
					scrolled.titleTop,
					before.titleTop,
					'title strip stays put while the rail scrolls'
				);
				assert.equal(
					scrolled.backTop,
					before.backTop,
					'return button stays put while the rail scrolls'
				);
				assert.ok(
					scrolled.lastTop >= scrolled.railTop - 1 &&
						scrolled.lastBottom <= scrolled.railBottom + 1,
					'last navigation item reaches the rail viewport'
				);
			} else {
				assert.ok(
					before.lastTop >= before.railTop - 1 &&
						before.lastBottom <= before.railBottom + 1,
					'every navigation item fits without clipping'
				);
			}
			await page.locator('[data-settings-nav] button').last().focus();
			await page.waitForTimeout(120);
			const focused = await page.evaluate(() => {
				const rail = document.querySelector('[data-settings-nav]');
				const item = rail.querySelector('button:focus');
				const rect = rail.getBoundingClientRect();
				const box = item.getBoundingClientRect();
				return {
					visible: box.top >= rect.top - 1 && box.bottom <= rect.bottom + 1,
				};
			});
			assert.equal(
				focused.visible,
				true,
				'keyboard focus reveals the last navigation item'
			);
			const railAtFocus = await readRail();
			const railScroll = railAtFocus.railScrollTop;
			await page.evaluate(() => {
				document.querySelector('.settings-layout').lastElementChild.scrollTop =
					80;
			});
			await page.waitForTimeout(80);
			const railAfterContentScroll = await readRail();
			assert.equal(
				railAfterContentScroll.railScrollTop,
				railScroll,
				'content pane scrolls independently'
			);
			await page.locator('[data-settings-nav] button').last().click();
			const lastLabel =
				(
					await page.locator('[data-settings-nav] button').last().textContent()
				)?.trim() ?? '';
			assert.ok(lastLabel.length > 0, 'last navigation item has a label');
			await page
				.getByRole('heading', { name: lastLabel, exact: true })
				.waitFor({ timeout: 4000 });
			assert.equal(
				await page
					.getByRole('heading', { name: lastLabel, exact: true })
					.count(),
				1,
				'last navigation item stays clickable'
			);
			report.navScroll.push({ height, before, railScroll });
			await page.screenshot({
				path: path.join(
					output,
					`${engine}-${mac ? 'mac' : 'nonmac'}-navscroll-${height}.png`
				),
			});
		}
		await page.setViewportSize({ width: 1280, height: 840 });
		await page.evaluate(() => window.workspacePanelsFixture.theme('light'));
		await page.waitForTimeout(150);
		await page.getByRole('button', { name: '常规', exact: true }).click();
		await page.getByRole('heading', { name: '常规', exact: true }).waitFor();
		await page.locator('.settings-layout').evaluate((el) => {
			el.lastElementChild.scrollTop = 0;
		});

		// 快捷键 page: shared list rows, in-place capture, and the global listener.
		await page.getByRole('button', { name: '快捷键', exact: true }).click();
		await page.getByRole('heading', { name: '快捷键', exact: true }).waitFor();
		const accelerator = mac ? 'Meta' : 'Control';
		const terminalKey = page.getByLabel('打开／收起终端的当前快捷键');
		const defaultTerminal = (await terminalKey.textContent())?.trim() ?? '';
		assert.ok(defaultTerminal.length > 0, 'terminal row shows its binding');
		assert.equal(
			await page.locator(".settings-layout [class*='divide-y'] > div").count(),
			8,
			'one row per application shortcut'
		);
		await page.getByRole('button', { name: '重新绑定打开／收起终端' }).click();
		assert.equal((await terminalKey.textContent())?.trim(), '按下组合…');
		await page.keyboard.press(`${accelerator}+Shift+K`);
		await page.waitForTimeout(150);
		const rebound = (await terminalKey.textContent())?.trim() ?? '';
		assert.notEqual(rebound, defaultTerminal, 'capture replaces the binding');
		assert.equal(
			await page.evaluate(
				() =>
					JSON.parse(localStorage.getItem('husk.shortcuts.v1') ?? '{}').terminal
			),
			'mod+shift+k',
			'the new binding is stored'
		);
		await page.screenshot({
			path: path.join(
				output,
				`${engine}-${mac ? 'mac' : 'nonmac'}-shortcuts.png`
			),
		});
		await page.getByRole('button', { name: '恢复默认' }).click();
		await page.waitForTimeout(150);
		assert.equal(
			(await terminalKey.textContent())?.trim(),
			defaultTerminal,
			'reset restores the default'
		);
		const terminalOpen = () =>
			page.locator('.terminal-panel').first().getAttribute('data-open');
		const openBefore = await terminalOpen();
		await page.keyboard.press(`${accelerator}+Shift+J`);
		await page.waitForTimeout(300);
		assert.notEqual(
			await terminalOpen(),
			openBefore,
			'the terminal shortcut toggles the panel'
		);
		await page.keyboard.press(`${accelerator}+Shift+J`);
		await page.waitForTimeout(300);
		assert.equal(
			await terminalOpen(),
			openBefore,
			'the same shortcut restores it'
		);
		await page.getByRole('button', { name: '常规', exact: true }).click();
		await page.getByRole('heading', { name: '常规', exact: true }).waitFor();
		await page
			.locator('.settings-layout')
			.locator('button[aria-pressed]')
			.first()
			.click();
		await page.setViewportSize({ width: 1280, height: 840 });
		await page.evaluate(() => window.workspacePanelsFixture.theme('light'));
		await page.waitForTimeout(150);
		await page.evaluate(() => {
			window.settingsIdentity.glass = window.settingsShellMetrics.glass.length;
		});
		await page.getByRole('button', { name: '返回工作区', exact: true }).click();
		await page.waitForTimeout(100);
		const restored = await page.evaluate(() => {
			const before = window.settingsIdentity;
			const shell = document.querySelector('.main-layout');
			return {
				shell: shell === before.shell,
				textarea: shell.querySelector('textarea') === before.textarea,
				stream: shell.querySelector('.stream-scroll') === before.stream,
				renderer: shell.querySelector('.stream-scroll > *') === before.renderer,
				terminalInput:
					shell.querySelector('.xterm-helper-textarea') ===
					before.terminalInput,
				terminalRenderer:
					shell.querySelector('.xterm-rows') === before.terminalRenderer,
				terminalRendererPresent: !!before.terminalRenderer,
				value: before.textarea.value,
				width: document
					.querySelector('#session-navigation')
					.getBoundingClientRect().width,
				beforeWidth: before.width,
				rect: JSON.stringify(shell.getBoundingClientRect()),
				beforeRect: before.rect,
				opacity: getComputedStyle(shell).opacity,
				glass: window.settingsShellMetrics.glass.length,
				beforeGlass: before.glass,
			};
		});
		report.restored = restored;
		for (const field of [
			'shell',
			'textarea',
			'stream',
			'renderer',
			'terminalInput',
			'terminalRenderer',
			'terminalRendererPresent',
		])
			assert.equal(restored[field], true, field);
		assert.equal(restored.value, 'settings lifecycle draft');
		assert.equal(restored.width, restored.beforeWidth);
		assert.equal(restored.rect, restored.beforeRect);
		assert.equal(restored.opacity, '1');
		assert.equal(restored.glass, restored.beforeGlass);
		await page.evaluate(() => window.settingsShellFixture.open());
		await page.getByRole('button', { name: '启动模式', exact: true }).click();
		const menu = page.getByRole('menu');
		await menu.waitFor();
		const item = page.getByRole('menuitemradio', { name: '编程', exact: true });
		await item.hover();
		const menuEvidence = await menu.evaluate((el) => {
			const r = el.getBoundingClientRect(),
				hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
			return {
				z: getComputedStyle(el).zIndex,
				hit: el.contains(hit),
				highlighted: !!el.querySelector('[data-highlighted]'),
			};
		});
		report.menu = menuEvidence;
		assert.equal(menuEvidence.z, '50');
		assert.equal(menuEvidence.hit, true);
		assert.equal(menuEvidence.highlighted, true);
		await page.screenshot({
			path: path.join(output, `${engine}-${mac ? 'mac' : 'nonmac'}-menu.png`),
		});
		await item.click();
		await page.getByRole('button', { name: '启动模式', exact: true }).focus();
		await page.keyboard.press('ArrowDown');
		await menu.waitFor();
		await page.keyboard.press('ArrowDown');
		await page.keyboard.press('Enter');
		await page.getByRole('button', { name: '启动模式', exact: true }).click();
		await menu.waitFor();
		await page.keyboard.press('Escape');
		await page.waitForTimeout(120);
		report.menuEscape = {
			settingsOpen:
				(await page
					.getByRole('heading', { name: '常规', exact: true })
					.count()) > 0,
			menuClosed: (await menu.count()) === 0,
		};
		assert.equal(report.menuEscape.menuClosed, true);
		if (!report.menuEscape.settingsOpen)
			await page.evaluate(() => window.settingsShellFixture.open());
		await page.getByRole('heading', { name: '常规', exact: true }).waitFor();
		await page.evaluate(() => {
			const content =
				document.querySelector('.settings-layout').lastElementChild;
			content.scrollTop = 300;
		});
		assert.ok(
			await page
				.locator('.settings-layout > div')
				.last()
				.evaluate((el) => el.scrollTop > 0)
		);
		await page.getByRole('button', { name: '外观', exact: true }).click();
		await page.getByRole('button', { name: '深色', exact: true }).click();
		await page.waitForFunction(() =>
			document.documentElement.classList.contains('dark')
		);
		await page.waitForTimeout(150);
		const appearance = await page.evaluate(() => ({
			calls: window.settingsShellMetrics.calls.filter(
				(c) =>
					c.args.op === 'set_appearance' || c.args.op === 'set_default_prefs'
			),
			glass: window.settingsShellMetrics.glass,
			maxInFlight: window.settingsShellMetrics.maxInFlight,
			scroll:
				document.querySelector('.settings-layout').lastElementChild.scrollTop,
		}));
		report.appearance = appearance;
		assert.ok(
			appearance.calls.some(
				(c) =>
					c.args.op === 'set_appearance' && c.args.payload.theme_mode === 'dark'
			)
		);
		assert.ok(
			appearance.calls.some(
				(c) =>
					c.args.op === 'set_default_prefs' &&
					c.args.payload.default_workspace_mode === 'build'
			)
		);
		assert.equal(appearance.scroll, 0);
		assert.ok(appearance.glass.every((c) => c.surface !== null));
		assert.ok(appearance.maxInFlight <= 1);
		assert.ok(
			!mac || appearance.glass.at(-1).surface.dark,
			'theme change reaches original owner'
		);
		await page.getByRole('button', { name: '终端', exact: true }).click();
		const size = page.locator('[aria-label="终端字号"]').getByRole('slider');
		await size.focus();
		const initialSize = Number(await size.getAttribute('aria-valuenow'));
		await page.keyboard.press('ArrowRight');
		assert.equal(
			Number(await size.getAttribute('aria-valuenow')),
			initialSize + 1
		);
		report.terminalInput = {
			initialSize,
			changedSize: Number(await size.getAttribute('aria-valuenow')),
		};
		await page.keyboard.press('Escape');
		await page.waitForTimeout(150);
		report.escape = {
			settingsClosed: (await page.locator('.settings-layout').count()) === 0,
		};
		assert.equal(report.escape.settingsClosed, true);
		assert.deepEqual(report.errors, []);
		report.ipc = await page.evaluate(() => window.settingsShellMetrics.calls);
		report.complete = true;
		console.log(JSON.stringify(report, null, 2));
	} finally {
		await writeFile(
			path.join(output, 'report.json'),
			JSON.stringify(report, null, 2)
		);
		await page.close();
	}
}
