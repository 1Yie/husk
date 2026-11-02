// Simulate the desktop boundary, not xterm. Real DOM/canvas/font layout and
// the actual terminal/session code run in the browser.
import type { LinkedRootLayers, ModelInfo } from '@/lib/agent-ipc/sessions';
import type { UiCommand } from '@/types';

export class Channel {
	onmessage = (_event: ArrayBuffer) => {};
}
let fixtureModels: ModelInfo = {
	active_provider: 'fixture',
	active_model: 'fixture',
	models: [],
};
let fixtureLinks: LinkedRootLayers = { workspace: [], session: [] };
let fixturePickedDir: string | null = null;
Object.assign(window, {
	popupBackendFixture: {
		configure: (data: {
			models?: ModelInfo;
			links?: LinkedRootLayers;
			pickedDir?: string | null;
		}) => {
			if (data.models) fixtureModels = structuredClone(data.models);
			if (data.links) fixtureLinks = structuredClone(data.links);
			if ('pickedDir' in data) fixturePickedDir = data.pickedDir ?? null;
		},
	},
});
const settingsShell = new URLSearchParams(location.search).has(
	'settings-shell'
);
const nativeGlass =
	settingsShell && !new URLSearchParams(location.search).has('fallback');
export const isTauri = () => nativeGlass;
const settingsMetrics = {
	calls: [] as { command: string; args: unknown }[],
	glass: [] as { surface: unknown; revision: number }[],
	inFlight: 0,
	maxInFlight: 0,
};
const appearance = {
	theme_mode: 'light',
	accent: '#339cff',
	background: '#ffffff',
	foreground: '#1a1c1f',
	ui_font: 'system-ui',
	code_font: 'monospace',
	contrast: 45,
	currency: 'usd',
};
if (settingsShell)
	Object.assign(window, { settingsShellMetrics: settingsMetrics });
type Args = {
	root?: string;
	cwd?: string;
	id?: string;
	path?: string;
	cols?: number;
	rows?: number;
	output?: Channel;
	op?: string;
	cmd?: UiCommand;
	payload?: {
		paths?: string[];
		path?: string;
		access?: string;
		persist?: boolean;
		enabled?: boolean;
	};
};
const workspaceLinks = new URLSearchParams(location.search).has(
	'workspace-links'
);
const markdownResources = new URLSearchParams(location.search).has(
	'markdown-resources'
);
const workspaceFileCalls: { command: string; root?: string; path?: string }[] =
	[];
if (workspaceLinks || markdownResources)
	Object.assign(window, { workspaceFileCalls });
