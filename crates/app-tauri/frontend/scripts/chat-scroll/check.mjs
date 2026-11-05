// npm run test:scroll                 — Chromium
// npm run test:scroll -- --webkit     — WebKit (Tauri/macOS/Linux)
// First install the chosen browser: npx playwright install chromium webkit.
// Optional: CHAT_SCROLL_EXECUTABLE points at an existing browser executable.
// jsdom cannot detect native scroll anchoring or real mixed-row layout.
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { createServer } from 'vite';
import { chromium, webkit } from 'playwright';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const engine = process.argv.includes('--webkit') ? 'webkit' : 'chromium';
const server = await createServer({
	configFile: false,
	root,
	cacheDir: 'node_modules/.vite-chat-scroll',
	logLevel: 'error',
	plugins: [
		react(),
		{
			name: 'chat-scroll-fixture',
			configureServer(vite) {
				vite.middlewares.use(async (req, res, next) => {
					if (!req.url?.startsWith('/chat-scroll.html')) return next();
					res.setHeader('Content-Type', 'text/html');
					res.end(
						await vite.transformIndexHtml(
							req.url,
							`<html><head></head><body><div id="root"></div><script type="module" src="/scripts/chat-scroll/fixture.tsx"></script></body></html>`
						)
					);
				});
			},
		},
	],
	resolve: {
		alias: [
			{
				find: /^@tauri-apps\/api\/(core|event|window)$/,
				replacement: path.join(here, 'tauri.ts'),
			},
			{ find: '@', replacement: path.join(root, 'src') },
		],
	},
	server: { host: '127.0.0.1', port: 0 },
});

