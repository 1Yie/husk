import { Skeleton } from '@/components/ui/skeleton';
import { ShortcutHint, TerminalToggle } from '@/components/layout-controls';
import iconUrl from '@/assets/husk-icon.png';
import {
	Plus,
	Code,
	ChevronRight,
	ChevronDown,
	FileText,
	Folder,
	FolderOpen,
	FolderPlus,
	Settings,
	Bell,
	MoreHorizontal,
	GitFork,
	Bin,
	Bookmark,
	X,
	Pen,
	Copy,
	Archive,
	Globe,
} from '@keyline-icons/react';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Download, ExternalLink } from 'lucide-react';
import { useRegisterCommand } from '@/lib/command-registry';
import { toast } from 'sonner';
import type {
	ProjectOverview,
	ProjectSessionRow,
	SessionRow,
	SidebarEntry,
} from '@/types';
import type { SidebarPaging } from '@/features/chat/hooks/use-agent-session';
import { useDelayedFlag } from '@/hooks/use-delayed-flag';
import {
	loadOnDownwardWheel,
	loadOnPageKey,
	loadReachedPage,
	SidebarPageTail,
} from './pagination';
import { Orb } from '@/features/chat/components/agent-orb/index';
import { ExtIcon } from '@/features/chat/components/artifacts-panel';
import {
	openPath,
	recentArtifacts,
	revealPath,
	saveArtifact,
	type RecentArtifact,
} from '@/lib/agent-ipc/index';
import {
	useActiveViewSelector,
	useHasRunningChildren,
} from '@/stores/agent-store';
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
	ContextMenuTrigger,
} from '@/components/ui/context-menu';
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	TooltipSimple,
} from '@/components/ui/tooltip';
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from '@/components/ui/popover';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { framedShell, isMac } from '@/lib/platform';
import { cn } from '@/lib/utils';
import {
	BORDER_OVERLAY_DIVIDER,
	INK_OVERLAY_DESTRUCTIVE,
	INK_OVERLAY_SECONDARY,
	OVERLAY_HEADING,
	SURFACE_OVERLAY_HOVER,
} from '@/lib/theme';
import {
	useNotifications,
	useUnreadCount,
	markRead,
	markAllRead,
	markSessionRead,
	clearAll,
	notificationHeadline,
	useSessionTitles,
	type AppNotification,
	type NotificationKind,
} from '@/lib/notifications';

/** Cross-project recent list cap — everything beyond it stays reachable by
 *  opening the owning project in 项目. */
const RECENT_LIMIT = 6;

/** Unread marker colour per kind — the row's only severity cue. */
const NOTIFY_DOT: Record<NotificationKind, string> = {
	turn: 'bg-blue-500',
	attention: 'bg-amber-500',
	error: 'bg-red-500',
};

/** Popover timestamp — clock time for today, day + time for anything older.
 *  A bare time on yesterday's row reads as "just now". */
function notifyStamp(ts: number): string {
	const d = new Date(ts);
	const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
	const day = (x: Date) => `${x.getFullYear()}-${x.getMonth()}-${x.getDate()}`;
	const today = new Date();
	if (day(d) === day(today)) return time;
	const yesterday = new Date(today);
	yesterday.setDate(today.getDate() - 1);
	if (day(d) === day(yesterday)) return `昨天 ${time}`;
	return `${d.getMonth() + 1}月${d.getDate()}日 ${time}`;
}

/** 产物 categories — the file tree's fixed buckets. `rep` is the ext used
 * for the group's icon; `asset` also matches anything under an
 * `assets/` directory (the mode prompt puts media there regardless of
 * ext). */
