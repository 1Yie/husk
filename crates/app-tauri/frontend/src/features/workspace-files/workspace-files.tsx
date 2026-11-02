import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
	type CSSProperties,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
	ArrowLeft,
	ChevronDown,
	ChevronRight,
	Folder,
	Link,
	Maximize2,
	RefreshCw,
} from 'lucide-react';
import { WorkspaceFileIcon } from './file-icon';
import { CodeLoadingSkeleton, DirectoryLoadingSkeleton } from './loading';
import {
	Dialog,
	DialogContent,
	DialogTitle,
	DialogTrigger,
} from '@/components/ui/dialog';
import { TooltipSimple } from '@/components/ui/tooltip';
import {
	codeHighlightPlugin,
	normalizeLanguage,
	type HighlightResult,
	type HighlightToken,
} from '@/lib/syntax-highlight';
import type { DirectoryListing, FilePreview, WorkspaceEntry } from './types';
import './workspace-files.css';
import {
	parseWorkspaceFileReference,
	type WorkspaceFileRequest,
	type MarkdownResourceTarget,
} from './file-reference';
import { WorkspaceMarkdown } from './markdown-resources';

export interface WorkspaceFilesProps {
	root: string;
	onOpenWorkspace?: () => void;
	onBack?: () => void;
	openRequest?: WorkspaceFileRequest;
}

type DirectoryState = {
	listing?: DirectoryListing;
	error?: string;
	loading?: boolean;
};
type FileLocation = MarkdownResourceTarget & { activation: number };
type PreviewState = {
	file?: FilePreview;
	path?: string;
	error?: string;
	loading?: boolean;
	location?: FileLocation;
};
type PreviewContext = {
	root: string;
	onNavigate: (target: MarkdownResourceTarget) => void;
};

function WorkspaceFilesHeader({
	name,
	path,
	onBack,
	backLabel,
	icon,
	children,
}: {
	name: string;
	path: string;
	onBack?: () => void;
	backLabel: string;
	icon?: React.ReactNode;
	children: React.ReactNode;
}) {
	return (
		<header className="workspace-files-header">
			{onBack && (
				<TooltipSimple content={backLabel}>
					<button
						type="button"
						className="workspace-files-header-action"
						onClick={onBack}
						aria-label={backLabel}
					>
						<ArrowLeft size={14} aria-hidden="true" />
					</button>
				</TooltipSimple>
			)}
			{icon}
			<TooltipSimple content={path}>
				<span className="workspace-files-name">{name}</span>
			</TooltipSimple>
			{children}
		</header>
	);
}