let browser;
try {
	await server.listen();
	browser = await (engine === 'webkit' ? webkit : chromium).launch({
		headless: true,
		...(process.env.CHAT_SCROLL_EXECUTABLE
			? { executablePath: process.env.CHAT_SCROLL_EXECUTABLE }
			: {}),
	});
	const page = await browser.newPage({
		viewport: { width: 1100, height: 720 },
	});
	const pageErrors = [];
	page.on('pageerror', (error) => pageErrors.push(error.message));
	const url = `${server.resolvedUrls.local[0]}chat-scroll.html`;
	await page.goto(`${url}?turns`);
	await page.waitForSelector('[data-step-mounted]');
	await page.waitForTimeout(1500);
	const firstTurn = () =>
		page
			.locator('[id^="chat-turn-t"]:not([id*="-step-"])')
			.first()
			.getAttribute('id');
	const initialTurn = await firstTurn();
	await page.locator('.stream-scroll').hover();
	await page.mouse.wheel(0, -100000);
	await page.waitForTimeout(700);
	const revealedTurn = await firstTurn();
	const earlierReachable = initialTurn !== revealedTurn;
	await page.goto(url);
	await page.waitForSelector('[data-step-mounted]');
	await page.waitForTimeout(1500);
	await page.evaluate(() => {
		const root = document.querySelector('.stream-scroll');
		root.dispatchEvent(new WheelEvent('wheel', { deltaY: -1, bubbles: true }));
		root.scrollTop = root.scrollHeight * 0.65;
	});
	await page.waitForTimeout(1500);
	const expanded = [];
	for (const label of ['思考过程', '次工具调用']) {
		const id = await page.evaluate((label) => {
			return [...document.querySelectorAll('[data-step-mounted]')].find(
				(el) => {
					const rect = el.getBoundingClientRect();
					const button = el.querySelector('button[aria-expanded="false"]');
					return (
						rect.top > 0 &&
						rect.bottom < innerHeight &&
						button?.textContent.includes(label)
					);
				}
			)?.id;
		}, label);
		assert.ok(id, `a visible ${label} row is available`);
		const row = page.locator(`[id="${id}"] button[aria-expanded]`);
		await row.click();
		await page.waitForTimeout(350);
		const geometry = () =>
			page.evaluate((id) => {
				const root = document.querySelector('.stream-scroll');
				const el = document.getElementById(id);
				return {
					top: root.scrollTop,
					height: el.getBoundingClientRect().height,
					nextTop: el.nextElementSibling.getBoundingClientRect().top,
					open: el
						.querySelector('button[aria-expanded]')
						?.getAttribute('aria-expanded'),
				};
			}, id);
		const before = await geometry();
		await page.evaluate(() => {
			const root = document.querySelector('.stream-scroll');
			root.dispatchEvent(
				new WheelEvent('wheel', { deltaY: 30000, bubbles: true })
			);
			root.scrollTop = root.scrollHeight;
		});
		await page.waitForTimeout(1600);
		assert.equal(
			await page.locator(`[id="${id}"][data-step-placeholder]`).count(),
			1,
			'expanded row must actually leave the virtual window'
		);
		await page.evaluate((top) => {
			const root = document.querySelector('.stream-scroll');
			root.dispatchEvent(new WheelEvent('wheel', { deltaY: 1, bubbles: true }));
			root.scrollTop = top;
		}, before.top);
		await page.waitForTimeout(450);
		const after = await geometry();
		expanded.push({ id, before, after });
	}
	console.log(
		JSON.stringify({ engine, initialTurn, revealedTurn, expanded }, null, 2)
	);
	assert.ok(
		earlierReachable,
		'the first real wheel to the top must reveal older loaded turns'
	);
	for (const { before, after } of expanded) {
		assert.equal(
			after.open,
			'true',
			'expanded details must survive virtual unmount'
		);
		assert.ok(
			Math.abs(after.height - before.height) <= 1,
			'expanded height must survive'
		);
		assert.ok(
			Math.abs(after.nextTop - before.nextTop) <= 1,
			'the neighboring row must remain anchored'
		);
	}
	await page.getByTestId('switch-session').click();
	await page.waitForTimeout(1500);
	await page.evaluate((top) => {
		const root = document.querySelector('.stream-scroll');
		root.dispatchEvent(new WheelEvent('wheel', { deltaY: -1, bubbles: true }));
		root.scrollTop = top;
	}, expanded[0].before.top);
	await page.waitForTimeout(450);
	for (const { id } of expanded) {
		assert.equal(
			await page
				.locator(`[id="${id}"] button[aria-expanded]`)
				.getAttribute('aria-expanded'),
			'false',
			'a new session must start collapsed'
		);
	}
	await page.goto(`${server.resolvedUrls.local[0]}chat-scroll.html`);
	await page.waitForSelector('[data-step-mounted]');
	await page.waitForTimeout(1500);
	const result = await page.evaluate(async () => {
		const root = document.querySelector('.stream-scroll');
		const frame = () =>
			new Promise((resolve) => requestAnimationFrame(resolve));
		const count = () => ({
			mounted: document.querySelectorAll('[data-step-mounted]').length,
			placeholders: document.querySelectorAll('[data-step-placeholder]').length,
		});
		const baseline = count();
		const drift = [];
		const frameTimes = [];
		let maxMounted = 0;
		const probe = () => {
			const top = root.getBoundingClientRect().top + 200;
			return [...document.querySelectorAll("[id*='-step-']")].find((el) => {
				const rect = el.getBoundingClientRect();
				return rect.top <= top && rect.bottom > top;
			});
		};
		const wheel = (delta) =>
			root.dispatchEvent(
				new WheelEvent('wheel', { deltaY: delta, bubbles: true })
			);
		wheel(-10);
		root.scrollTop = root.scrollHeight * 0.65;
		await new Promise((resolve) => setTimeout(resolve, 1500));
		// For the SAME anchor node, a 4px scroll must move it by exactly 4px.
		// Any additional displacement is the twitch, even if scrollTop itself
		// legitimately changed to compensate an offscreen height correction.
		for (const delta of [-4, 4]) {
			for (let i = 0; i < 300; i++) {
				const target = probe();
				const before = target?.getBoundingClientRect().top;
				const started = performance.now();
				wheel(delta);
				root.scrollTop += delta;
				await frame();
				await frame();
				const after = target?.getBoundingClientRect().top;
				frameTimes.push(performance.now() - started);
				const extra =
					before != null && after != null ? after - before + delta : 0;
				if (Math.abs(extra) > 1) drift.push({ i, delta, extra, id: target.id });
				maxMounted = Math.max(maxMounted, count().mounted);
			}
		}
		return {
			baseline,
			maxMounted,
			final: count(),
			drift,
			overflowAnchor: getComputedStyle(root).overflowAnchor,
			framesOver70ms: frameTimes.filter((time) => time > 70).length,
		};
	});
	console.log(JSON.stringify({ engine, ...result, pageErrors }, null, 2));
	assert.equal(pageErrors.length, 0, 'browser must have no runtime errors');
	assert.equal(
		result.overflowAnchor,
		'none',
		'manual and native anchors must not compete'
	);
	assert.ok(
		result.baseline.placeholders > 250,
		"fixture must exceed a single long turn's viewport"
	);
	assert.ok(
		result.maxMounted < 100,
		'scrolling must retain real virtualization'
	);
	assert.equal(result.drift.length, 0, 'mixed-row scrolling must not twitch');
} finally {
	await browser?.close();
	await server.close();
}