const ART_GROUPS: { key: string; label: string; rep: string }[] = [
	{ key: 'pdf', label: 'PDF', rep: 'pdf' },
	{ key: 'doc', label: 'DOC', rep: 'docx' },
	{ key: 'ppt', label: 'PPT', rep: 'pptx' },
	{ key: 'xls', label: 'XLS', rep: 'xlsx' },
	{ key: 'asset', label: '素材', rep: 'png' },
	{ key: 'other', label: '其他', rep: '' },
];
function artCategory(f: RecentArtifact): string {
	if (/(^|\/)assets\//.test(f.path)) return 'asset';
	switch ((f.name.split('.').pop() ?? '').toLowerCase()) {
		case 'pdf':
			return 'pdf';
		case 'docx':
		case 'doc':
		case 'md':
		case 'txt':
		case 'rtf':
			return 'doc';
		case 'pptx':
		case 'ppt':
			return 'ppt';
		case 'xlsx':
		case 'xls':
		case 'csv':
			return 'xls';
		case 'png':
		case 'jpg':
		case 'jpeg':
		case 'gif':
		case 'webp':
		case 'svg':
		case 'bmp':
		case 'mp4':
		case 'mov':
		case 'webm':
		case 'mp3':
		case 'wav':
		case 'flac':
		case 'ttf':
		case 'otf':
		case 'woff':
		case 'woff2':
			return 'asset';
		default:
			return 'other';
	}
}

interface Props {
	/** Metadata totals are independent of each project's loaded rows. */
	projects: (ProjectOverview & { total?: number; archived_total?: number })[];
	recents?: SidebarEntry[];
	paging?: SidebarPaging;
	error?: string;
	onRetry?: () => void;
	/** The first project list has landed. Until it has, an empty `projects`
	 * means "still loading", not "no sessions" — the lists draw placeholder
	 * rows instead of the empty-state text. */
	projectsLoaded?: boolean;
	activeId?: number;
	version?: string;
	hasWorkspace?: boolean;
	onNew: () => void;
	/** Open a conversation anywhere in the tree — `root` lets the shell switch
	 * workspaces when the row belongs to another project. */
	onOpenSession: (root: string, id: number) => void;
	onPickWorkspace?: () => void;
	onSwitchWorkspace?: (path: string) => void;
	/** The 编程 ↔ 工作 mode switch. Separate from `onSwitchWorkspace` because
	 * only a mode switch gets the full-window veil; a plain project switch
	 * stays in the sidebar and the chat pane. */
	onSwitchMode?: (path: string) => void;
	onRemoveWorkspace?: (project: ProjectOverview) => void;
	onOpenSettings?: () => void;
	onToggleTerminal?: () => void;
	terminalOpen?: boolean;
	/** Per-row session actions — see `SessionActions`. */
	actions?: SessionActions;
}

/** Which pin a list orders by — see `SessionItem.pinMark`. */
type PinMark = 'workspace' | 'global' | 'single';

/** What a session row's menu can do. Every handler gets the row's workspace
 *  `root`: meta edits (rename, pins, archive, export) work on any listed
 *  project, while fork/delete need the session's live actor and so are only
 *  offered on the active workspace's rows. */
export interface SessionActions {
	rename?: (root: string, row: SessionRow) => void;
	pin?: (root: string, id: number, pinned: boolean) => void;
	pinGlobal?: (root: string, id: number, pinned: boolean) => void;
	archive?: (root: string, row: SessionRow, archived: boolean) => void;
	exportRecord?: (root: string, id: number) => void;
	fork?: (id: number) => void;
	delete?: (id: number) => void;
}

/** Placeholder rows shaped like a session row — drawn while the project list
 * is still loading, so the sidebar doesn't say "no sessions" before it knows. */
function SidebarRowSkeleton({
	rows,
	small = false,
}: {
	rows: number;
	small?: boolean;
}) {
	return (
		<div className="flex animate-pulse flex-col gap-0.5" aria-hidden="true">
			{Array.from({ length: rows }, (_, i) => (
				<div
					key={i}
					className={cn('flex items-center pl-7', small ? 'h-7' : 'h-8')}
				>
					<Skeleton className={cn('h-3 rounded', small ? 'w-24' : 'w-32')} />
				</div>
			))}
		</div>
	);
}

export function SessionSidebar({
	projects,
	recents: summaryRecents,
	paging,
	error,
	onRetry,
	projectsLoaded = true,
	activeId,
	version: propVersion,
	hasWorkspace = true,
	onNew,
	onOpenSession,
	onPickWorkspace,
	onSwitchWorkspace,
	onSwitchMode,
	onRemoveWorkspace,
	onOpenSettings,
	onToggleTerminal,
	terminalOpen,
	actions = {},
}: Props) {
	const initialBusyShown = useDelayedFlag(!projectsLoaded);
	const notifications = useNotifications();
	const unreadCount = useUnreadCount();
	// The office pseudo-workspace arrives in the same project list flagged
	// `office`. It's never a project row — it drives the header tab (工作 ↔
	// 编程 switch) and, while office mode is active, supplies the flat
	// session archive rendered where the project tree normally sits.
	const officeProject = projects.find((p) => p.office);
	const projectList = projects.filter((p) => !p.office);
	const officeMode = officeProject?.current === true;
	useRegisterCommand('mode-build', () => {
		const target = projectList[0]?.root;
		if (target) onSwitchMode?.(target);
		else onPickWorkspace?.();
	});
	useRegisterCommand('mode-office', () => {
		if (officeProject) onSwitchMode?.(officeProject.root);
	});
	// Rows are named the way the desktop toast was — one headline rule, fed by
	// the title index the app shell keeps current.
	const sessionTitles = useSessionTitles();
	const titleFor = (n: AppNotification): string =>
		notificationHeadline(n, sessionTitles);
	const openNotification = (n: AppNotification) => {
		if (n.root === undefined || n.session === undefined) {
			markRead(n.id);
			return;
		}
		// The whole conversation counts as seen once it is in front of the user.
		markSessionRead(n.root, n.session);
		onOpenSession(n.root, n.session);
	};
	// Global recents: every *project's* conversations merged. Globally pinned
	// sessions are always listed, ahead of the RECENT_LIMIT newest others, so
	// the top of the sidebar answers "what was I just doing" while global pins
	// stay reachable. Workspace pins order only their own project's list.
	// Archived sessions are out; office sessions too — modes are separate: the
	// 会话 list is build-mode content, office sessions show on the 工作 tab.
	const recents = useMemo(() => {
		if (summaryRecents) return summaryRecents;
		const all = projectList.flatMap((p) =>
			p.sessions
				.filter((s) => !s.archived)
				.map((s) => ({
					root: p.root,
					project: p.name,
					current: p.current,
					row: s,
				}))
		);
		// Same-second stamps fall back to project, then id — a tie must not
		// reorder between polls.
		all.sort(
			(a, b) =>
				b.row.updated_at - a.row.updated_at ||
				a.root.localeCompare(b.root) ||
				b.row.id - a.row.id
		);
		const pinned = all.filter((e) => e.row.pinned_global);
		const rest = all.filter((e) => !e.row.pinned_global).slice(0, RECENT_LIMIT);
		return [...pinned, ...rest];
	}, [projectList, summaryRecents]);

	// Projects open/close independently — opening one reveals its full
	// conversation list without switching the active workspace. The current
	// workspace's project starts expanded by default.
	const currentRoot = projects.find((p) => p.current)?.root;
	const [expanded, setExpanded] = useState<Set<string>>(() =>
		currentRoot ? new Set([currentRoot]) : new Set()
	);
	// When the current workspace changes (first tree load lands async, or a
	// switch lands), its project opens by default. A manual collapse still
	// wins — this only fires when `currentRoot` itself changes.
	const lastCurrentRef = useRef(currentRoot);
	useEffect(() => {
		if (!currentRoot || currentRoot === lastCurrentRef.current) return;
		lastCurrentRef.current = currentRoot;
		setExpanded((prev) => {
			if (prev.has(currentRoot)) return prev;
			const next = new Set(prev);
			next.add(currentRoot);
			return next;
		});
	}, [currentRoot]);
	const toggleProject = (root: string) => {
		setExpanded((prev) => {
			const next = new Set(prev);
			if (next.has(root)) next.delete(root);
			else next.add(root);
			return next;
		});
	};

	// 产物 list — office mode's counterpart of the project tree: files the
	// sessions produced, grouped by day (今天/昨天/date) like projects group
	// conversations. Refetches whenever the active view registers a new
	// artifact mid-turn, when a session's stamp bumps, or on tab entry.
	const viewArtifactCount = useActiveViewSelector((view) =>
		officeMode ? view.artifacts.length : 0
	);
	const sessionsStamp = officeMode
		? (officeProject?.sessions
				.map((s) => `${s.id}:${s.updated_at}`)
				.join('|') ?? '')
		: '';
	const [officeFiles, setOfficeFiles] = useState<RecentArtifact[] | null>(null);
	useEffect(() => {
		if (!officeMode) {
			setOfficeFiles(null);
			return;
		}
		let alive = true;
		void recentArtifacts()
			.then((r) => alive && setOfficeFiles(r))
			.catch(() => {});
		return () => {
			alive = false;
		};
	}, [officeMode, viewArtifactCount, sessionsStamp]);

	interface ArtGroup {
		key: string;
		label: string;
		/** Representative ext — drives the group's ExtIcon. */
		rep: string;
		files: RecentArtifact[];
	}
	interface ArtDayGroup {
		key: string;
		label: string;
		types: ArtGroup[];
	}
	const artifactGroups = useMemo<ArtDayGroup[]>(() => {
		// Top level: calendar days (今天/昨天/M月D日) — the time classification.
		const days: { key: string; label: string; files: RecentArtifact[] }[] = [];
		const startOfDay = (t: number) => {
			const d = new Date(t);
			d.setHours(0, 0, 0, 0);
			return d.getTime();
		};
		const today = startOfDay(Date.now());
		for (const f of officeFiles ?? []) {
			const day = startOfDay(f.mtime * 1000);
			const key = String(day);
			let g = days.find((x) => x.key === key);
			if (!g) {
				const diff = Math.round((today - day) / 86400000);
				const d = new Date(f.mtime * 1000);
				g = {
					key,
					label:
						diff <= 0
							? '今天'
							: diff === 1
								? '昨天'
								: d.getFullYear() === new Date().getFullYear()
									? `${d.getMonth() + 1}月${d.getDate()}日`
									: `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日`,
					files: [],
				};
				days.push(g);
			}
			g.files.push(f);
		}
		// Second level: inside each day, split into the fixed type buckets
		// (PDF / DOC / PPT / 素材 / 其他) — files keep mtime-desc order.
		return days.map((g) => ({
			key: g.key,
			label: g.label,
			types: ART_GROUPS.map((t) => ({
				...t,
				files: g.files.filter((f) => artCategory(f) === t.key),
			})).filter((t) => t.files.length > 0),
		}));
	}, [officeFiles]);
	// Only the newest day starts expanded — mirrors the current project
	// opening by default while older days stay folded.
	const [openArt, setOpenArt] = useState<Set<string> | null>(null);
	useEffect(() => {
		if (openArt === null && artifactGroups.length > 0) {
			setOpenArt(new Set([artifactGroups[0].key]));
		}
	}, [artifactGroups, openArt]);
	const toggleArtGroup = (key: string) =>
		setOpenArt((prev) => {
			const next = new Set(prev ?? []);
			if (next.has(key)) next.delete(key);
			else next.add(key);
			return next;
		});

	return (
		<TooltipProvider delayDuration={300}>
			<aside
				data-tauri-drag-region="deep"
				className={cn(
					`session-navigation-panel flex h-full w-full flex-none select-none
					flex-col border-hairline bg-panel text-neutral-800`,
					!framedShell && 'border-r'
				)}
			>
				<div
					data-tauri-drag-region="deep"
					className={cn(
						`session-navigation-surface flex h-9 flex-none cursor-default
						select-none items-center gap-2 border-hairline bg-panel px-3`,
						!framedShell && 'border-b'
					)}
				>
					{isMac && <div className="w-[55px] shrink-0" />}

					{!isMac && (
						<>
							<img
								className="session-navigation-brand h-4 w-4 shrink-0"
								src={iconUrl}
								alt=""
								draggable={false}
							/>
							<span
								className="session-navigation-brand min-w-0 truncate text-[12px]
									font-semibold tracking-tight text-neutral-700"
							>
								Husk
							</span>
						</>
					)}

					{/* Mode tabs — the 编程 ↔ 工作 switch lives in the sidebar header,
              not the composer. 工作 targets the built-in office workspace
              (~/.local/share/husk/office); 编程 returns to the most recent
              real project (the folder picker covers the no-project case). */}
					{onSwitchMode && (
						<Tabs
							value={officeMode ? 'office' : 'build'}
							onValueChange={(v) => {
								if (v === 'office') {
									if (officeProject) onSwitchMode(officeProject.root);
								} else {
									const target = projectList[0]?.root;
									if (target) onSwitchMode(target);
									else onPickWorkspace?.();
								}
							}}
							className="ml-auto"
						>
							<TabsList
								data-nodrag
								data-tauri-drag-region="false"
								className="relative h-auto gap-0 rounded-md
									bg-[color-mix(in_srgb,var(--husk-black)_5%,transparent)] p-0.5
									dark:bg-[var(--husk-border)]"
							>
								{/* Sliding active pill — triggers are fixed-width so the
                    translate is a constant. This theme overrides
                    rounded-md to 10px — the pill's 8px (= 10 − the 2px
                    p-0.5 ring) keeps the nested radii concentric. */}
								<span
									aria-hidden
									className={cn(
										`absolute inset-y-0.5 left-0.5 w-12 transform-gpu
										rounded-[8px] bg-white shadow-[0_1px_2px_rgba(0,0,0,0.08)]
										transition-transform duration-200 ease-out
										will-change-transform dark:bg-[var(--husk-hover)]
										dark:shadow-[0_1px_2px_rgba(0,0,0,0.6)]`,
										officeMode && 'translate-x-12'
									)}
								/>
								{/* Tooltip's Slot writes its own `data-state` (closed/open) onto
                    whatever it wraps. Wrapping the trigger directly overwrote the
                    tab's `active` state, so the selected label fell back to grey.
                    The hover wrapper keeps that attribute off the trigger. */}
								<TooltipSimple
									content={
										<ShortcutHint action="mode-build">
											编程 — 在项目中读写代码、运行命令
										</ShortcutHint>
									}
									side="bottom"
								>
									<span className="inline-flex" data-nodrag>
										<TabsTrigger
											value="build"
											data-nodrag
											aria-label="切换到编程模式"
											className="relative h-[22px] w-12 gap-1 rounded px-0 py-0
												text-[11px] font-normal text-text-secondary
												transition-none hover:text-neutral-700
												data-[state=active]:bg-transparent
												data-[state=active]:text-neutral-800
												data-[state=active]:shadow-none"
										>
											<Code className="h-3 w-3" />
											编程
										</TabsTrigger>
									</span>
								</TooltipSimple>
								{/* A disabled button swallows pointer events, so the hint hangs on a
                    wrapper that stays hoverable while the tab itself is disabled. */}
								<TooltipSimple
									content={
										<ShortcutHint action="mode-office">
											{officeProject
												? '工作 — 创建与编辑 Word / Excel / PPT'
												: '尚无工作目录'}
										</ShortcutHint>
									}
									side="bottom"
								>
									<span className="inline-flex" data-nodrag>
										<TabsTrigger
											value="office"
											data-nodrag
											aria-label="切换到工作模式"
											disabled={!officeProject}
											className="relative h-[22px] w-12 gap-1 rounded px-0 py-0
												text-[11px] font-normal text-text-secondary
												transition-none hover:text-neutral-700
												disabled:opacity-100 data-[state=active]:bg-transparent
												data-[state=active]:text-neutral-800
												data-[state=active]:shadow-none"
										>
											<FileText className="h-3 w-3" />
											工作
										</TabsTrigger>
									</span>
								</TooltipSimple>
							</TabsList>
						</Tabs>
					)}
				</div>

				<div className="flex min-h-0 flex-1 flex-col">
					{/* The merged-recents 会话 block is build-mode only — office mode's
              scroll area IS the office archive, so a cross-workspace top-6
              would just duplicate it (and leak project rows into a mode
              that hides them). */}
					{!officeMode && (
						<>
							<div
								className="flex flex-none items-center justify-between px-4 pb-1
									pt-3"
							>
								<span className="text-[13px] font-medium text-neutral-600">
									会话
								</span>
							</div>
							<div className="flex flex-none flex-col gap-0.5 px-2 pb-1">
								{recents.length === 0 && !projectsLoaded ? (
									initialBusyShown ? (
										<SidebarRowSkeleton rows={3} />
									) : null
								) : recents.length === 0 ? (
									<div
										className="flex h-8 flex-none shrink-0 items-center pl-7
											text-[13px] text-text-secondary"
									>
										暂无会话
									</div>
								) : (
									recents.map(({ root, project, current, row }) => (
										<SessionItem
											key={`${root}:${row.id}`}
											row={row}
											root={root}
											hint={current ? undefined : project}
											isActive={
												current && (activeId ? row.id === activeId : row.active)
											}
											live={current}
											pinMark="global"
											onOpen={() => onOpenSession(root, row.id)}
											actions={actions}
										/>
									))
								)}
							</div>
						</>
					)}

					{/* Section header —「项目」in build mode,「会话」in office mode
              (the scroll area below it is then the office session archive;
              no project tree shows there). "+" starts a session in the
              active workspace either way; the folder picks a project dir
              and so only exists in build mode. */}
					<div
						className={cn(
							'flex flex-none items-center justify-between px-4 pb-1',
							officeMode ? 'pt-3' : 'pt-2'
						)}
					>
						<span className="text-[13px] font-medium text-neutral-600">
							{officeMode ? '会话' : '项目'}
						</span>
						<div className="flex items-center gap-0.5">
							{hasWorkspace && (
								<Tooltip>
									<TooltipTrigger asChild>
										<button
											type="button"
											data-nodrag
											data-tauri-drag-region="false"
											onClick={onNew}
											aria-label="新建会话"
											className="flex h-5 w-5 items-center justify-center
												rounded-full text-neutral-500 transition-colors
												hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
												hover:text-neutral-700"
										>
											<Plus className="h-3.5 w-3.5" />
										</button>
									</TooltipTrigger>
									<TooltipContent side="right" className="text-xs">
										<ShortcutHint action="new-session">新建会话</ShortcutHint>
									</TooltipContent>
								</Tooltip>
							)}
							{onPickWorkspace && !officeMode && (
								<Tooltip>
									<TooltipTrigger asChild>
										<button
											type="button"
											data-nodrag
											data-tauri-drag-region="false"
											onClick={onPickWorkspace}
											aria-label="打开其他工作区"
											className="flex h-5 w-5 items-center justify-center
												rounded-full text-neutral-500 transition-colors
												hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
												hover:text-neutral-700"
										>
											<FolderPlus className="h-3.5 w-3.5" />
										</button>
									</TooltipTrigger>
									<TooltipContent side="right" className="text-xs">
										打开其他工作区
									</TooltipContent>
								</Tooltip>
							)}
						</div>
					</div>

					{officeMode ? (
						/* 工作: sessions shrink-wrap to a capped strip; the 产物 file
               tree owns the remaining scroller — the same "header then
               contents" split the 项目 tree gives build mode. */
						<>
							<div
								data-tauri-drag-region="false"
								className="max-h-[45%] flex-none overflow-y-auto"
								data-testid="sidebar-office-scroll"
								tabIndex={0}
								onScroll={(event) => loadReachedPage(event, paging)}
								onWheel={(event) => loadOnDownwardWheel(event, paging)}
								onKeyDown={(event) => loadOnPageKey(event, paging)}
							>
								<div className="flex flex-col gap-0.5 px-2 pb-1">
									{error && (
										<button
											type="button"
											onClick={onRetry}
											className="pl-7 text-[12px] text-text-secondary"
										>
											会话列表加载失败，重试
										</button>
									)}
									{!projectsLoaded && !officeProject ? (
										initialBusyShown ? (
											<SidebarRowSkeleton rows={2} />
										) : null
									) : !paging &&
									  officeProject &&
									  officeProject.sessions.every((s) => s.archived) ? (
										<div
											className="flex h-8 items-center pl-7 text-[13px]
												text-text-secondary"
										>
											暂无会话
										</div>
									) : (
										<SessionList
											sessions={officeProject?.sessions ?? []}
											root={officeProject?.root ?? ''}
											paging={paging}
											total={officeProject?.total}
											archivedTotal={officeProject?.archived_total}
											activeId={activeId}
											live
											pinMark="single"
											onOpenSession={onOpenSession}
											actions={actions}
										/>
									)}
								</div>
							</div>
							<div className="flex flex-none items-center px-4 pb-1 pt-2">
								<span className="text-[13px] font-medium text-neutral-600">
									产物
								</span>
							</div>
							{/* Same scroller rules as the project list — `false` outer,
                  `deep` inner so the empty tail still drags the window. */}
							<div
								data-tauri-drag-region="false"
								className="min-h-0 flex-1 transform-gpu overflow-y-auto"
							>
								<div
									data-tauri-drag-region="deep"
									className="flex min-h-full flex-col gap-0.5 px-2 pb-2"
								>
									{artifactGroups.length === 0 ? (
										<div
											className="flex h-8 items-center pl-7 text-[13px]
												text-text-secondary"
										>
											暂无产物
										</div>
									) : (
										artifactGroups.map((g) => (
											<OfficeFileGroup
												key={g.key}
												label={g.label}
												types={g.types}
												open={openArt?.has(g.key) ?? false}
												onToggle={() => toggleArtGroup(g.key)}
											/>
										))
									)}
								</div>
							</div>
						</>
					) : (
						/* Project list: the content area's only scroller. It must stay
                `false` — the scrollbar's mousedown targets this element, and a
                drag region here makes scrollbar drags move the window. The
                inner wrapper re-enables `deep` so row gaps and the empty tail
                still drag the window (rows opt out themselves). */
						<div
							data-tauri-drag-region="false"
							className="min-h-0 flex-1 transform-gpu overflow-y-auto"
							data-testid="sidebar-project-scroll"
							tabIndex={0}
							onScroll={(event) => loadReachedPage(event, paging)}
							onWheel={(event) => loadOnDownwardWheel(event, paging)}
							onKeyDown={(event) => loadOnPageKey(event, paging)}
						>
							<div
								data-tauri-drag-region="deep"
								className="flex min-h-full flex-col gap-0.5 px-2 pb-2"
							>
								{error && (
									<button
										type="button"
										onClick={onRetry}
										className="pl-7 text-[12px] text-text-secondary"
									>
										会话列表加载失败，重试
									</button>
								)}
								{projectList.length === 0 && !projectsLoaded ? (
									initialBusyShown ? (
										<SidebarRowSkeleton rows={3} />
									) : null
								) : projectList.length === 0 ? (
									// Same inset as 暂无会话 — the two empty states are the same kind
									// of thing, so they read at the same level.
									<div
										className="flex h-8 items-center pl-7 text-[13px]
											text-text-secondary"
									>
										暂无项目
									</div>
								) : (
									projectList.map((p) => (
										<ProjectItem
											key={p.root}
											project={p}
											paging={paging}
											open={expanded.has(p.root)}
											activeId={activeId}
											projectsLoaded={projectsLoaded}
											onToggle={() => toggleProject(p.root)}
											onOpenSession={onOpenSession}
											onSwitchWorkspace={onSwitchWorkspace}
											onRemoveWorkspace={onRemoveWorkspace}
											actions={actions}
										/>
									))
								)}
							</div>
						</div>
					)}
				</div>

				<div
					className="session-navigation-surface flex h-11 flex-none select-none
						items-center bg-panel px-4"
				>
					<div
						data-nodrag
						data-tauri-drag-region="false"
						className="flex items-center gap-3.5"
					>
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={onOpenSettings}
									aria-label="设置"
									className="rounded p-1 text-neutral-500 transition-colors
										hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
										hover:text-neutral-800"
								>
									<Settings className="h-4 w-4" />
								</button>
							</TooltipTrigger>
							<TooltipContent side="top" className="text-xs">
								<ShortcutHint action="settings">设置</ShortcutHint>
							</TooltipContent>
						</Tooltip>

						<Popover>
							<Tooltip>
								<TooltipTrigger asChild>
									<PopoverTrigger asChild>
										<button
											type="button"
											aria-label="通知"
											className="relative rounded p-1 text-neutral-500
												transition-colors
												hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
												hover:text-neutral-800"
										>
											<Bell className="h-4 w-4" />
											{unreadCount > 0 && (
												<span
													className="absolute -right-0.5 -top-0.5 h-3.5
														min-w-3.5 select-none rounded-full bg-red-500 px-0.5
														text-center text-[9px] font-semibold leading-[14px]
														text-zinc-50"
												>
													{unreadCount > 99 ? '99+' : unreadCount}
												</span>
											)}
										</button>
									</PopoverTrigger>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									通知
								</TooltipContent>
							</Tooltip>
							<PopoverContent
								side="top"
								align="start"
								className="w-72 overflow-hidden p-0 text-xs"
							>
								<div
									className="flex items-center justify-between px-3 pb-2 pt-2.5"
								>
									<div className={OVERLAY_HEADING}>通知</div>
									<div className="flex items-center gap-2.5">
										{unreadCount > 0 && (
											<button
												type="button"
												onClick={() => markAllRead()}
												className={cn(
													`cursor-pointer text-[11px] transition-colors
													hover:text-neutral-800`,
													INK_OVERLAY_SECONDARY
												)}
											>
												全部已读
											</button>
										)}
										{notifications.length > 0 && (
											<button
												type="button"
												onClick={() => clearAll()}
												className={cn(
													`cursor-pointer text-[11px] transition-colors
													hover:text-neutral-800`,
													INK_OVERLAY_SECONDARY
												)}
											>
												清空
											</button>
										)}
									</div>
								</div>
								{notifications.length === 0 ? (
									<div className={cn('px-3 pb-3', INK_OVERLAY_SECONDARY)}>
										暂无通知。
									</div>
								) : (
									<div
										data-tauri-drag-region="false"
										className={cn(
											'max-h-64 overflow-y-auto border-t',
											BORDER_OVERLAY_DIVIDER
										)}
									>
										{notifications.map((n) => (
											<button
												key={n.id}
												type="button"
												onClick={() => openNotification(n)}
												className={cn(
													`flex w-full cursor-pointer items-start gap-2 px-3
														py-2 text-left transition-colors`,
													SURFACE_OVERLAY_HOVER
												)}
											>
												{/* The dot keeps its slot once read, so the row's text
                            edge does not shift under the pointer. */}
												<span
													className={cn(
														'mt-1.5 h-1.5 w-1.5 flex-none rounded-full',
														n.read ? 'bg-transparent' : NOTIFY_DOT[n.kind]
													)}
												/>
												<span className="min-w-0 flex-1">
													<span
														className={cn(
															'block truncate',
															n.read
																? INK_OVERLAY_SECONDARY
																: 'font-medium text-neutral-800'
														)}
													>
														{titleFor(n)}
													</span>
													{n.body && (
														<span
															className={cn(
																'mt-0.5 line-clamp-2 block break-words',
																INK_OVERLAY_SECONDARY
															)}
															title={n.body}
														>
															{n.body}
														</span>
													)}
													<span
														className={cn(
															'mt-0.5 block text-[10.5px]',
															INK_OVERLAY_SECONDARY
														)}
													>
														{notifyStamp(n.ts)}
													</span>
												</span>
											</button>
										))}
									</div>
								)}
							</PopoverContent>
						</Popover>
					</div>
					{/* The terminal is a panel of the main column, not an app-level
              setting like the two on the left — it sits at the far end. */}
					{onToggleTerminal && (
						<div
							data-nodrag
							data-tauri-drag-region="false"
							className="ml-auto flex items-center"
						>
							<TerminalToggle open={terminalOpen} onToggle={onToggleTerminal} />
						</div>
					)}
				</div>
			</aside>
		</TooltipProvider>
	);
}

/** A project's (or office's) sessions: live rows first, archived ones folded
 *  under a collapsed 已归档 toggle at the bottom. */
function SessionList({
	sessions,
	root,
	paging,
	total,
	archivedTotal,
	activeId,
	live,
	small = false,
	pinMark = 'workspace',
	onOpenSession,
	actions,
}: {
	sessions: ProjectSessionRow[];
	root: string;
	paging?: SidebarPaging;
	total?: number;
	archivedTotal?: number;
	activeId?: number;
	/** The rows belong to the active workspace (fork/delete available). */
	live: boolean;
	small?: boolean;
	/** See `SessionItem.pinMark`. */
	pinMark?: PinMark;
	onOpenSession: (root: string, id: number) => void;
	actions: SessionActions;
}) {
	const [showArchived, setShowArchived] = useState(false);
	useEffect(() => {
		if (!root) return;
		const stopVisible = paging?.ensurePage(root);
		const stopArchived = showArchived
			? paging?.ensurePage(root, true)
			: undefined;
		return () => {
			stopVisible?.();
			stopArchived?.();
		};
	}, [paging?.ensurePage, paging?.generation, root, showArchived]);
	const visible = sessions.filter((s) => !s.archived);
	const archived = sessions.filter((s) => s.archived);
	const archiveCount = archivedTotal ?? archived.length;
	const item = (s: ProjectSessionRow) => (
		<SessionItem
			key={s.id}
			row={s}
			root={root}
			isActive={live && (activeId ? s.id === activeId : s.active)}
			live={live}
			pinMark={pinMark}
			onOpen={() => onOpenSession(root, s.id)}
			actions={actions}
		/>
	);
	return (
		<>
			{/* The active row can live outside the prefix, including in archives. */}
			{!showArchived && archived.filter((s) => s.active).map(item)}
			{visible.map(item)}
			{paging?.get(root).loaded && (total ?? visible.length) === 0 && (
				<div
					className="flex h-7 items-center pl-7 text-[12px] text-text-secondary"
				>
					暂无对话记录
				</div>
			)}
			<SidebarPageTail root={root} paging={paging} />
			{archiveCount > 0 && (
				<button
					type="button"
					data-nodrag
					data-tauri-drag-region="false"
					onClick={() => setShowArchived((v) => !v)}
					className={cn(
						`flex flex-none items-center gap-1 pl-7 text-text-secondary
						hover:text-neutral-700`,
						small ? 'h-7 text-[12px]' : 'h-8 text-[13px]'
					)}
				>
					{showArchived ? (
						<ChevronDown className="h-3 w-3" />
					) : (
						<ChevronRight className="h-3 w-3" />
					)}
					已归档 {archiveCount}
				</button>
			)}
			{showArchived && archived.map(item)}
			{showArchived && <SidebarPageTail root={root} archived paging={paging} />}
		</>
	);
}

function SessionItem({
	row,
	root,
	isActive,
	hint,
	live,
	pinMark,
	onOpen,
	actions,
}: {
	row: SessionRow;
	root: string;
	isActive: boolean;
	/** Trailing project label — set on the cross-project recent list so rows
	 * from different projects stay tellable apart. */
	hint?: string;
	/** The row belongs to the active workspace. Fork/delete need its live
	 *  actor (ids are per-workspace), so other projects' rows omit them. */
	live: boolean;
	/** Which pin the row shows and its menu offers — each list orders by its
	 *  own. `single`: office mode is one workspace, so there is no workspace vs
	 *  global distinction — one plain 置顶 (the workspace pin) is the whole of it. */
	pinMark: PinMark;
	onOpen: () => void;
	actions: SessionActions;
}) {
	const displayTitle =
		row.title === 'new session' ? '新会话' : row.title || `会话 ${row.id}`;
	const [menuOpen, setMenuOpen] = useState(false);
	// A row whose session delegated work reads differently from one merely
	// streaming: the orb names which of the two is going on.
	const subagentRunning = useHasRunningChildren(root, row.id);
	const pinnedHere = pinMark === 'global' ? row.pinned_global : row.pinned;

	const copyId = () => {
		void navigator.clipboard
			.writeText(String(row.id))
			.then(() => toast.success(`已复制会话 ID：${row.id}`))
			.catch(() => toast.error('复制失败'));
	};

	// Identical items for the "···" dropdown and the row's right-click menu.
	// The two Radix families keep separate `Menu` contexts, so the same JSX
	// can't be shared — one builder renders the items with whichever
	// primitives the host menu provides.
	const renderActionItems = (M: {
		Item: typeof DropdownMenuItem;
		Separator: typeof DropdownMenuSeparator;
		Sub: typeof DropdownMenuSub;
		SubTrigger: typeof DropdownMenuSubTrigger;
		SubContent: typeof DropdownMenuSubContent;
	}) => {
		const item = (
			icon: ReactNode,
			label: string,
			run: () => void,
			danger = false
		) => (
			<M.Item
				className={cn(
					'cursor-pointer gap-2 text-xs',
					danger && INK_OVERLAY_DESTRUCTIVE
				)}
				onClick={(e) => {
					e.stopPropagation();
					run();
				}}
			>
				{icon}
				{label}
			</M.Item>
		);
		const icon = 'h-3.5 w-3.5';
		return (
			<>
				{actions.rename &&
					item(<Pen className={icon} />, '重命名', () =>
						actions.rename?.(root, row)
					)}
				{!row.archived &&
					pinMark === 'single' &&
					item(
						<Bookmark className={cn(icon, row.pinned && 'fill-current')} />,
						row.pinned ? '取消置顶' : '置顶',
						() => actions.pin?.(root, row.id, !row.pinned)
					)}
				{!row.archived && pinMark !== 'single' && (
					<M.Sub>
						<M.SubTrigger className="gap-2 text-xs">
							<Bookmark
								className={cn(
									icon,
									(row.pinned || row.pinned_global) && 'fill-current'
								)}
							/>
							置顶
						</M.SubTrigger>
						<M.SubContent className="min-w-[140px]">
							{item(
								<Folder className={icon} />,
								row.pinned ? '取消工作区置顶' : '在工作区置顶',
								() => actions.pin?.(root, row.id, !row.pinned)
							)}
							{item(
								<Globe className={icon} />,
								row.pinned_global ? '取消全局置顶' : '在全局置顶',
								() => actions.pinGlobal?.(root, row.id, !row.pinned_global)
							)}
						</M.SubContent>
					</M.Sub>
				)}
				{live &&
					actions.fork &&
					item(<GitFork className={icon} />, 'Fork 会话', () =>
						actions.fork?.(row.id)
					)}
				<M.Separator />
				{item(<Copy className={icon} />, '复制会话 ID', copyId)}
				{actions.exportRecord &&
					item(<Download className={icon} />, '导出记录', () =>
						actions.exportRecord?.(root, row.id)
					)}
				<M.Separator />
				{item(
					<Archive className={icon} />,
					row.archived ? '取消归档' : '归档',
					() => actions.archive?.(root, row, !row.archived)
				)}
				{live &&
					actions.delete &&
					item(
						<Bin className={icon} />,
						'删除会话',
						() => actions.delete?.(row.id),
						true
					)}
			</>
		);
	};

	const rowEl = (
		<div
			role="button"
			tabIndex={0}
			data-nodrag
			data-tauri-drag-region="false"
			onClick={onOpen}
			onKeyDown={(e) => {
				if (e.key === 'Enter') onOpen();
			}}
			className={cn(
				`group relative flex h-8 min-h-8 w-full flex-none shrink-0
				cursor-pointer select-none items-center gap-1.5 rounded-md pl-7 pr-2
				text-[13px] transition-colors`,
				isActive
					? `bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)]
						text-neutral-900`
					: `font-normal text-neutral-600
						hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
						hover:text-neutral-900`,
				row.archived && 'text-text-secondary'
			)}
		>
			<span className="min-w-0 flex-1 truncate text-left">{displayTitle}</span>
			{pinnedHere && (
				<Bookmark
					className="h-3 w-3 flex-none fill-current text-amber-500
						dark:text-amber-400"
				/>
			)}
			{hint && (
				<span
					className="max-w-[64px] flex-none truncate text-[11px]
						text-text-secondary"
				>
					{hint}
				</span>
			)}
			{/* Trailing: the running orb keeps its slot; the more menu floats
          over the row's right edge on hover/focus — a non-running row
          reserves nothing (the old fixed w-5 cell read as a dead gap). */}
			{row.running && (
				<span className="relative h-5 w-5 flex-none">
					{!menuOpen && (
						<Orb
							variant={subagentRunning ? 'G2' : 'C3'}
							size={14}
							label={subagentRunning ? '子代理运行中' : undefined}
							className="absolute inset-0 flex items-center justify-center
								text-neutral-800 group-hover:invisible
								group-focus-visible:invisible"
						/>
					)}
				</span>
			)}
			<DropdownMenu onOpenChange={setMenuOpen}>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						data-nodrag
						data-tauri-drag-region="false"
						aria-label="会话操作"
						onClick={(e) => e.stopPropagation()}
						onKeyDown={(e) => e.stopPropagation()}
						// right-2 lands it exactly on the orb's rest position
						// (pr-2 + w-5); the button also backs itself with a panel
						// blur so a long title doesn't bleed through.
						className="invisible absolute right-2 top-1/2 flex h-5 w-5
							-translate-y-1/2 items-center justify-center rounded-full
							bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)]
							text-neutral-500 backdrop-blur-sm
							hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)]
							hover:text-neutral-700 group-hover:visible
							group-focus-visible:visible data-[state=open]:visible"
					>
						<MoreHorizontal className="h-3.5 w-3.5" />
					</button>
				</DropdownMenuTrigger>
				{/* The menu portals out of the DOM but stays inside this row in the
            React tree, so its clicks and keys bubble to the row's
            onClick/onKeyDown and open the session. Items stopped that
            themselves; the 置顶 sub-trigger, separators, padding and Enter on
            an item did not — the "clicking 置顶 reloads the chat" bug. Stop
            them all here, once. */}
				<DropdownMenuContent
					side="right"
					align="start"
					sideOffset={4}
					className="min-w-[150px]"
					onClick={(e) => e.stopPropagation()}
					onKeyDown={(e) => e.stopPropagation()}
				>
					{renderActionItems({
						Item: DropdownMenuItem,
						Separator: DropdownMenuSeparator,
						Sub: DropdownMenuSub,
						SubTrigger: DropdownMenuSubTrigger,
						SubContent: DropdownMenuSubContent,
					})}
				</DropdownMenuContent>
			</DropdownMenu>
		</div>
	);

	// Right-click on the row opens the same menu the "···" button does.
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>{rowEl}</ContextMenuTrigger>
			<ContextMenuContent className="min-w-[150px]">
				{renderActionItems({
					Item: ContextMenuItem as typeof DropdownMenuItem,
					Separator: ContextMenuSeparator as typeof DropdownMenuSeparator,
					Sub: ContextMenuSub as typeof DropdownMenuSub,
					SubTrigger: ContextMenuSubTrigger as typeof DropdownMenuSubTrigger,
					SubContent: ContextMenuSubContent as typeof DropdownMenuSubContent,
				})}
			</ContextMenuContent>
		</ContextMenu>
	);
}

