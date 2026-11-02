// Settings page — full-window takeover inside the main window (Zed-style):
// clicking the sidebar gear swaps the chat layout for a settings view with
// its own left nav + scrollable content, same window, same session running
// underneath. Esc or "返回工作区" goes back.

import { useEffect, useRef, useState } from 'react';
import {
	ArrowLeft,
	Bot,
	Brain,
	Cpu,
	FileText,
	FlaskConical,
	Info,
	ListCheck,
	Palette,
	Plug,
	SlidersHorizontal,
	Sparkles,
	Terminal,
	Wrench,
	ChartColumn,
	Keyboard,
} from '@keyline-icons/react';
import { cn } from '@/lib/utils';
import { WindowControls } from '@/components/window-controls';
import { framedShell, isMac } from '@/lib/platform';
import { GeneralPane } from '@/features/settings/pages/general/index';
import { DeveloperPane } from '@/features/settings/pages/general/developer';
import { AppearanceSettings } from '@/features/settings/pages/appearance/index';
import { TerminalSettings } from '@/features/settings/pages/terminal/index';
import { ShortcutsSettings } from '@/features/settings/pages/shortcuts/index';
import { StatsPane } from '@/features/settings/pages/stats/index';
import { AboutSettings } from '@/features/settings/pages/about/index';
import {
	AgentSettings,
	type AgentTab,
} from '@/features/settings/pages/agent/index';
import { getDevConfig } from '@/lib/agent-ipc/sessions';

type SettingsTab =
	| 'general'
	| 'appearance'
	| 'terminal'
	| 'shortcuts'
	| 'developer'
	| 'stats'
	| AgentTab
	| 'about';

const NAV_GROUPS: {
	label: string;
	items: { key: SettingsTab; label: string; icon: React.ReactNode }[];
}[] = [
	{
		label: '基础设置',
		items: [
			{
				key: 'general',
				label: '常规',
				icon: <SlidersHorizontal className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'appearance',
				label: '外观',
				icon: <Palette className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'terminal',
				label: '终端',
				icon: <Terminal className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'shortcuts',
				label: '快捷键',
				icon: <Keyboard className="h-4 w-4 shrink-0" />,
			},
		],
	},
	{
		label: '智能体',
		items: [
			{
				key: 'instructions',
				label: '指令',
				icon: <FileText className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'rules',
				label: '规则',
				icon: <ListCheck className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'model',
				label: '模型',
				icon: <Cpu className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'memory',
				label: '记忆',
				icon: <Brain className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'skills',
				label: '技能',
				icon: <Sparkles className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'plugins',
				label: '插件',
				icon: <Plug className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'mcp',
				label: '连接器',
				icon: <Wrench className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'subagent',
				label: '子代理',
				icon: <Bot className="h-4 w-4 shrink-0" />,
			},
		],
	},
	{
		label: '系统',
		items: [
			{
				key: 'stats',
				label: '统计',
				icon: <ChartColumn className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'developer',
				label: '开发者选项',
				icon: <FlaskConical className="h-4 w-4 shrink-0" />,
			},
			{
				key: 'about',
				label: '关于',
				icon: <Info className="h-4 w-4 shrink-0" />,
			},
		],
	},
];

