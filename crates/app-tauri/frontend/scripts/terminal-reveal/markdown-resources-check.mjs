import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';

export async function checkMarkdownResources({ browser, base, root }) {
	const parent =
		process.env.MARKDOWN_REMOTE_EVIDENCE ??
		path.resolve(
			root,
			'../../../.husk/scratch/workspace-markdown-image-stability'
		);
	await mkdir(parent, { recursive: true });
	const output = await mkdtemp(path.join(parent, 'browser-'));
	console.log(`Markdown resource evidence: ${output}`);
	const page = await browser.newPage({
		viewport: { width: 1280, height: 840 },
	});
	page.setDefaultTimeout(10000);
	const errors = [],
		externalRequests = [],
		checks = [];
	page.on('pageerror', (error) => errors.push(error.message));
	await page.route('**/*', (route) => {
		const url = route.request().url();
		try {
			if (url === 'https://example.com/external.png')
				return route.fulfill({
					status: 404,
					contentType: 'text/plain',
					body: 'Controlled image failure',
				});
			if (
				/^https?:/.test(url) &&
				new URL(url).origin !== new URL(base).origin
			) {
				externalRequests.push(url);
				return route.abort();
			}
			return route.continue();
		} catch (error) {
			errors.push(String(error));
			return route.abort();
		}
	});
	const markdown = () => page.getByLabel('Markdown 预览', { exact: true });
	const treeFile = async (name) => {
		if (
			await page
				.getByRole('button', { name: '返回文件列表', exact: true })
				.count()
		) {
			await page
				.getByRole('button', { name: '返回文件列表', exact: true })
				.click();
		}
		const docs = page.getByRole('button', { name: 'docs', exact: true });
		await docs.waitFor();
		if ((await docs.getAttribute('aria-expanded')) !== 'true')
			await docs.click();
		await page.getByRole('button', { name, exact: true }).click();
		await markdown().waitFor();
		assert.equal(
			await page
				.getByRole('tab', { name: '预览', exact: true })
				.getAttribute('aria-selected'),
			'true'
		);
	};
	const images = async (scope = markdown()) => {
		for (const name of ['PNG alt', 'JPEG alt', 'GIF alt', 'WebP alt']) {
			const img = scope.getByRole('img', { name, exact: true });
			await img.waitFor();
			await page.waitForFunction(
				(element) => element.complete && element.naturalWidth > 0,
				await img.elementHandle()
			);
			const geometry = await img.evaluate((element) => ({
				width: element.naturalWidth,
				src: element.src.slice(0, 30),
				alt: element.alt,
			}));
			assert.ok(geometry.width > 0);
			assert.ok(geometry.src.startsWith('data:image/'));
			checks.push({ image: name, ...geometry });
		}
		await scope
			.getByRole('alert')
			.filter({ hasText: 'Fixture image not found' })
			.waitFor();
		await scope
			.getByRole('alert')
			.filter({ hasText: '不支持的图片格式' })
			.waitFor();
		await scope.getByRole('alert').filter({ hasText: '图片过大' }).waitFor();
		assert.equal(
			await scope.getByRole('link', { name: 'escape', exact: true }).count(),
			0
		);
		assert.equal(
			await scope.getByRole('link', { name: 'unsafe', exact: true }).count(),
			0
		);
		await scope
			.getByRole('alert')
			.filter({ hasText: '远程图片加载失败' })
			.waitFor();
		assert.equal(
			await scope
				.locator(
					'img[src^="http"], img[src^="file:"], script, svg, object, embed'
				)
				.count(),
			0
		);
		assert.equal(await page.evaluate(() => window.markdownExecuted), undefined);
	};
	const stability = async (scope, label) => {
		const img = scope.getByRole('img', { name: 'PNG alt', exact: true });
		await img.scrollIntoViewIfNeeded();
		const handle = await img.elementHandle();
		await handle.evaluate((element) => {
			const container = element.closest('[aria-label="Markdown 预览"]');
			const state = {
				image: element,
				samples: [],
				replacements: 0,
				missing: 0,
			};
			const sample = () => {
				const following = [...container.querySelectorAll('span')].find((node) =>
					node.textContent.startsWith('图片：external alt')
				);
				state.samples.push(following.getBoundingClientRect().top);
				if (!element.isConnected || !element.complete) state.missing++;
			};
			state.observer = new MutationObserver(() => {
				if (!element.isConnected) state.replacements++;
				sample();
			});
			state.observer.observe(container, { childList: true, subtree: true });
			sample();
			state.timer = setInterval(sample, 16);
			window.markdownStability = state;
		});
		const reads = await page.evaluate(
			() =>
				window.workspaceFileCalls.filter((call) =>
					call.path?.startsWith('assets/')
				).length
		);
		for (let i = 0; i < 16; i++) {
			await page.evaluate(() => window.workspaceLinksFixture.update());
			await page.waitForTimeout(200);
		}
		const result = await page.evaluate(() => {
			const state = window.markdownStability;
			clearInterval(state.timer);
			state.observer.disconnect();
			return {
				sameDOM: state.image.isConnected,
				replacements: state.replacements,
				missingSamples: state.missing,
				sampleCount: state.samples.length,
				textTopRange: Math.max(...state.samples) - Math.min(...state.samples),
				reads: window.workspaceFileCalls.filter((call) =>
					call.path?.startsWith('assets/')
				).length,
			};
		});
		result.extraImageIPC = result.reads - reads;
		checks.push({ stability: label, ...result });
		await writeFile(
			path.join(output, `${label}-stability.json`),
			JSON.stringify(result, null, 2)
		);
		await page.screenshot({
			path: path.join(output, `${label}-stability.png`),
		});
		if (!process.env.MARKDOWN_STABILITY_BASELINE) {
			assert.equal(result.sameDOM, true);
			assert.equal(result.replacements, 0);
			assert.equal(result.missingSamples, 0);
			assert.equal(result.extraImageIPC, 0);
			assert.ok(result.textTopRange <= 1, JSON.stringify(result));
		}
	};
	try {
		await page.goto(`${base}terminal-reveal.html?markdown-resources`);
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
				await treeFile('start.md');
				await images();
				await stability(
					markdown(),
					`${theme}-${office ? 'office' : 'programming'}-small`
				);
				assert.equal(
					await markdown()
						.getByRole('link', { name: 'sibling', exact: true })
						.getAttribute('href'),
					'/repo/docs/guide.md#L2C3'
				);
				await markdown().evaluate((element) => {
					element.scrollTop = 0;
				});
				await page.screenshot({
					path: path.join(
						output,
						`${theme}-${office ? 'office' : 'programming'}-preview.png`
					),
				});
				await page
					.getByRole('button', { name: '展开预览', exact: true })
					.click();
				const dialog = page.getByRole('dialog');
				await images(dialog.getByLabel('Markdown 预览', { exact: true }));
				await stability(
					dialog.getByLabel('Markdown 预览', { exact: true }),
					`${theme}-${office ? 'office' : 'programming'}-expanded`
				);
				await page.screenshot({
					path: path.join(
						output,
						`${theme}-${office ? 'office' : 'programming'}-expanded.png`
					),
				});
				await dialog
					.getByRole('link', { name: 'sibling', exact: true })
					.click();
				await page
					.getByRole('status', { name: '文件定位' })
					.filter({ hasText: '第 2 行，第 3 列' })
					.waitFor();
				assert.equal(await page.getByRole('dialog').count(), 0);
				assert.equal(
					await page
						.getByRole('tab', { name: '源码', exact: true })
						.getAttribute('aria-selected'),
					'true'
				);
				await page
					.locator(
						'[data-line="2"][aria-current="location"] [data-source-column]'
					)
					.waitFor();
				await treeFile('start.md');
				await markdown()
					.getByRole('link', { name: 'parent', exact: true })
					.click();
				await page
					.getByLabel('文件路径', { exact: true })
					.filter({ hasText: 'README.md' })
					.waitFor();
				checks.push({
					theme,
					office,
					preview: true,
					expanded: true,
					sibling: true,
					parent: true,
				});
			}
		}
		for (const [name, expected] of [
			['spaced', 'a b.md'],
			['encoded', 'a b.md'],
			['absolute', 'guide.md'],
			['file URL', 'guide.md'],
		]) {
			await treeFile('start.md');
			await markdown().getByRole('link', { name, exact: true }).click();
			await page
				.getByLabel('文件路径', { exact: true })
				.filter({ hasText: expected })
				.waitFor();
		}
		for (const [name, expected] of [
			['heading', 'start.md'],
			['sibling heading', 'guide.md'],
		]) {
			await treeFile('start.md');
			assert.equal(
				await markdown()
					.getByText(/本地资源不支持查询参数/)
					.count(),
				2
			);
			await markdown().getByRole('link', { name, exact: true }).click();
			await page
				.getByLabel('文件路径', { exact: true })
				.filter({ hasText: expected })
				.waitFor();
			await page
				.getByRole('status', { name: '文件定位' })
				.filter({ hasText: '暂不支持标题锚点定位' })
				.waitFor();
			checks.push({
				fragment: name,
				opened: expected,
				headingPosition: 'explicitly unsupported',
			});
		}
		await treeFile('start.md');
		await markdown()
			.getByRole('link', { name: 'local line', exact: true })
			.click();
		await page
			.getByRole('status', { name: '文件定位' })
			.filter({ hasText: '第 1 行，第 2 列' })
			.waitFor();
		await treeFile('slow.md');
		await page.waitForFunction(() =>
			window.workspaceFileCalls.some((call) => call.path === 'assets/slow.png')
		);
		await treeFile('other.md');
		await page.waitForTimeout(650);
		await markdown()
			.getByRole('img', { name: 'other alt', exact: true })
			.waitFor();
		assert.equal(
			await page.getByRole('img', { name: 'slow alt', exact: true }).count(),
			0
		);
		await treeFile('slow.md');
		await page.waitForFunction(
			() =>
				window.workspaceFileCalls.filter(
					(call) => call.path === 'assets/slow.png'
				).length >= 2
		);
		await page.evaluate(() => window.workspaceLinksFixture.switch('/other', 1));
		await page.getByRole('button', { name: 'docs', exact: true }).waitFor();
		await treeFile('other.md');
		await page.waitForTimeout(650);
		const other = markdown().getByRole('img', {
			name: 'other alt',
			exact: true,
		});
		await page.waitForFunction(
			(element) => element.complete && element.naturalWidth > 0,
			await other.elementHandle()
		);
		assert.equal(
			await page.getByRole('img', { name: 'slow alt', exact: true }).count(),
			0
		);
		const calls = await page.evaluate(() => window.workspaceFileCalls);
		assert.ok(
			calls.every(
				(call) =>
					['/repo', '/other'].includes(call.root) &&
					!call.path.startsWith('/') &&
					!call.path.split('/').includes('..')
			)
		);
		assert.ok(
			!calls.some((call) =>
				/outside|escape|external|javascript|data:/.test(call.path)
			)
		);
		assert.deepEqual(externalRequests, []);
		assert.deepEqual(errors, []);
		await writeFile(
			path.join(output, 'result.json'),
			JSON.stringify(
				{ passed: true, checks, calls, externalRequests, errors },
				null,
				2
			)
		);
	} catch (error) {
		await page.screenshot({ path: path.join(output, 'first-failure.png') });
		await writeFile(
			path.join(output, 'failure.json'),
			JSON.stringify(
				{ error: String(error), checks, externalRequests, errors },
				null,
				2
			)
		);
		throw error;
	} finally {
		await page.close();
	}
}