/** One day group in the office 产物 tree — mirrors `ProjectItem`: a
 * collapsible header (chevron + folder + label + count) whose children are
 * type sub-groups (PDF/DOC/PPT/素材), each collapsible down to file rows. */
function OfficeFileGroup({
	label,
	types,
	open,
	onToggle,
}: {
	label: string;
	types: { key: string; label: string; rep: string; files: RecentArtifact[] }[];
	open: boolean;
	onToggle: () => void;
}) {
	const count = types.reduce((n, t) => n + t.files.length, 0);
	// Type sub-groups start COLLAPSED — expanding a day shouldn't flood the
	// sidebar with every file at once; the user opens the bucket they need.
	const [closedTypes, setClosedTypes] = useState<Set<string>>(
		() => new Set(types.map((t) => t.key))
	);
	const toggleType = (key: string) =>
		setClosedTypes((prev) => {
			const next = new Set(prev);
			if (next.has(key)) next.delete(key);
			else next.add(key);
			return next;
		});
	return (
		<div className="flex flex-col">
			<div
				data-nodrag
				data-tauri-drag-region="false"
				className={cn(
					'session-navigation-surface',
					open && 'sticky top-0 z-10 bg-panel pb-0.5'
				)}
			>
				<div
					role="button"
					tabIndex={0}
					data-nodrag
					data-tauri-drag-region="false"
					onClick={onToggle}
					onKeyDown={(e) => {
						if (e.key === 'Enter' || e.key === ' ') {
							e.preventDefault();
							onToggle();
						}
					}}
					className="group flex w-full cursor-pointer items-center gap-1.5
						rounded-md px-2 py-1.5 text-left text-[13px] text-neutral-800
						transition-colors
						hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
				>
					<span className="shrink-0 text-neutral-500">
						{open ? (
							<ChevronDown className="h-3.5 w-3.5" />
						) : (
							<ChevronRight className="h-3.5 w-3.5" />
						)}
					</span>
					<span className="shrink-0 text-neutral-500">
						{open ? (
							<FolderOpen className="h-4 w-4" />
						) : (
							<Folder className="h-4 w-4" />
						)}
					</span>
					<span className="min-w-0 flex-1 truncate font-medium">{label}</span>
					<span
						className="flex-none text-[11px] tabular-nums text-text-secondary"
					>
						{count}
					</span>
				</div>
			</div>
			{open && (
				<div className="flex flex-col gap-0.5">
					{types.map((t) => {
						const tOpen = !closedTypes.has(t.key);
						return (
							<div key={t.key} className="flex flex-col">
								<div
									role="button"
									tabIndex={0}
									data-nodrag
									data-tauri-drag-region="false"
									onClick={() => toggleType(t.key)}
									onKeyDown={(e) => {
										if (e.key === 'Enter' || e.key === ' ') {
											e.preventDefault();
											toggleType(t.key);
										}
									}}
									className="group relative flex h-8 min-h-8 w-full
										cursor-pointer select-none items-center gap-1.5 rounded-md
										pl-4 pr-2 text-[13px] text-neutral-600 transition-colors
										hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
										hover:text-neutral-900"
								>
									<span className="shrink-0 text-neutral-400">
										{tOpen ? (
											<ChevronDown className="h-3 w-3" />
										) : (
											<ChevronRight className="h-3 w-3" />
										)}
									</span>
									<ExtIcon ext={t.rep} />
									<span className="min-w-0 flex-1 truncate text-[12px]">
										{t.label}
									</span>
									<span
										className="flex-none text-[11px] tabular-nums
											text-text-secondary"
									>
										{t.files.length}
									</span>
								</div>
								{tOpen && (
									<div className="flex flex-col gap-0.5">
										{t.files.map((f) => (
											<OfficeFileRow key={f.path} file={f} />
										))}
									</div>
								)}
							</div>
						);
					})}
				</div>
			)}
		</div>
	);
}