type Metrics = { start: number; ipc: unknown[] };
const resizing = new URLSearchParams(location.search).has('resize');
let nextId = 0;
const outputs = new Map<string, Channel>();
function redraw(output: Channel | undefined, cols = 80, rows = 24) {
	const lines = Array.from(
		{ length: Math.max(1, rows - 1) },
		(_, i) =>
			`\x1b[3${(i % 7) + 1}m${`row ${i} | terminal output `.padEnd(Math.max(1, cols - 2), '.')}\x1b[0m`
	).join('\r\n');
	output?.onmessage(new TextEncoder().encode(`\x1b[H${lines}`).buffer);
}
export async function invoke(command: string, args: Args = {}) {
	if (command === 'agent_cmd' && args.cmd && typeof args.cmd === 'object') {
		const cmd = args.cmd;
		if (
			'SetModel' in cmd ||
			'SetThinkingLevel' in cmd ||
			'SetPermissionMode' in cmd ||
			'SetAgentMode' in cmd
		) {
			if (settingsShell) settingsMetrics.calls.push({ command, args });
			if ('SetModel' in cmd) {
				fixtureModels.active_provider = cmd.SetModel.provider;
				fixtureModels.active_model = cmd.SetModel.model;
			}
			if ('SetThinkingLevel' in cmd)
				fixtureModels.active_thinking_level = cmd.SetThinkingLevel.level;
			if ('SetPermissionMode' in cmd)
				fixtureModels.active_permission_mode = cmd.SetPermissionMode.mode;
			if ('SetAgentMode' in cmd)
				fixtureModels.active_agent_mode = cmd.SetAgentMode.mode;
			return;
		}
	}
	if (
		(workspaceLinks || markdownResources) &&
		(command === 'workspace_list_files' || command === 'workspace_preview_file')
	) {
		workspaceFileCalls.push({ command, root: args.root, path: args.path });
		if (
			!['/repo', '/other'].includes(args.root ?? '') ||
			args.path?.startsWith('/') ||
			args.path?.split('/').includes('..')
		)
			throw new Error('Invalid fixture workspace read');
		if (markdownResources) {
			const name = args.path?.split('/').pop() ?? '';
			const file = (text: string) => ({
				path: args.path,
				name,
				kind: 'text',
				size: text.length,
				text,
			});
			if (command === 'workspace_list_files')
				return {
					path: args.path ?? '',
					truncated: false,
					entries:
						args.path === 'docs'
							? [
									'start.md',
									'slow.md',
									'other.md',
									'remote.md',
									'remote-error.md',
								].map((name) => ({
									name,
									path: `docs/${name}`,
									kind: 'file',
									size: 100,
								}))
							: [{ name: 'docs', path: 'docs', kind: 'directory', size: 0 }],
				};
			if (name === 'remote.md')
				return file(
					[
						'<div align="center">\n\n<img src="../assets/logo.png" alt="Husk" width="112" />\n\n</div>',
						'# Remote badges',
						'[![version](https://img.shields.io/github/v/tag/1Yie/husk?style=flat-square&label=version&color=2f6feb&sort=semver "Version badge")](https://github.com/1Yie/husk/releases) ![rust](https://img.shields.io/badge/rust-stable-dea584?style=flat-square&logo=rust&logoColor=white) ![tauri](https://img.shields.io/badge/tauri-2-24c8db?style=flat-square&logo=tauri&logoColor=white) ![platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-6e7681?style=flat-square) ![license](https://img.shields.io/badge/license-MIT-3fb950?style=flat-square)',
						'Following badges',
						'![HTTP blocked](http://example.com/unapproved.png)',
						'![credentials blocked](https://user:secret@example.com/private.png)',
						'![unsafe remote](javascript:alert%281%29)',
						'<img src="https://img.shields.io/badge/license-MIT-3fb950?style=flat-square" alt="unsafe attributes" width="112px" height="-10" onerror="window.markdownExecuted=true" />',
						'<script>window.markdownExecuted = true</script>',
					].join('\n\n')
				);
			if (name === 'remote-error.md')
				return file('![remote failure](https://example.com/external.png)');
			if (name === 'start.md')
				return file(
					[
						'# Markdown resources',
						'[sibling](guide.md#L2C3)',
						'[parent](../README.md)',
						'[spaced](<a b.md>)',
						'[encoded](a%20b.md:2)',
						'[absolute](/repo/docs/guide.md)',
						'[file URL](file:///repo/docs/guide.md)',
						'[escape](../../outside.md)',
						'[unsafe](javascript:alert%281%29)',
						'[heading](#markdown-resources)',
						'[sibling heading](guide.md#guide)',
						'[local line](#L1C2)',
						'[query](guide.md?download=1)',
						'![query image](../assets/picture.png?raw=1)',
						'[web](https://example.com/)',
						'![PNG alt](../assets/picture.png)',
						'![JPEG alt](../assets/picture.jpg)',
						'![GIF alt](../assets/picture.gif)',
						'![WebP alt](../assets/picture.webp)',
						'![missing alt](../assets/missing.png)',
						'![SVG alt](../assets/unsafe.svg)',
						'![large alt](../assets/large.png)',
						'![external alt](https://example.com/external.png)',
						'![escape alt](../../escape.png)',
						'![data alt](data:image/png;base64,AA==)',
						'<script>window.markdownExecuted = true</script>',
					].join('\n\n')
				);
			if (name === 'slow.md')
				return file('# Pending\n\n![slow alt](../assets/slow.png)');
			if (name === 'other.md')
				return file('# Other document\n\n![other alt](../assets/picture.png)');
			if (args.path === 'assets/missing.png')
				throw new Error('Fixture image not found');
			if (args.path === 'assets/slow.png')
				await new Promise((resolve) => setTimeout(resolve, 500));
			if (args.path === 'assets/large.png')
				return { path: args.path, name, kind: 'too_large', size: 9_000_000 };
			if (args.path === 'assets/unsafe.svg')
				return file('<svg onload="alert(1)"/>');
			if (args.path?.startsWith('assets/')) {
				const mime = name.endsWith('.jpg')
					? 'image/jpeg'
					: name.endsWith('.webp')
						? 'image/webp'
						: name.endsWith('.gif')
							? 'image/gif'
							: 'image/png';
				const canvas = document.createElement('canvas');
				canvas.width = name === 'logo.png' ? 224 : 96;
				canvas.height = name === 'logo.png' ? 112 : 48;
				const context = canvas.getContext('2d')!;
				context.fillStyle = '#339cff';
				context.fillRect(0, 0, canvas.width, canvas.height);
				const data =
					mime === 'image/gif'
						? 'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'
						: canvas.toDataURL(mime).split(',')[1];
				return {
					path: args.path,
					name,
					kind: 'image',
					size: data.length,
					mime,
					data,
				};
			}
			return file('# Guide\nsecond line\nthird line');
		}
		if (command === 'workspace_list_files')
			return {
				path: args.path ?? '',
				truncated: false,
				entries: [
					{ name: 'src', path: 'src', kind: 'directory', size: 0 },
					{ name: 'notes.md', path: 'notes.md', kind: 'file', size: 50 },
				],
			};
		if (args.path === 'missing.ts') throw new Error('Fixture file not found');
		if (args.path === 'slow.ts')
			await new Promise((resolve) => setTimeout(resolve, 400));
		const name = args.path?.split('/').pop() ?? '';
		if (name === 'large.ts')
			return { path: args.path, name, size: 3_000_000, kind: 'too_large' };
		const content =
			name === 'notes.md'
				? '# Fixture\n\nMarkdown source target\n'
				: Array.from(
						{ length: name === 'long.ts' ? 2100 : 120 },
						(_, i) => `line ${i + 1}: workspace fixture text`
					).join('\n');
		return {
			path: args.path,
			name,
			size: content.length,
			kind: 'text',
			text: content,
		};
	}
	if (
		new URLSearchParams(location.search).has('welcome-responsive') &&
		command === 'agent_session'
	) {
		if (args.op === 'usage_stats')
			return {
				sessions: new URLSearchParams(location.search).has('zero')
					? []
					: [
							{
								session: 1,
								title: 'Welcome fixture',
								workspace: '/repo',
								ts: Date.now() / 1000,
								prompt: 1200,
								completion: 300,
								cached: 100,
								context_window: 128000,
							},
						],
			};
		if (args.op === 'recent_artifacts')
			return [
				{
					path: '/repo/out/report.pdf',
					name: 'Fixture report.pdf',
					ext: 'pdf',
					mtime: Date.now() / 1000,
				},
			];
	}
	if (command === 'agent_session' && args.op === 'recent_artifacts') return [];
	if (settingsShell) {
		settingsMetrics.calls.push({ command, args });
		if (command === 'sidebar_glass_update') {
			// SAFETY: this fixture branch receives the sidebar-glass command's surface/revision payload.
			const glass = args as unknown as { surface: unknown; revision: number };
			settingsMetrics.glass.push(glass);
			settingsMetrics.maxInFlight = Math.max(
				settingsMetrics.maxInFlight,
				++settingsMetrics.inFlight
			);
			await new Promise((resolve) => setTimeout(resolve, 20));
			settingsMetrics.inFlight--;
			return glass.surface !== null;
		}
		if (command === 'agent_session') {
			if (args.op === 'get_dev_config') return { developer_ui: false };
			if (args.op === 'get_default_prefs')
				return { permission_mode: 'auto', thinking_level: 'medium' };
			if (args.op === 'get_app_config') return { config: { providers: {} } };
			if (args.op === 'get_appearance') return { ...appearance };
			if (args.op === 'set_appearance') {
				Object.assign(appearance, args.payload);
				return { success: true };
			}
			if (args.op === 'set_default_prefs') return { success: true };
		}
	}
	if (command === 'terminal_fonts') return [];
	// SAFETY: the fixture bootstrap owns this optional window property and its Metrics shape.
	const metrics = (window as unknown as { __terminalMetrics?: Metrics })
		.__terminalMetrics;
	if (command.startsWith('terminal_')) {
		metrics?.ipc.push({
			command,
			at: performance.now() - metrics.start,
			...args,
			output: undefined,
		});
	}
	if (command === 'terminal_open') {
		await new Promise((resolve) => setTimeout(resolve, 80));
		const id = `pty-fixture-${++nextId}`;
		const output = args.output;
		if (output) outputs.set(id, output);
		if (resizing) {
			const data = new TextEncoder().encode(
				Array.from(
					{ length: 3000 },
					(_, i) => `history ${i} ${'scrollback content '.repeat(8)}\r\n`
				).join('')
			);
			for (let i = 0; i < data.length; i += 8192)
				output?.onmessage(data.slice(i, i + 8192).buffer);
			output?.onmessage(new TextEncoder().encode('\x1b[?1049h').buffer);
			redraw(output, args.cols, args.rows);
		}
		args.output?.onmessage(
			new TextEncoder().encode('\x1b[32mhusk\x1b[0m on main\r\n❯ ').buffer
		);
		return { id, cwd: args.cwd ?? '/repo', shell: '/bin/sh' };
	}
	if (command === 'terminal_resize' && resizing) {
		// A slow native round-trip plus a TUI's SIGWINCH redraw. Instant resize
		// mocks and an empty terminal concealed the obsolete-size backlog.
		await new Promise((resolve) => setTimeout(resolve, 200));
		redraw(args.id ? outputs.get(args.id) : undefined, args.cols, args.rows);
	}
	if (command === 'terminal_close' && args.id) outputs.delete(args.id);
	if (command === 'workspace_list_files')
		return { path: args.path ?? '', entries: [], truncated: false };
	if (command === 'agent_session') {
		if (args.op === 'stat_paths')
			return Object.fromEntries(
				(args.payload?.paths ?? []).map((path) => [
					path,
					path === 'out/long-deliverable-name.pdf',
				])
			);
		if (args.op === 'model_info') return structuredClone(fixtureModels);
		if (args.op === 'agent_overview')
			return {
				instructions: '',
				model: structuredClone(fixtureModels),
				skills: [],
				plugins: [],
				subagents: [],
				rules: [],
			};
		if (args.op === 'add_mcp')
			return { success: true, path: '/fixture/connectors/manifest.json' };
		if (args.op === 'reload_mcp') return { plugins: [] };
		if (args.op === 'set_fast_mode') {
			fixtureModels.active_fast_mode = args.payload?.enabled === true;
			return { fast_mode: fixtureModels.active_fast_mode };
		}
		if (args.op === 'git_info') return { repo: true, branch: 'main', dirty: 0 };
		if (args.op === 'workspace_changes') return { repo: false, files: [] };
		if (args.op === 'linked_roots') return structuredClone(fixtureLinks);
		if (args.op === 'pick_link_dir') return fixturePickedDir;
		if (
			args.op === 'set_link_access' ||
			args.op === 'unlink_root' ||
			args.op === 'link_root'
		) {
			const linkedPath = args.payload?.path;
			if (!linkedPath) throw new Error('A linked path is required');
			if (args.op === 'unlink_root') {
				fixtureLinks.workspace = fixtureLinks.workspace.filter(
					(root) => root.path !== linkedPath
				);
				fixtureLinks.session = fixtureLinks.session.filter(
					(root) => root.path !== linkedPath
				);
			} else {
				const access = args.payload?.access;
				if (access !== 'read' && access !== 'readwrite')
					throw new Error('Invalid linked access');
				if (args.op === 'set_link_access') {
					const update = (rows: LinkedRootLayers['workspace']) =>
						rows.map((root) =>
							root.path === linkedPath ? { ...root, access } : root
						);
					fixtureLinks.workspace = update(fixtureLinks.workspace);
					fixtureLinks.session = update(fixtureLinks.session);
				} else {
					const scope = args.payload?.persist ? 'workspace' : 'session';
					fixtureLinks[scope] = [
						...fixtureLinks[scope].filter((root) => root.path !== linkedPath),
						{ path: linkedPath, access, scope },
					];
				}
			}
			return structuredClone(fixtureLinks);
		}
		if (
			args.op?.startsWith('list_') ||
			args.op === 'bg_list' ||
			args.op === 'plugin_commands'
		)
			return [];
	}
	if (settingsShell && command === 'terminal_resize') return;
	if (settingsShell && command === 'terminal_close') return;
	if (settingsShell)
		throw new Error(
			`Unhandled settings fixture IPC: ${command} / ${args.op ?? ''}`
		);
	return {};
}
export const convertFileSrc = (src: string) => src;
export async function listen<T>(
	event: string,
	handler: (event: { event: string; id: number; payload: T }) => void
) {
	const listener = (message: Event) =>
		handler({
			event,
			id: 0,
			payload: (message as CustomEvent<T>).detail,
		});
	window.addEventListener(event, listener);
	return () => window.removeEventListener(event, listener);
}
export async function emit<T>(event: string, payload?: T) {
	window.dispatchEvent(new CustomEvent(event, { detail: payload }));
}
export const getCurrentWindow = () => ({
	onFocusChanged: async () => () => {},
	isFocused: async () => true,
	startDragging: async () => {},
	toggleMaximize: async () => {},
	isMaximized: async () => false,
	onResized: async () => () => {},
	minimize: async () => {},
	close: async () => {},
});