function sizeLabel(size: number) {
	if (size < 1024) return `${size} B`;
	if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
	return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

function languageOf(name: string) {
	const extension = name.split('.').pop()?.toLowerCase() ?? '';
	return normalizeLanguage(extension === 'md' ? 'markdown' : extension);
}

function PreviewStatus({
	file,
	language,
	lines,
}: {
	file: FilePreview;
	language?: string;
	lines?: number;
}) {
	return (
		<footer className="workspace-files-status" aria-label="文件状态">
			<span>{language || file.mime || '文件'}</span>
			{lines !== undefined && <span>{lines.toLocaleString()} 行</span>}
			<span>{sizeLabel(file.size)}</span>
			<span className="workspace-files-readonly">只读</span>
		</footer>
	);
}

function SourcePreview({
	file,
	large,
	location,
}: {
	file: FilePreview;
	large: boolean;
	location?: FileLocation;
}) {
	const source = file.text ?? '';
	// Bound DOM/tokenization cost independently of the backend's byte limit.
	const cap = large ? 1_000_000 : 200_000;
	const lineCap = large ? 10_000 : 2_000;
	const text = useMemo(
		() => source.slice(0, cap).split('\n').slice(0, lineCap).join('\n'),
		[source, cap, lineCap]
	);
	const language = languageOf(file.name);
	const [highlight, setHighlight] = useState<{
		code: string;
		language: string;
		result: HighlightResult | null;
	} | null>(null);
	useEffect(() => {
		let live = true;
		let settled = false;
		let fallback: ReturnType<typeof setTimeout> | undefined;
		setHighlight(null);
		const publish = (result: HighlightResult | null) => {
			if (!live) return;
			settled = true;
			if (fallback !== undefined) clearTimeout(fallback);
			setHighlight({ code: text, language, result });
		};
		try {
			const initial = codeHighlightPlugin.highlight(
				{ code: text, language },
				publish
			);
			// A lazy grammar failing to load must never leave an endless skeleton.
			// Plain text is readable immediately after this bounded upgrade window.
			if (!settled) fallback = setTimeout(() => publish(initial), 180);
		} catch {
			publish(null);
		}
		return () => {
			live = false;
			if (fallback !== undefined) clearTimeout(fallback);
		};
	}, [text, language]);
	const ready =
		highlight?.code === text && highlight.language === language
			? highlight
			: null;
	const lines: HighlightToken[][] =
		ready?.result?.tokens ?? text.split('\n').map((content) => [{ content }]);
	const scroller = useRef<HTMLDivElement>(null);
	const sourceLines = source.split('\n');
	const visibleLines = text.split('\n');
	const line = location?.line;
	const column = location?.column;
	let notice = '';
	if (line !== undefined) {
		if (
			!Number.isSafeInteger(line) ||
			line < 1 ||
			(column !== undefined && (!Number.isSafeInteger(column) || column < 1))
		)
			notice = '无效的行列位置。';
		else if (line > sourceLines.length)
			notice = `第 ${line} 行超出文件范围（共 ${sourceLines.length} 行）。`;
		else if (column !== undefined && column > sourceLines[line - 1].length + 1)
			notice = `第 ${line} 行的第 ${column} 列超出行范围。`;
		else if (
			line > visibleLines.length ||
			(column !== undefined && column > visibleLines[line - 1].length + 1)
		)
			notice = '目标位置超出当前预览范围；可展开预览查看。';
		else
			notice = `第 ${line} 行${column === undefined ? '' : `，第 ${column} 列`} · 只读`;
	}
	const located = line !== undefined && notice.endsWith('只读');
	useLayoutEffect(() => {
		if (!ready || !located || !scroller.current) return;
		const element = scroller.current.querySelector<HTMLElement>(
			'[aria-current="location"]'
		);
		if (!element) return;
		const pane = scroller.current;
		pane.scrollTop +=
			element.getBoundingClientRect().top -
			pane.getBoundingClientRect().top -
			pane.clientHeight / 2;
		const marker = element.querySelector<HTMLElement>('[data-source-column]');
		if (marker)
			pane.scrollLeft +=
				marker.getBoundingClientRect().left -
				pane.getBoundingClientRect().left -
				pane.clientWidth / 2;
	}, [ready, located, line, column, location?.activation]);
	return (
		<>
			{location?.fragment !== undefined && (
				<p className="workspace-files-note" role="status" aria-label="文件定位">
					已打开文档；暂不支持标题锚点定位：#{location.fragment}
				</p>
			)}
			{line !== undefined && (
				<p className="workspace-files-note" role="status" aria-label="文件定位">
					{notice}
				</p>
			)}
			{text.length < source.length && (
				<p className="workspace-files-note">
					为保持流畅，仅显示前 {lineCap.toLocaleString()} 行 /{' '}
					{cap.toLocaleString()} 字符。{!large && '展开预览可阅读更多内容。'}
				</p>
			)}
			{ready ? (
				<div
					ref={scroller}
					className="workspace-files-code-scroll"
					tabIndex={0}
					aria-label="文件源码"
					style={
						{
							'--workspace-gutter': `calc(${Math.max(3, String(lines.length).length)}ch + 20px)`,
						} as CSSProperties
					}
				>
					<pre>
						<code>
							{lines.map((tokens, index) => {
								let offset = 0;
								const selected = located && line === index + 1;
								return (
									<span
										className="workspace-files-code-line"
										key={index}
										data-line={index + 1}
										aria-current={selected ? 'location' : undefined}
									>
										<span
											className="workspace-files-line-number"
											aria-hidden="true"
										>
											{index + 1}
										</span>
										<span>
											{tokens.map((token, i) => {
												const at =
													column === undefined ? -1 : column - 1 - offset;
												offset += token.content.length;
												return (
													// The shared chat highlighter intentionally emits dark-only inline
													// colors. Preview uses semantic token classes and its own light/dark
													// editor palette; unclassified tokens inherit readable foreground.
													<span key={i} className={token.htmlAttrs?.className}>
														{selected &&
														at >= 0 &&
														at < token.content.length ? (
															<>
																{token.content.slice(0, at)}
																<mark data-source-column>
																	{token.content[at]}
																</mark>
																{token.content.slice(at + 1)}
															</>
														) : (
															token.content
														)}
													</span>
												);
											})}
											{selected &&
												column === visibleLines[index].length + 1 && (
													<mark data-source-column> </mark>
												)}
											{'\n'}
										</span>
									</span>
								);
							})}
						</code>
					</pre>
				</div>
			) : (
				<CodeLoadingSkeleton />
			)}
			<PreviewStatus
				file={file}
				language={language || 'text'}
				lines={source.split('\n').length}
			/>
		</>
	);
}

function PreviewBody({
	file,
	large = false,
	location,
	root,
	onNavigate,
}: {
	file: FilePreview;
	large?: boolean;
	location?: FileLocation;
} & PreviewContext) {
	if (file.kind === 'text')
		return (
			<TextPreview
				key={file.path}
				file={file}
				large={large}
				location={location}
				root={root}
				onNavigate={onNavigate}
			/>
		);
	// Only signature-verified raster MIME types from the read-only command are allowed.
	if (
		file.kind === 'image' &&
		file.data &&
		/^image\/(png|jpeg|gif|webp)$/.test(file.mime ?? '')
	) {
		return (
			<>
				<div className="workspace-files-image">
					<img alt={file.name} src={`data:${file.mime};base64,${file.data}`} />
				</div>
				<PreviewStatus file={file} />
			</>
		);
	}
	return (
		<p className="workspace-files-note">
			{file.kind === 'too_large'
				? '文件过大，无法预览。文本上限 2 MiB，图片上限 8 MiB。'
				: '二进制或不支持的文件，无法预览。'}
			<br />
			{file.name} · {sizeLabel(file.size)}
		</p>
	);
}

function TextPreview({
	file,
	large = false,
	location,
	root,
	onNavigate,
}: {
	file: FilePreview;
	large?: boolean;
	location?: FileLocation;
} & PreviewContext) {
	const [source, setSource] = useState(false);
	useEffect(() => {
		setSource(location?.line !== undefined);
	}, [location?.activation, location?.line]);
	if (!/\.(md|markdown)$/i.test(file.name))
		return <SourcePreview file={file} large={large} location={location} />;
	const text = file.text ?? '';
	const rendered = text.slice(0, 200_000);
	return (
		<div className="workspace-files-text-preview">
			<div
				role="tablist"
				aria-label="Markdown 显示模式"
				className="workspace-files-preview-modes"
			>
				<button
					type="button"
					role="tab"
					aria-selected={!source}
					onClick={() => setSource(false)}
				>
					预览
				</button>
				<button
					type="button"
					role="tab"
					aria-selected={source}
					onClick={() => setSource(true)}
				>
					源码
				</button>
			</div>
			{!source && location?.fragment !== undefined && (
				<p className="workspace-files-note" role="status" aria-label="文件定位">
					已打开文档；暂不支持标题锚点定位：#{location.fragment}
				</p>
			)}
			{source ? (
				<SourcePreview file={file} large={large} location={location} />
			) : (
				<div className="workspace-files-markdown" aria-label="Markdown 预览">
					<WorkspaceMarkdown
						text={rendered}
						root={root}
						documentPath={file.path}
						onNavigate={onNavigate}
					/>
					{rendered.length < text.length && (
						<p className="workspace-files-note">
							Markdown 预览仅渲染前 200,000 字符；源码模式可查看有界原文。
						</p>
					)}
				</div>
			)}
			{!source && (
				<PreviewStatus
					file={file}
					language="Markdown"
					lines={text.split('\n').length}
				/>
			)}
		</div>
	);
}

export function WorkspaceFiles({
	root,
	onOpenWorkspace,
	onBack,
	openRequest,
}: WorkspaceFilesProps) {
	if (!root.trim())
		return (
			<section
				className="workspace-files workspace-files-empty"
				aria-label="预览工作区文件"
			>
				<p>尚未打开工作区</p>
				{onOpenWorkspace && (
					<button onClick={onOpenWorkspace}>打开工作区</button>
				)}
				{onBack && <button onClick={onBack}>返回侧栏</button>}
			</section>
		);
	// A workspace switch must not retain paths, cached listings, dialogs or pending requests.
	return (
		<WorkspaceBrowser
			key={root}
			root={root}
			onBack={onBack}
			openRequest={openRequest}
		/>
	);
}

function WorkspaceBrowser({
	root,
	onBack,
	openRequest,
}: Pick<WorkspaceFilesProps, 'root' | 'onBack' | 'openRequest'>) {
	const [directories, setDirectories] = useState<
		Record<string, DirectoryState>
	>({});
	const [expanded, setExpanded] = useState<Set<string>>(new Set(['']));
	const [preview, setPreview] = useState<PreviewState>({});
	const [dialogOpen, setDialogOpen] = useState(false);
	const generation = useRef(0);
	const previewRequest = useRef(0);
	const requested = useRef(new Set<string>());
	const tree = useRef<HTMLDivElement>(null);
	const treeScroll = useRef(0);
	const lastOpened = useRef('');
	const restoreFocus = useRef(false);

	useLayoutEffect(() => {
		if (preview.path || !tree.current) return;
		tree.current.scrollTop = treeScroll.current;
		if (restoreFocus.current) {
			Array.from(
				tree.current.querySelectorAll<HTMLButtonElement>(
					'[data-workspace-path]'
				)
			)
				.find((button) => button.dataset.workspacePath === lastOpened.current)
				?.focus({ preventScroll: true });
			restoreFocus.current = false;
		}
	}, [preview.path]);

	function loadDirectory(path: string) {
		if (requested.current.has(path)) return;
		requested.current.add(path);
		const epoch = generation.current;
		setDirectories((current) => ({ ...current, [path]: { loading: true } }));
		void invoke<DirectoryListing>('workspace_list_files', { root, path })
			.then((listing) => {
				if (epoch === generation.current)
					setDirectories((current) => ({ ...current, [path]: { listing } }));
			})
			.catch((error) => {
				if (epoch === generation.current) {
					requested.current.delete(path);
					setDirectories((current) => ({
						...current,
						[path]: { error: String(error) },
					}));
				}
			});
	}

	useEffect(() => {
		loadDirectory('');
		return () => {
			generation.current += 1;
			previewRequest.current += 1;
			requested.current.clear();
		};
	}, [root]);

	function refresh() {
		generation.current += 1;
		previewRequest.current += 1;
		requested.current.clear();
		setDirectories({});
		setExpanded(new Set(['']));
		setPreview({});
		setDialogOpen(false);
		treeScroll.current = 0;
		lastOpened.current = '';
		loadDirectory('');
	}

	const openFile = useCallback(
		(entry: Pick<WorkspaceEntry, 'path'>, location?: FileLocation) => {
			treeScroll.current = tree.current?.scrollTop ?? treeScroll.current;
			lastOpened.current = entry.path;
			const id = ++previewRequest.current;
			const epoch = generation.current;
			setPreview({ path: entry.path, loading: true, location });
			void invoke<FilePreview>('workspace_preview_file', {
				root,
				path: entry.path,
			})
				.then((file) => {
					if (id === previewRequest.current && epoch === generation.current)
						setPreview({ path: entry.path, file, location });
				})
				.catch((error) => {
					if (id === previewRequest.current && epoch === generation.current)
						setPreview({ path: entry.path, error: String(error), location });
				});
		},
		[root]
	);

	const navigate = useCallback(
		(target: MarkdownResourceTarget) => {
			setDialogOpen(false);
			openFile(target, { ...target, activation: previewRequest.current + 1 });
		},
		[openFile]
	);

	useEffect(() => {
		if (!openRequest || openRequest.workspaceRoot !== root) return;
		const target = parseWorkspaceFileReference(openRequest.path, root);
		if (!target || target.path !== openRequest.path) return;
		const parents = openRequest.path.split('/').slice(0, -1);
		let path = '';
		for (const part of parents) {
			path = path ? `${path}/${part}` : part;
			const directory = path;
			setExpanded((current) => new Set([...current, directory]));
			loadDirectory(directory);
		}
		setDialogOpen(false);
		openFile(target, openRequest);
	}, [openRequest, root]);

	function backToFiles() {
		previewRequest.current += 1;
		restoreFocus.current = true;
		setDialogOpen(false);
		setPreview({});
	}

	function toggleDirectory(path: string) {
		setExpanded((current) => {
			const next = new Set(current);
			if (next.has(path)) next.delete(path);
			else next.add(path);
			return next;
		});
		if (!expanded.has(path)) loadDirectory(path);
	}

	function renderDirectory(path: string, depth: number): React.ReactNode {
		const state = directories[path];
		if (!state || state.loading)
			return <DirectoryLoadingSkeleton depth={depth} />;
		if (state.error)
			return (
				<div className="workspace-files-note" role="alert">
					{state.error}
					<button onClick={() => loadDirectory(path)}>重试</button>
				</div>
			);
		const listing = state.listing!;
		return (
			<>
				{listing.entries.length === 0 && (
					<p className="workspace-files-note">此目录为空</p>
				)}
				<ul className="workspace-files-list">
					{listing.entries.map((entry) => {
						const directory = entry.kind === 'directory';
						const isOpen = expanded.has(entry.path);
						return (
							<li key={entry.path}>
								<button
									className="workspace-files-entry"
									style={{ paddingLeft: 8 + depth * 14 }}
									data-workspace-path={entry.path}
									aria-label={
										entry.kind === 'symlink'
											? `${entry.name}（符号链接）`
											: entry.name
									}
									title={entry.path}
									aria-expanded={directory ? isOpen : undefined}
									aria-current={
										!directory &&
										(preview.path ?? lastOpened.current) === entry.path
											? 'true'
											: undefined
									}
									onClick={() =>
										directory ? toggleDirectory(entry.path) : openFile(entry)
									}
								>
									{directory ? (
										isOpen ? (
											<ChevronDown />
										) : (
											<ChevronRight />
										)
									) : (
										<span className="workspace-files-chevron-space" />
									)}
									{directory ? (
										<Folder />
									) : entry.kind === 'symlink' ? (
										<Link />
									) : (
										<WorkspaceFileIcon name={entry.name} />
									)}
									<span className="workspace-files-name">{entry.name}</span>
									{entry.kind === 'symlink' && (
										<span className="workspace-files-link-label">链接</span>
									)}
								</button>
								{directory && isOpen && renderDirectory(entry.path, depth + 1)}
							</li>
						);
					})}
				</ul>
				{listing.truncated && (
					<p className="workspace-files-note">
						目录内容过多，仅显示前 2,000 项。
					</p>
				)}
			</>
		);
	}

	return (
		<section
			className="workspace-files"
			aria-label="预览工作区文件"
			data-view={preview.path ? 'preview' : 'tree'}
		>
			{!preview.path && (
				<WorkspaceFilesHeader
					name={root.split(/[\\/]/).filter(Boolean).pop() ?? root}
					path={root}
					onBack={onBack}
					backLabel="返回侧栏"
				>
					<TooltipSimple content="刷新文件">
						<button
							type="button"
							className="workspace-files-header-action"
							onClick={refresh}
							aria-label="刷新文件"
						>
							<RefreshCw size={14} aria-hidden="true" />
						</button>
					</TooltipSimple>
				</WorkspaceFilesHeader>
			)}
			{!preview.path ? (
				<div
					ref={tree}
					className="workspace-files-tree"
					aria-label="工作区目录"
				>
					{renderDirectory('', 0)}
				</div>
			) : (
				<section className="workspace-files-preview" aria-label="文件预览">
					<WorkspaceFilesHeader
						name={preview.path.split('/').pop() ?? preview.path}
						path={preview.path}
						onBack={backToFiles}
						backLabel="返回文件列表"
						icon={
							<WorkspaceFileIcon
								name={preview.path}
								className="workspace-files-editor-icon"
							/>
						}
					>
						{preview.file && (
							<Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
								<TooltipSimple content="展开预览">
									<DialogTrigger asChild>
										<button
											type="button"
											className="workspace-files-header-action"
											aria-label="展开预览"
										>
											<Maximize2 size={14} aria-hidden="true" />
										</button>
									</DialogTrigger>
								</TooltipSimple>
								<DialogContent
									className="workspace-files-dialog"
									aria-describedby={undefined}
								>
									<DialogTitle
										className="workspace-files-dialog-title"
										title={preview.path}
									>
										{preview.path}
									</DialogTitle>
									{dialogOpen && (
										<PreviewBody
											file={preview.file}
											large
											location={preview.location}
											root={root}
											onNavigate={navigate}
										/>
									)}
								</DialogContent>
							</Dialog>
						)}
					</WorkspaceFilesHeader>
					<div
						className="workspace-files-breadcrumbs"
						aria-label="文件路径"
						title={preview.path}
					>
						{preview.path.split('/').join(' › ')}
					</div>
					{preview.loading && <CodeLoadingSkeleton label="正在读取文件" />}
					{preview.error && (
						<p className="workspace-files-note" role="alert">
							{preview.error}
						</p>
					)}
					{preview.file && (
						<PreviewBody
							file={preview.file}
							location={preview.location}
							root={root}
							onNavigate={navigate}
						/>
					)}
				</section>
			)}
		</section>
	);
}