/** One produced file row under an 产物 date group — click opens it in the
 * OS; the hover chip carries 位置/另存为 like the artifacts dock. */
function OfficeFileRow({ file }: { file: RecentArtifact }) {
	const dir = file.path.includes('/')
		? file.path.slice(0, file.path.lastIndexOf('/'))
		: '';
	const open = () =>
		void openPath(file.path).catch((err) =>
			toast.error(`打开失败：${err instanceof Error ? err.message : err}`)
		);
	return (
		<div
			role="button"
			tabIndex={0}
			data-nodrag
			data-tauri-drag-region="false"
			onClick={open}
			onKeyDown={(e) => {
				if (e.key === 'Enter') open();
			}}
			title={file.path}
			className="group relative flex h-8 min-h-8 w-full cursor-pointer
				select-none items-center gap-1.5 rounded-md pl-9 pr-2 text-[13px]
				text-neutral-600 transition-colors
				hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
				hover:text-neutral-900"
		>
			<ExtIcon ext={file.name.split('.').pop() ?? ''} />
			<span className="min-w-0 flex-1 truncate">{file.name}</span>
			{dir && (
				<span
					className="max-w-[72px] flex-none truncate text-[11px]
						text-text-secondary"
				>
					{dir}
				</span>
			)}
			{/* Floating hover actions — same pattern as the dock: absolute over
          the row's right edge, no reserved space. */}
			<span
				className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center
					gap-0.5 rounded-full
					bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5
					backdrop-blur-sm group-hover:flex group-focus-visible:flex"
			>
				<button
					type="button"
					aria-label="打开"
					onClick={(e) => {
						e.stopPropagation();
						open();
					}}
					className="flex h-5 w-5 items-center justify-center rounded-full
						text-neutral-500
						hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)]
						hover:text-neutral-700"
				>
					<ExternalLink className="h-3 w-3" />
				</button>
				<button
					type="button"
					aria-label="另存为"
					onClick={(e) => {
						e.stopPropagation();
						void saveArtifact(file.path).catch((err) =>
							toast.error(
								`另存为失败：${err instanceof Error ? err.message : err}`
							)
						);
					}}
					className="flex h-5 w-5 items-center justify-center rounded-full
						text-neutral-500
						hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)]
						hover:text-neutral-700"
				>
					<Download className="h-3 w-3" />
				</button>
				<button
					type="button"
					aria-label="在文件夹中显示"
					onClick={(e) => {
						e.stopPropagation();
						void revealPath(file.path).catch((err) =>
							toast.error(
								`打开文件夹失败：${err instanceof Error ? err.message : err}`
							)
						);
					}}
					className="flex h-5 w-5 items-center justify-center rounded-full
						text-neutral-500
						hover:bg-[color-mix(in_srgb,var(--husk-n300)_60%,transparent)]
						hover:text-neutral-700"
				>
					<FolderOpen className="h-3 w-3" />
				</button>
			</span>
		</div>
	);
}

