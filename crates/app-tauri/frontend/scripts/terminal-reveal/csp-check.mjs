import assert from 'node:assert/strict';
import path from 'node:path';
import { readFile, mkdir } from 'node:fs/promises';
import { preview } from 'vite';
import { chromium, webkit } from 'playwright';

export async function checkTerminalCsp({ root, outDir, output, engine }) {
	const config = JSON.parse(
		await readFile(path.join(root, '../tauri.conf.json'), 'utf8')
	);
	const security = config.app.security;
	const disabled = security.dangerousDisableAssetCspModification;
	assert.notEqual(disabled, true, 'Asset CSP modification must remain enabled');
	assert.ok(
		!Array.isArray(disabled) || !disabled.includes('script-src'),
		'Script nonce/hash hardening must remain enabled'
	);
	const modifiesStyles =
		!Array.isArray(disabled) || !disabled.includes('style-src');
	const withStyleNonce = (csp) =>
		csp.replace(/style-src([^;]*)/, "$& 'nonce-husk-style-regression'");
	const productionPolicy = modifiesStyles
		? withStyleNonce(security.csp)
		: security.csp;
	const server = await preview({
		configFile: false,
		root,
		build: { outDir },
		plugins: [
			{
				name: 'terminal-native-csp',
				configurePreviewServer(vite) {
					vite.middlewares.use((req, res, next) => {
						const policy = new URL(
							req.url,
							'http://localhost'
						).searchParams.get('policy');
						if (policy !== 'development') {
							res.setHeader(
								'Content-Security-Policy',
								policy === 'blocked-control'
									? withStyleNonce(security.csp)
									: productionPolicy
							);
						}
						next();
					});
				},
			},
		],
		preview: { host: '127.0.0.1', port: 0 },
	});
	const browser = await (engine === 'webkit' ? webkit : chromium).launch({
		headless: true,
		...(engine === 'chromium' && process.env.CHAT_TERMINAL_EXECUTABLE
			? { executablePath: process.env.CHAT_TERMINAL_EXECUTABLE }
			: {}),
	});
	const cases = [
		{ policy: 'development', dark: false },
		{ policy: 'production', dark: false },
		{ policy: 'production', dark: true },
		{ policy: 'blocked-control', dark: false },
	];
	const results = [];
	await mkdir(output, { recursive: true });
	try {
		for (const test of cases) {
			const page = await browser.newPage({
				viewport: { width: test.dark ? 640 : 1100, height: 600 },
			});
			const errors = [];
			page.on('pageerror', (error) => errors.push(error.message));
			await page.addInitScript(() => {
				window.__terminalCspViolations = [];
				document.addEventListener('securitypolicyviolation', (event) => {
					window.__terminalCspViolations.push(event.effectiveDirective);
				});
			});
			await page.goto(
				`${server.resolvedUrls.local[0]}scripts/terminal-reveal/index.html?policy=${test.policy}`
			);
			await page.waitForFunction(() => Boolean(window.__terminalTestSession));
			await page.evaluate(({ dark }) => {
				document.documentElement.classList.toggle('dark', dark);
				const host = document.createElement('div');
				host.id = 'terminal-csp-probe';
				Object.assign(host.style, {
					position: 'fixed',
					inset: '20px',
					zIndex: '9999',
					background: dark ? '#141414' : '#ffffff',
				});
				document.body.append(host);
				window.__terminalCspSession = new window.__terminalTestSession(
					host,
					'/repo',
					(state) => (window.__terminalCspState = state),
					{
						theme: {
							background: dark ? '#141414' : '#ffffff',
							foreground: dark ? '#eeeeee' : '#101010',
							green: dark ? '#22c55e' : '#138844',
						},
						fontFamily: dark
							? '"JetBrains Mono", monospace'
							: '"Husk Terminal Mono", monospace',
						fontSize: dark ? 18 : 14,
						lineHeight: 1.2,
						cursorStyle: 'block',
						cursorBlink: false,
						scrollback: 100,
					}
				);
				window.__terminalCspSession.setVisible(true);
			}, test);
			await page.waitForFunction(
				() =>
					window.__terminalCspState?.status === 'running' &&
					document
						.querySelector('#terminal-csp-probe .xterm-rows')
						?.textContent.includes('husk on main')
			);
			await page.evaluate(() => document.fonts.ready);
			const metrics = await page.evaluate(({ dark }) => {
				const rows = document.querySelector('#terminal-csp-probe .xterm-rows');
				const cell = rows.querySelector('span');
				const family = dark ? 'JetBrains Mono' : 'Husk Terminal Mono';
				return {
					text: rows.textContent,
					font: getComputedStyle(rows).fontFamily,
					whiteSpace: getComputedStyle(rows).whiteSpace,
					color: getComputedStyle(cell).color,
					cellDisplay: getComputedStyle(cell).display,
					fontLoaded: [...document.fonts].some(
						(font) =>
							font.family.replace(/["']/g, '') === family &&
							font.status === 'loaded'
					),
					violations: window.__terminalCspViolations,
				};
			}, test);
			const name = `${engine}-${test.policy}-${test.dark ? 'dark' : 'light'}`;
			await page.screenshot({ path: path.join(output, `${name}.png`) });
			results.push({ name, ...metrics, errors });
			assert.deepEqual(errors, [], `${name}: JavaScript errors`);
			assert.ok(
				metrics.text.includes('husk on main'),
				`${name}: ANSI text ${JSON.stringify(metrics.text)}`
			);
			if (test.policy === 'blocked-control') {
				assert.ok(
					metrics.violations.some((d) => d.startsWith('style-src')),
					`${name}: negative control must block dynamic styles`
				);
				assert.notEqual(metrics.whiteSpace, 'pre');
			} else {
				assert.ok(
					metrics.font.includes(
						test.dark ? 'JetBrains Mono' : 'Husk Terminal Mono'
					),
					`${name}: configured terminal font`
				);
				assert.equal(
					metrics.whiteSpace,
					'pre',
					`${name}: preserve terminal columns`
				);
				assert.equal(
					metrics.color,
					test.dark ? 'rgb(34, 197, 94)' : 'rgb(19, 136, 68)',
					`${name}: ANSI palette`
				);
				assert.equal(
					metrics.cellDisplay,
					'inline-block',
					`${name}: glyph grid`
				);
				assert.equal(metrics.fontLoaded, true, `${name}: terminal font loaded`);
				assert.deepEqual(
					metrics.violations.filter((d) => d.startsWith('style-src')),
					[],
					`${name}: renderer style CSP violations`
				);
			}
			await page.evaluate(() => window.__terminalCspSession.dispose());
			await page.close();
		}
		return { engine, output, results, ipc: 'mocked', bundle: 'production' };
	} finally {
		await browser.close();
		await new Promise((resolve, reject) =>
			server.httpServer.close((error) => (error ? reject(error) : resolve()))
		);
	}
}
