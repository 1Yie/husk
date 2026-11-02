import { act, fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithProviders as render } from '@/test/render';
import type { FilePreview } from '../types';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
import { WorkspaceMarkdown } from '../markdown-resources';

beforeEach(() => {
	invoke.mockReset();
});

describe('workspace Markdown resources', () => {
	it('renders HTTPS badges with safe dimensions and keeps unsafe sources out of img', async () => {
		const src =
			'https://img.shields.io/badge/rust-stable-dea584?style=flat-square&logo=rust&logoColor=white';
		const { rerender } = render(
			<WorkspaceMarkdown
				root="/repo"
				documentPath="README.md"
				onNavigate={vi.fn()}
				text={[
					`![rust](${src} "Rust badge")`,
					'<img src="https://example.com/logo.png" alt="Husk" width="112" height="56" />',
					'![HTTP](http://example.com/image.png)',
					'![credentials](https://user:secret@example.com/private.png)',
					'![unsafe](javascript:alert%281%29)',
					'![data](data:image/svg+xml;base64,AA==)',
				].join('\n\n')}
			/>
		);
		const badge = screen.getByRole('img', { name: 'rust' });
		expect(badge).toHaveAttribute('src', src);
		expect(badge).toHaveAttribute('title', 'Rust badge');
		expect(badge).toHaveAttribute('referrerpolicy', 'no-referrer');
		const logo = screen.getByRole('img', { name: 'Husk' });
		expect(logo).toHaveAttribute('width', '112');
		expect(logo).toHaveAttribute('height', '56');
		expect(screen.getAllByRole('img')).toHaveLength(2);
		expect(invoke).not.toHaveBeenCalled();
		for (let i = 0; i < 16; i++) {
			await act(async () =>
				rerender(
					<WorkspaceMarkdown
						root="/repo"
						documentPath="README.md"
						onNavigate={vi.fn()}
						text={`![rust](${src} "Rust badge")`}
					/>
				)
			);
		}
		expect(screen.getByRole('img', { name: 'rust' })).toBe(badge);
		fireEvent.error(badge);
		expect(screen.getByRole('alert')).toHaveTextContent('远程图片加载失败');
		rerender(
			<WorkspaceMarkdown
				root="/repo"
				documentPath="README.md"
				onNavigate={vi.fn()}
				text={`![rust](${src}&label=new)`}
			/>
		);
		expect(screen.getByRole('img', { name: 'rust' })).toHaveAttribute(
			'src',
			`${src}&label=new`
		);
	});

	it('retains a loaded image across new navigation callbacks and invokes the latest callback', async () => {
		invoke.mockResolvedValue({
			path: 'picture.png',
			name: 'picture.png',
			size: 67,
			kind: 'image',
			mime: 'image/png',
			data: 'AA==',
		});
		const props = {
			root: '/repo',
			documentPath: 'README.md',
			text: '![picture](picture.png)\n\nFollowing text\n\n[next](next.md)',
		};
		const first = vi.fn();
		const { rerender } = render(
			<WorkspaceMarkdown {...props} onNavigate={first} />
		);
		const image = await screen.findByRole('img', { name: 'picture' });
		const latest = vi.fn();
		for (let i = 0; i < 8; i++) {
			await act(async () =>
				rerender(
					<WorkspaceMarkdown
						{...props}
						onNavigate={i === 7 ? latest : vi.fn()}
					/>
				)
			);
		}
		expect(screen.getByRole('img', { name: 'picture' })).toBe(image);
		expect(invoke).toHaveBeenCalledTimes(1);
		expect(screen.queryByText(/正在读取图片/)).toBeNull();
		fireEvent.click(screen.getByRole('link', { name: 'next' }));
		expect(latest).toHaveBeenCalledWith({ path: 'next.md' });
		expect(first).not.toHaveBeenCalled();
	});

	it('keeps ordinary fragments and numeric locations while explaining unsupported queries', () => {
		const navigate = vi.fn();
		render(
			<WorkspaceMarkdown
				root="/repo"
				documentPath="docs/start.md"
				onNavigate={navigate}
				text={[
					'[heading](#heading)',
					'[guide heading](guide.md#hello%20world)',
					'[line](#L2C3)',
					'[query](guide.md?download=1)',
					'![query image](../assets/picture.png?raw=1)',
				].join('\n\n')}
			/>
		);
		fireEvent.click(screen.getByRole('link', { name: 'heading' }));
		expect(navigate).toHaveBeenLastCalledWith({
			path: 'docs/start.md',
			fragment: 'heading',
		});
		fireEvent.click(screen.getByRole('link', { name: 'guide heading' }));
		expect(navigate).toHaveBeenLastCalledWith({
			path: 'docs/guide.md',
			fragment: 'hello world',
		});
		fireEvent.click(screen.getByRole('link', { name: 'line' }));
		expect(navigate).toHaveBeenLastCalledWith({
			path: 'docs/start.md',
			line: 2,
			column: 3,
		});
		expect(screen.getAllByText(/本地资源不支持查询参数/)).toHaveLength(2);
		expect(screen.queryByRole('link', { name: 'query' })).toBeNull();
		expect(invoke).not.toHaveBeenCalled();
	});

	it('keeps unsafe and remote resources out of the local reader and exposes reader failures', async () => {
		invoke.mockImplementation((_command, args) => {
			if (args.path === 'assets/missing.png')
				return Promise.reject(new Error('Fixture image not found'));
			return Promise.resolve({
				path: args.path,
				name: args.path,
				size: 9_000_000,
				kind: args.path === 'assets/large.png' ? 'too_large' : 'text',
				text: '<svg/>',
			});
		});
		const { container } = render(
			<WorkspaceMarkdown
				root="/repo"
				documentPath="docs/start.md"
				onNavigate={vi.fn()}
				text={[
					'[escape](../../repo/secret.md)',
					'[unsafe](javascript:alert%281%29)',
					'[outside](file:///outside/secret.md)',
					'![escape](../../repo/secret.png)',
					'![external](https://example.com/tracker.png)',
					'![data](data:image/svg+xml;base64,AA==)',
					'![missing](../assets/missing.png)',
					'![unsupported](../assets/unsafe.svg)',
					'![large](../assets/large.png)',
					"<script>alert('never')</script>",
				].join('\n\n')}
			/>
		);
		expect(await screen.findByText(/Fixture image not found/)).toBeVisible();
		expect(await screen.findByText(/不支持的图片格式/)).toBeVisible();
		expect(await screen.findByText(/图片过大/)).toBeVisible();
		expect(screen.queryByRole('link')).toBeNull();
		expect(screen.getByRole('img', { name: 'external' })).toHaveAttribute(
			'src',
			'https://example.com/tracker.png'
		);
		expect(container.querySelectorAll('img')).toHaveLength(1);
		expect(container.querySelector('script, svg, object, embed')).toBeNull();
		expect(invoke.mock.calls.map(([, args]) => args.path).sort()).toEqual([
			'assets/large.png',
			'assets/missing.png',
			'assets/unsafe.svg',
		]);
	});

	it('separates processor bases and roots and drops an old pending image after switching', async () => {
		let resolve!: (file: FilePreview) => void;
		const pending = new Promise<FilePreview>((yes) => {
			resolve = yes;
		});
		invoke.mockImplementation((_command, args) =>
			args.path === 'docs/picture.png'
				? pending
				: Promise.resolve({
						path: args.path,
						name: 'picture.png',
						size: 67,
						kind: 'image',
						mime: 'image/png',
						data: args.root === '/repo' ? 'NEW' : 'OTHER',
					})
		);
		const props = { text: '![picture](picture.png)', onNavigate: vi.fn() };
		const { rerender } = render(
			<WorkspaceMarkdown {...props} root="/repo" documentPath="docs/start.md" />
		);
		expect(invoke).toHaveBeenCalledWith('workspace_preview_file', {
			root: '/repo',
			path: 'docs/picture.png',
		});
		rerender(
			<WorkspaceMarkdown
				{...props}
				root="/repo"
				documentPath="nested/start.md"
			/>
		);
		expect(await screen.findByRole('img', { name: 'picture' })).toHaveAttribute(
			'src',
			'data:image/png;base64,NEW'
		);
		rerender(
			<WorkspaceMarkdown
				{...props}
				root="/other"
				documentPath="nested/start.md"
			/>
		);
		expect(await screen.findByRole('img', { name: 'picture' })).toHaveAttribute(
			'src',
			'data:image/png;base64,OTHER'
		);
		await act(async () =>
			resolve({
				path: 'docs/picture.png',
				name: 'picture.png',
				size: 67,
				kind: 'image',
				mime: 'image/png',
				data: 'STALE',
			})
		);
		expect(screen.getByRole('img', { name: 'picture' })).toHaveAttribute(
			'src',
			'data:image/png;base64,OTHER'
		);
		expect(invoke).toHaveBeenCalledWith('workspace_preview_file', {
			root: '/other',
			path: 'nested/picture.png',
		});
	});
});