/** One project row: click toggles its full conversation list; non-active
 * projects expose a hover "switch to this project" action. */
function ProjectItem({
	project,
	paging,
	open,
	activeId,
	projectsLoaded,
	onToggle,
	onOpenSession,
	onSwitchWorkspace,
	onRemoveWorkspace,
	actions,
}: {
	project: ProjectOverview & { total?: number; archived_total?: number };
	paging?: SidebarPaging;
	open: boolean;
	activeId?: number;
	projectsLoaded: boolean;
	onToggle: () => void;
	onOpenSession: (root: string, id: number) => void;
	onSwitchWorkspace?: (path: string) => void;
	onRemoveWorkspace?: (project: ProjectOverview) => void;
	actions: SessionActions;
}) {
	return (
		<div className="flex flex-col">
			<div
				data-nodrag
				data-tauri-drag-region="false"
				className={cn(
					'session-navigation-surface',
					open && 'sticky top-0 z-10 bg-panel pb-0.5'
				)}
			>
				<TooltipSimple content={project.root} side="right" sideOffset={8}>
					<div
						role="button"
						tabIndex={0}
						data-nodrag
						data-tauri-drag-region="false"
						onClick={onToggle}
						onKeyDown={(e) => {
							if (e.key === 'Enter' || e.key === ' ') {
								e.preventDefault();
								onToggle();
							}
						}}
						className="group relative flex w-full cursor-pointer items-center
							gap-1.5 rounded-md px-2 py-1.5 text-left text-[13px]
							text-neutral-800 transition-colors
							hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]"
					>
						<span className="shrink-0 text-neutral-500">
							{open ? (
								<ChevronDown className="h-3.5 w-3.5" />
							) : (
								<ChevronRight className="h-3.5 w-3.5" />
							)}
						</span>
						<span className="shrink-0 text-neutral-500">
							{open ? (
								<FolderOpen className="h-4 w-4" />
							) : (
								<Folder className="h-4 w-4" />
							)}
						</span>
						<span className="min-w-0 flex-1 truncate font-medium">
							{project.name}
						</span>
						{/* Hover actions float over the count badge / current dot —
                `invisible`-in-flow used to reserve ~40px of dead space on
                every row. */}
						<span
							className="absolute right-1 top-1/2 hidden -translate-y-1/2
								items-center gap-0.5 rounded-full
								bg-[color-mix(in_srgb,var(--husk-panel)_85%,transparent)] p-0.5
								backdrop-blur-sm group-hover:flex group-focus-visible:flex"
						>
							{!project.current && onSwitchWorkspace && (
								<button
									type="button"
									data-nodrag
									data-tauri-drag-region="false"
									aria-label="切换到此项目"
									onClick={(e) => {
										e.stopPropagation();
										onSwitchWorkspace(project.root);
									}}
									onKeyDown={(e) => e.stopPropagation()}
									className="flex h-5 w-5 flex-none items-center justify-center
										rounded-full text-neutral-500
										hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
										hover:text-neutral-700"
								>
									<FolderOpen className="h-3.5 w-3.5" />
								</button>
							)}
							{onRemoveWorkspace && (
								<button
									type="button"
									data-nodrag
									data-tauri-drag-region="false"
									aria-label="移除项目"
									onClick={(e) => {
										e.stopPropagation();
										onRemoveWorkspace(project);
									}}
									onKeyDown={(e) => e.stopPropagation()}
									className="flex h-5 w-5 flex-none items-center justify-center
										rounded-full text-neutral-500
										hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
										hover:text-neutral-700"
								>
									<X className="h-3.5 w-3.5" />
								</button>
							)}
						</span>
						{/* Archived sessions are out of sight, so out of the count too. */}
						{(project.total ??
							project.sessions.filter((s) => !s.archived).length) > 0 && (
							<span
								className="flex-none text-[11px] tabular-nums
									text-text-secondary"
							>
								{project.total ??
									project.sessions.filter((s) => !s.archived).length}
							</span>
						)}
						{project.current && (
							<span className="h-1.5 w-1.5 shrink-0 rounded-full bg-neutral-800" />
						)}
					</div>
				</TooltipSimple>
			</div>

			{open && (
				<div className="flex flex-col gap-0.5">
					{!paging && project.sessions.length === 0 && !projectsLoaded ? (
						<SidebarRowSkeleton rows={2} small />
					) : !paging && project.sessions.length === 0 ? (
						<div
							className="flex h-7 items-center pl-7 text-[12px]
								text-text-secondary"
						>
							暂无对话记录
						</div>
					) : (
						<SessionList
							sessions={project.sessions}
							root={project.root}
							paging={paging}
							total={project.total}
							archivedTotal={project.archived_total}
							activeId={activeId}
							live={project.current}
							small
							onOpenSession={onOpenSession}
							actions={actions}
						/>
					)}
				</div>
			)}
		</div>
	);
}
