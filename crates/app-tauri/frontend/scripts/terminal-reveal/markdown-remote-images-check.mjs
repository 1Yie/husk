import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';

const urls = [
	'https://img.shields.io/github/v/tag/1Yie/husk?style=flat-square&label=version&color=2f6feb&sort=semver',
	'https://img.shields.io/badge/rust-stable-dea584?style=flat-square&logo=rust&logoColor=white',
	'https://img.shields.io/badge/tauri-2-24c8db?style=flat-square&logo=tauri&logoColor=white',
	'https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-6e7681?style=flat-square',
	'https://img.shields.io/badge/license-MIT-3fb950?style=flat-square',
];
const names = ['version', 'rust', 'tauri', 'platform', 'license'];
const failedURL = 'https://example.com/external.png';

export async function checkRemoteMarkdownImages({ browser, base, root }) {
	const parent =
		process.env.MARKDOWN_REMOTE_EVIDENCE ??
		path.resolve(
			root,
			'../../../.husk/scratch/workspace-markdown-remote-images'
		);
	await mkdir(parent, { recursive: true });
	const output = await mkdtemp(path.join(parent, 'browser-'));
	console.log(`Controlled SVG evidence: ${output}`);
	const page = await browser.newPage({
		viewport: { width: 1280, height: 840 },
	});
	page.setDefaultTimeout(10000);
	const requests = [],
		unexpected = [],
		errors = [],
		checks = [];
	page.on('pageerror', (error) => errors.push(error.message));
	await page.route('**/*', async (route) => {
		try {
			const request = route.request(),
				url = request.url();
			if (new URL(url).origin === new URL(base).origin) return route.continue();
			if (urls.includes(url) || url === failedURL) {
				requests.push({
					url,
					type: request.resourceType(),
					headers: await request.allHeaders(),
				});
				if (url === failedURL)
					return route.fulfill({
						status: 404,
						contentType: 'text/plain',
						body: 'Controlled image failure',
					});
				return route.fulfill({
					status: 200,
					contentType: 'image/svg+xml',
					headers: { 'cache-control': 'no-store' },
					body: '<svg xmlns="http://www.w3.org/2000/svg" width="110" height="20" onload="fetch(&quot;https://example.invalid/svg-script-executed&quot;);parent.markdownExecuted=true"><script>fetch("https://example.invalid/svg-script-executed");parent.markdownExecuted=true</script><rect width="110" height="20" fill="#2f6feb"/><text x="5" y="14" fill="white">controlled fixture</text></svg>',
				});
			}
			unexpected.push(url);
			return route.abort();
		} catch (error) {
			errors.push(String(error));
			return route.abort();
		}
	});
	const preview = () => page.getByLabel('Markdown 预览', { exact: true });
	const file = async (name) => {
		const back = page.getByRole('button', {
			name: '返回文件列表',
			exact: true,
		});
		if (await back.count()) await back.click();
		const docs = page.getByRole('button', { name: 'docs', exact: true });
		await docs.waitFor();
		if ((await docs.getAttribute('aria-expanded')) !== 'true')
			await docs.click();
		await page.getByRole('button', { name, exact: true }).click();
		await preview().waitFor();
	};
	const images = async (scope) => {
		for (const name of ['Husk', ...names]) {
			const img = scope.getByRole('img', { name, exact: true });
			await img.waitFor();
			await page.waitForFunction(
				(element) => element.complete && element.naturalWidth > 0,
				await img.elementHandle()
			);
		}
		await page.waitForFunction(
			(element) => Math.abs(element.getBoundingClientRect().width - 112) < 0.01,
			await scope
				.getByRole('img', { name: 'Husk', exact: true })
				.elementHandle()
		);
		const dimensions = await scope.locator('img').evaluateAll((elements) =>
			elements.map((element) => {
				const box = element.getBoundingClientRect();
				return {
					alt: element.alt,
					src: element.getAttribute('src'),
					title: element.title,
					width: box.width,
					height: box.height,
					naturalWidth: element.naturalWidth,
					naturalHeight: element.naturalHeight,
					x: box.x,
					y: box.y,
					display: getComputedStyle(element).display,
					referrerPolicy: element.referrerPolicy,
					widthAttribute: element.getAttribute('width'),
					heightAttribute: element.getAttribute('height'),
				};
			})
		);
		for (let i = 0; i < urls.length; i++) {
			const badge = dimensions.find((image) => image.alt === names[i]);
			assert.equal(badge.src, urls[i]);
			assert.equal(badge.referrerPolicy, 'no-referrer');
			assert.equal(badge.display, 'inline');
			assert.ok(
				Math.abs(
					badge.width / badge.height - badge.naturalWidth / badge.naturalHeight
				) < 0.01
			);
		}
		assert.equal(
			dimensions.find((image) => image.alt === 'version').title,
			'Version badge'
		);
		const logo = dimensions.find((image) => image.alt === 'Husk');
		assert.equal(logo.width, 112);
		assert.ok(
			Math.abs(
				logo.width / logo.height - logo.naturalWidth / logo.naturalHeight
			) < 0.01
		);
		const centered = await scope
			.getByRole('img', { name: 'Husk', exact: true })
			.evaluate((element) => {
				const image = element.getBoundingClientRect(),
					parent = element.parentElement.getBoundingClientRect();
				return Math.abs(
					image.x + image.width / 2 - parent.x - parent.width / 2
				);
			});
		assert.ok(centered <= 1, `Logo center offset: ${centered}`);
		const badges = dimensions.filter((image) => names.includes(image.alt));
		assert.ok(
			badges.some(
				(image, i) => i > 0 && Math.abs(image.y - badges[i - 1].y) <= 1
			),
			'Badges share inline rows'
		);
		const attributes = dimensions.find(
			(image) => image.alt === 'unsafe attributes'
		);
		assert.equal(attributes.widthAttribute, null);
		assert.equal(attributes.heightAttribute, null);
		assert.equal(
			await scope.locator('script, svg, object, embed, iframe').count(),
			0
		);
		assert.equal(
			await scope
				.getByRole('img', { name: 'HTTP blocked', exact: true })
				.count(),
			0
		);
		assert.equal(
			await scope
				.getByRole('img', { name: 'credentials blocked', exact: true })
				.count(),
			0
		);
		assert.equal(
			await scope
				.getByRole('img', { name: 'unsafe remote', exact: true })
				.count(),
			0
		);
		assert.equal(await page.evaluate(() => window.markdownExecuted), undefined);
		return dimensions;
	};
	const stability = async (scope, label) => {
		await scope
			.getByRole('img', { name: 'Husk', exact: true })
			.scrollIntoViewIfNeeded();
		await scope.evaluate((container) => {
			const state = {
				images: [...container.querySelectorAll('img')],
				samples: [],
				missing: 0,
				replacements: 0,
			};
			const sample = () => {
				const following = [...container.querySelectorAll('p')].find(
					(element) => element.textContent === 'Following badges'
				);
				state.samples.push(following.getBoundingClientRect().top);
				if (
					state.images.some(
						(element) =>
							!element.isConnected || !element.complete || !element.naturalWidth
					)
				)
					state.missing++;
			};
			state.observer = new MutationObserver(() => {
				if (state.images.some((element) => !element.isConnected))
					state.replacements++;
				sample();
			});
			state.observer.observe(container, { childList: true, subtree: true });
			sample();
			state.timer = setInterval(sample, 16);
			window.remoteStability = state;
		});
		const reads = await page.evaluate(
			() =>
				window.workspaceFileCalls.filter((call) =>
					call.path?.startsWith('assets/')
				).length
		);
		const network = requests.length;
		for (let i = 0; i < 16; i++) {
			await page.evaluate(() => window.workspaceLinksFixture.update());
			await page.waitForTimeout(200);
		}
		const result = await page.evaluate(() => {
			const state = window.remoteStability;
			clearInterval(state.timer);
			state.observer.disconnect();
			return {
				sameDOM: state.images.every((element) => element.isConnected),
				replacements: state.replacements,
				missingSamples: state.missing,
				samples: state.samples.length,
				textTopRange: Math.max(...state.samples) - Math.min(...state.samples),
				reads: window.workspaceFileCalls.filter((call) =>
					call.path?.startsWith('assets/')
				).length,
			};
		});
		result.extraImageIPC = result.reads - reads;
		result.extraNetwork = requests.length - network;
		checks.push({ label, ...result });
		await writeFile(
			path.join(output, `${label}-stability.json`),
			JSON.stringify(result, null, 2)
		);
		await page.screenshot({ path: path.join(output, `${label}.png`) });
		assert.equal(result.sameDOM, true);
		assert.equal(result.replacements, 0);
		assert.equal(result.missingSamples, 0);
		assert.equal(result.extraImageIPC, 0);
		assert.equal(result.extraNetwork, 0);
		assert.ok(result.textTopRange <= 1, JSON.stringify(result));
	};
	try {
		await page.goto(
			`${base}terminal-reveal.html?markdown-resources&remote-badges`
		);
		for (const theme of ['light', 'dark']) {
			await page.evaluate(
				(theme) => window.workspacePanelsFixture.theme(theme),
				theme
			);
			for (const office of [false, true]) {
				await page.evaluate(
					(office) => window.workspaceLinksFixture.office(office),
					office
				);
				await page.setViewportSize({ width: office ? 880 : 1280, height: 840 });
				await file('remote.md');
				const label = `${theme}-${office ? 'office' : 'programming'}`;
				const smallDimensions = await images(preview());
				checks.push({ label, dimensions: smallDimensions });
				await stability(preview(), `${label}-small`);
				await page
					.getByRole('button', { name: '展开预览', exact: true })
					.click();
				const dialog = page.getByRole('dialog', {
					name: 'docs/remote.md',
					exact: true,
				});
				const expanded = dialog.getByLabel('Markdown 预览', { exact: true });
				const expandedDimensions = await images(expanded);
				checks.push({
					label: `${label}-expanded`,
					dimensions: expandedDimensions,
				});
				const rows = (dimensions) =>
					new Set(
						dimensions
							.filter((image) => names.includes(image.alt))
							.map((image) => Math.round(image.y))
					).size;
				assert.ok(rows(smallDimensions) > 1);
				assert.ok(rows(expandedDimensions) <= rows(smallDimensions));
				await stability(expanded, `${label}-expanded`);
				await page.keyboard.press('Escape');
				await dialog.waitFor({ state: 'detached' });
			}
		}
		const narrowLogo = await preview()
			.getByRole('img', { name: 'Husk', exact: true })
			.evaluate((element) => {
				const parent = element.parentElement;
				const previous = parent.style.width;
				parent.style.width = '80px';
				const box = element.getBoundingClientRect();
				parent.style.width = previous;
				return {
					width: box.width,
					height: box.height,
					naturalRatio: element.naturalWidth / element.naturalHeight,
				};
			});
		assert.equal(narrowLogo.width, 80);
		assert.ok(
			Math.abs(narrowLogo.width / narrowLogo.height - narrowLogo.naturalRatio) <
				0.01
		);
		checks.push({ narrowLogo });
		const before = await page.evaluate(() => window.workspaceFileCalls.length);
		await preview()
			.locator('a[href="https://github.com/1Yie/husk/releases"]')
			.click();
		assert.equal(
			await page.evaluate(() => window.workspaceFileCalls.length),
			before
		);
		await file('remote-error.md');
		await preview()
			.getByRole('alert')
			.filter({ hasText: '远程图片加载失败' })
			.waitFor();
		await file('remote.md');
		await images(preview());
		await page.evaluate(() => window.workspaceLinksFixture.switch('/other', 2));
		await file('remote.md');
		await images(preview());
		const calls = await page.evaluate(() => window.workspaceFileCalls);
		assert.ok(
			calls.every(
				(call) => !call.path?.includes('://') && !call.path?.includes('secret')
			)
		);
		assert.ok(
			requests.every(
				(request) => request.type === 'image' && !request.headers.referer
			)
		);
		assert.ok(
			urls.every((url) => requests.some((request) => request.url === url))
		);
		assert.deepEqual(unexpected, []);
		assert.deepEqual(errors, []);
		await writeFile(
			path.join(output, 'result.json'),
			JSON.stringify(
				{
					passed: true,
					controlledSVG: true,
					online: 'UNVERIFIED: prior HTTP 403',
					checks,
					calls,
					requests,
					unexpected,
					errors,
				},
				null,
				2
			)
		);
		console.log(
			JSON.stringify({
				passed: true,
				output,
				states: checks.filter((check) => check.sameDOM).length,
			})
		);
	} catch (error) {
		await writeFile(
			path.join(output, 'failure-dom.html'),
			await preview()
				.evaluate((element) => element.outerHTML)
				.catch(() => '')
		);
		await page.screenshot({ path: path.join(output, 'failure.png') });
		await writeFile(
			path.join(output, 'failure.json'),
			JSON.stringify(
				{ error: String(error), checks, requests, unexpected, errors },
				null,
				2
			)
		);
		throw error;
	} finally {
		await page.close();
	}
}