export function SettingsPage({
	onClose,
	initialTab = 'general',
}: {
	onClose: () => void;
	/** Where the overlay opens — a command can ask for 技能 / 记忆 directly. The
	 *  shell mounts this fresh on every open, so this is a starting point, not a
	 *  controlled value. */
	initialTab?: SettingsTab;
}) {
	const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);
	const [devUnlocked, setDevUnlocked] = useState(false);
	const contentRef = useRef<HTMLDivElement>(null);
	// Esc goes back to the workspace — same reflex as every full-screen pane.
	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key === 'Escape') onClose();
		};
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	}, [onClose]);

	// Read the persisted unlock flag once on mount. The dev pane is hidden
	// by default — opting in is a manual edit of
	// `~/.config/husk/settings.toml` (`developer_ui = true`), so a reload is
	// always required anyway; no event subscription needed.
	useEffect(() => {
		void getDevConfig()
			.then((c) => setDevUnlocked(c.developer_ui === true))
			.catch(() => {});
	}, []);

	// Each tab opens at the top — the container keeps its scrollTop otherwise.
	useEffect(() => {
		contentRef.current?.scrollTo({ top: 0 });
	}, [activeTab]);

	const tabMeta = NAV_GROUPS.flatMap((g) => g.items).find(
		(i) => i.key === activeTab
	);

	return (
		<div
			className={cn(
				`settings-layout relative flex h-full w-full select-none
				overflow-hidden`,
				framedShell ? 'main-workspace-frame bg-panel' : 'bg-white'
			)}
		>
			{/* Top draggable strip across the right area + window controls anchored at top-right */}
			<div
				data-tauri-drag-region="deep"
				className="absolute left-[240px] right-0 top-0 z-30 flex h-9 select-none
					items-center justify-end px-3"
			>
				<WindowControls />
			</div>

			{/* Left nav — same rail as session sidebar: panel bg, white active card */}
			<aside
				data-tauri-drag-region="deep"
				className={cn(
					'flex h-full w-[240px] flex-none select-none flex-col justify-between',
					framedShell ? 'bg-transparent' : 'border-r border-hairline bg-panel'
				)}
			>
				<div className="flex min-h-0 flex-1 flex-col">
					{/* Top title strip matching the main sidebar */}
					<div
						data-tauri-drag-region="deep"
						className={cn(
							`flex h-9 flex-none cursor-default select-none items-center gap-2
							px-3`,
							framedShell ? 'bg-transparent' : 'bg-panel'
						)}
					>
						{/* The traffic lights end near 72px; this leaves a small gap before the title. */}
						{isMac && <div className="w-[68px] shrink-0" />}
						<span
							className="text-[12px] font-semibold tracking-tight
								text-neutral-700"
						>
							设置
						</span>
					</div>

					<div className="flex flex-none flex-col px-3 pt-3">
						<button
							type="button"
							onClick={onClose}
							className="flex h-8 cursor-pointer items-center gap-2 rounded-lg
								px-2 text-[13px] font-medium text-neutral-500 transition-colors
								hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
								hover:text-neutral-800"
						>
							<ArrowLeft className="h-4 w-4 shrink-0" />
							返回工作区
						</button>
					</div>

					{/* Same scroller rules as the session sidebar — `false` outer so a
              scrollbar drag can't move the window, `deep` inner so row gaps and
              the empty tail still drag it. */}
					<div
						data-settings-nav
						data-tauri-drag-region="false"
						className="min-h-0 flex-1 overflow-y-auto px-3 pb-3 pt-3"
					>
						<div
							data-tauri-drag-region="deep"
							className="flex min-h-full flex-col gap-4"
						>
							{NAV_GROUPS.map((g) => (
								<div key={g.label} className="flex flex-col gap-1">
									<span
										className="px-3 pb-1 text-[11px] font-medium tracking-wide
											text-neutral-500"
									>
										{g.label}
									</span>
									{g.items
										.filter((item) => item.key !== 'developer' || devUnlocked)
										.map((item) => (
											<button
												key={item.key}
												type="button"
												onClick={() => setActiveTab(item.key)}
												className={cn(
													`flex h-9 w-full cursor-pointer items-center gap-2.5
													rounded-lg px-3 text-[13px] transition-all`,
													activeTab === item.key
														? // Same recipe as the session sidebar's active row:
															// 6% black wash + bright text — the vars lift it in
															// both themes, no dark: override needed.
															`bg-[color-mix(in_srgb,var(--husk-black)_6%,transparent)]
																text-neutral-900`
														: `font-normal text-neutral-600
															hover:bg-[color-mix(in_srgb,var(--husk-black)_4%,transparent)]
															hover:text-neutral-900`
												)}
											>
												{item.icon}
												<span>{item.label}</span>
											</button>
										))}
								</div>
							))}
						</div>
					</div>
				</div>
			</aside>

			{/* Content — big page title + grouped setting rows. */}
			<div
				ref={contentRef}
				className={cn(
					'no-scrollbar min-h-0 min-w-0 flex-1 overflow-y-auto bg-white',
					framedShell && 'm-2 rounded-lg border border-hairline'
				)}
			>
				<div className="mx-auto w-full max-w-3xl px-6 py-8 pb-16 sm:px-10">
					<h1
						className="mb-6 text-xl font-semibold tracking-tight
							text-neutral-900"
					>
						{tabMeta?.label}
					</h1>

					{activeTab === 'general' ? (
						<GeneralPane />
					) : activeTab === 'stats' ? (
						<StatsPane />
					) : activeTab === 'developer' ? (
						<DeveloperPane />
					) : activeTab === 'appearance' ? (
						<AppearanceSettings />
					) : activeTab === 'terminal' ? (
						<TerminalSettings />
					) : activeTab === 'shortcuts' ? (
						<ShortcutsSettings />
					) : activeTab === 'about' ? (
						<AboutSettings />
					) : (
						<AgentSettings tab={activeTab as AgentTab} />
					)}
				</div>
			</div>
		</div>
	);
}
