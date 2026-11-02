// ModelPicker — what answers and how hard it thinks, in one popover with two
// levels: the root states the current pair, and each row drills into its own
// list with a back arrow at the top-left. The trigger keeps the same summary
// (model + tier) it always had.

import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { TruncatedText } from '@/components/ui/truncated-text';
import { ReasoningEffort } from '@/components/ui/reasoning-effort';
import { TooltipSimple } from '@/components/ui/tooltip';
import { THINKING_LEVELS, thinkingTone } from '@/lib/thinking';
import { groupOptions } from '@/lib/group-options';
import { cn } from '@/lib/utils';
import {
	INK_OVERLAY_SECONDARY,
	OVERLAY_HEADING,
	SURFACE_OVERLAY_HOVER,
} from '@/lib/theme';
import type { ToolbarDensity } from '@/features/chat/components/composer-bar/toolbar-density';
import {
	Bot,
	Brain,
	Check,
	ChevronLeft,
	ChevronRight,
	ChevronsUpDown,
	Zap,
} from '@keyline-icons/react';
import type * as agent from '@/lib/agent-ipc/index';

/** The picker's primary label: the `name` set in settings, falling back to the
 *  wire id when the config declares none. */
export function modelLabel(m: agent.ModelItem): string {
	return m.name?.trim() || m.model;
}

/** The picker's small second line — the raw model id, what actually goes on
 *  the wire and what `/model` expects. Empty when the id IS the label, so a
 *  nameless model never prints its own name twice; the provider is the row's
 *  section heading, not a sub-label. */
function modelSubLabel(m: agent.ModelItem): string {
	return modelLabel(m) !== m.model ? m.model : '';
}

/** Full name of a thinking tier — `THINKING_LEVELS` is the one list. */
function tierLabel(value: string): string {
	return THINKING_LEVELS.find((level) => level.value === value)?.label ?? value;
}

const FAST_INK = 'text-purple-700 dark:text-purple-300';

function FastIndicator({ className }: { className: string }) {
	return (
		<span
			role="img"
			aria-label="Fast 已开启"
			className="inline-flex shrink-0 items-center"
		>
			<Zap aria-hidden="true" className={cn(FAST_INK, className)} />
		</span>
	);
}

/** A row in the root level: label on the left, the current value on the right. */
function SummaryRow({
	icon,
	label,
	value,
	status,
	disabled,
	onOpen,
}: {
	icon: React.ReactNode;
	label: string;
	value: string;
	status?: React.ReactNode;
	disabled?: boolean;
	onOpen: () => void;
}) {
	return (
		<DropdownMenuItem
			disabled={disabled}
			// The row only navigates: the popover must stay open for the level it
			// opens (Radix closes a menu on select by default).
			onSelect={(e) => {
				e.preventDefault();
				onOpen();
			}}
			className="flex min-w-0 cursor-pointer items-center gap-2 px-2 py-2"
		>
			<span className={cn('flex-none', INK_OVERLAY_SECONDARY)}>{icon}</span>
			<span className={cn('flex-none text-xs', INK_OVERLAY_SECONDARY)}>
				{label}
			</span>
			<span className="min-w-0 flex-1" />
			<TruncatedText
				side="left"
				text={value}
				className="min-w-0 truncate text-xs font-medium text-neutral-900"
			/>
			{status}
			<ChevronRight
				className={cn('size-3.5 flex-none', INK_OVERLAY_SECONDARY)}
			/>
		</DropdownMenuItem>
	);
}

/** The header every drilled-in level carries: back arrow, title, and whatever
 *  the level wants on the right (a wire mapping, a count). */
function LevelHeader({
	title,
	onBack,
	trailing,
}: {
	title: string;
	onBack: () => void;
	trailing?: React.ReactNode;
}) {
	return (
		<div className="flex items-center gap-1 px-2 py-1">
			<button
				type="button"
				aria-label="返回"
				onClick={(e) => {
					e.stopPropagation();
					onBack();
				}}
				className={cn(
					`flex size-5 flex-none items-center justify-center rounded-md
					transition-colors hover:text-neutral-800`,
					INK_OVERLAY_SECONDARY,
					SURFACE_OVERLAY_HOVER
				)}
			>
				<ChevronLeft className="size-4" />
			</button>
			<span className={cn('text-xs leading-5', OVERLAY_HEADING)}>{title}</span>
			{trailing && (
				<div className="ml-auto flex items-center gap-2">{trailing}</div>
			)}
		</div>
	);
}

type View = 'root' | 'models' | 'thinking';

/** Cross-fades between the picker's levels and animates the box height, so a
 *  drill-in grows out of the summary instead of hard-swapping from one size to
 *  the next. The measured child is a sibling of the animated box, not inside
 *  it, so its natural height is never confused with the height being tweened. */
function LevelSwitch({
	view,
	children,
	onReadyChange,
}: {
	view: View;
	children: React.ReactNode;
	onReadyChange: (ready: boolean) => void;
}) {
	const measureRef = useRef<HTMLDivElement>(null);
	const [height, setHeight] = useState<number | undefined>(undefined);
	// The first measurement lands with the popover's own opening animation: no
	// tween from zero.
	const settled = useRef(false);
	const targetHeight = useRef<number>();
	const moving = useRef(false);

	useLayoutEffect(() => {
		onReadyChange(false);
		const el = measureRef.current;
		if (!el) return;
		const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
		const apply = () => {
			// Clamped to the popover's own ceiling; the lists scroll inside it.
			const next = Math.min(el.offsetHeight, 320);
			if (
				settled.current &&
				!motion.matches &&
				targetHeight.current !== undefined &&
				Math.abs(next - targetHeight.current) > 0.5
			) {
				moving.current = true;
				onReadyChange(false);
			}
			targetHeight.current = next;
			setHeight(next);
			settled.current = true;
			if (!moving.current) onReadyChange(true);
		};
		const finishReduced = () => {
			if (motion.matches) {
				moving.current = false;
				onReadyChange(true);
			}
		};
		apply();
		const ro = new ResizeObserver(apply);
		ro.observe(el);
		motion.addEventListener('change', finishReduced);
		return () => {
			ro.disconnect();
			motion.removeEventListener('change', finishReduced);
		};
	}, [view, onReadyChange]);

	return (
		<div
			className={cn(
				'overflow-hidden',
				settled.current &&
					`transition-[height] duration-200 ease-out
					motion-reduce:transition-none`
			)}
			style={{ height }}
			onTransitionEnd={(event) => {
				if (
					event.target === event.currentTarget &&
					event.propertyName === 'height'
				) {
					moving.current = false;
					onReadyChange(true);
				}
			}}
		>
			<div ref={measureRef} key={view}>
				{children}
			</div>
		</div>
	);
}

export function ModelPicker({
	activeModel,
	activeProvider,
	models,
	onSelectModel,
	hasReasoning,
	activeThinkingLevel,
	thinkingMap,
	effectiveThinkingLevels,
	onSelectThinkingLevel,
	activeFastMode = false,
	fastModePending = false,
	onSelectFastMode,
	open: openProp,
	onOpenChange,
	density = 'full',
	configured = true,
	onConfigureModel,
}: {
	activeModel: string;
	activeProvider: string;
	models: agent.ModelItem[];
	onSelectModel: (provider: string, model: string) => Promise<void>;
	hasReasoning: boolean;
	activeThinkingLevel: string;
	thinkingMap?: Record<string, string | null>;
	effectiveThinkingLevels: readonly { value: string; token: string }[];
	onSelectThinkingLevel: (level: string) => Promise<void>;
	activeFastMode?: boolean;
	fastModePending?: boolean;
	onSelectFastMode?: (enabled: boolean) => Promise<void>;
	/** Controlled open state — `/model` opens the picker from the panel, which
	 *  the trigger alone cannot do. Uncontrolled when absent. */
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	/** The toolbar's width tier — caps the name, then drops the tier's word. */
	density?: ToolbarDensity;
	/** False when no provider is configured: there is no model to pick, so the
	 *  trigger asks for one instead of naming a placeholder. */
	configured?: boolean;
	/** Opens 设置 → 模型 — offered when `configured` is false. */
	onConfigureModel?: () => void;
}) {
	const [view, setView] = useState<View>('root');
	// True while the effort slider has the pointer: the popup must survive a drag
	// that ends past its edge, so every outside-dismiss is cancelled meanwhile.
	const effortDragging = useRef(false);
	const [readyView, setReadyView] = useState<View | null>(null);
	const handleLevelReady = useCallback(
		(ready: boolean) => setReadyView(ready ? view : null),
		[view]
	);
	const [selfOpen, setSelfOpen] = useState(false);
	const open = openProp ?? selfOpen;
	const setOpen = (next: boolean) => {
		setSelfOpen(next);
		// A fresh open always starts at the summary — a half-drilled popover from
		// a minute ago is not what the trigger's label promised.
		if (!next) setView('root');
		onOpenChange?.(next);
	};

	// The active model's config entry — supplies the display name. `undefined`
	// when the model isn't in the configured list (a hand-encoded `/model`
	// target, or a config edited outside the app): the raw id is then the only
	// thing we can show.
	const activeItem = models.find(
		(model) => model.model === activeModel && model.provider === activeProvider
	);
	const modelSections = useMemo(
		() =>
			groupOptions(
				models,
				(m) => m.provider || undefined,
				(a, b) => modelLabel(a).localeCompare(modelLabel(b))
			),
		[models]
	);
	/** The wire value the active tier rewrites to, when the model remaps it
	 *  (e.g. picking Low sends `max`) — shown so the control never lies. */
	const thinkingOverride =
		thinkingMap?.[activeThinkingLevel] &&
		thinkingMap[activeThinkingLevel] !== activeThinkingLevel
			? thinkingMap[activeThinkingLevel]
			: null;
	const tiers = effectiveThinkingLevels.map((level) => ({
		value: level.value,
		token: level.token,
		label: tierLabel(level.value),
	}));
	const thinkingAvailable = hasReasoning && tiers.length > 0;
	const fastAvailable = Boolean(
		activeItem?.supports_fast_mode && onSelectFastMode
	);
	const fastActive = fastAvailable && activeFastMode;
	const fastToggle = fastAvailable ? (
		<TooltipSimple
			content="更快速的吐字模式，可能会产生额外费用"
			onEscapeKeyDown={() => setOpen(false)}
		>
			<DropdownMenuItem
				density="compact"
				role="switch"
				aria-checked={activeFastMode}
				aria-label="Fast 模式"
				aria-busy={fastModePending}
				disabled={fastModePending}
				className="size-5 shrink-0 justify-center"
				onSelect={(event) => {
					event.preventDefault();
					void onSelectFastMode?.(!activeFastMode);
				}}
			>
				<Zap
					aria-hidden="true"
					className={cn(
						'size-4',
						activeFastMode ? FAST_INK : INK_OVERLAY_SECONDARY
					)}
				/>
			</DropdownMenuItem>
		</TooltipSimple>
	) : null;

	/** How much of the model name the trigger may print. Full keeps a cap so a
	 *  long id cannot stretch a roomy row; below full the name takes whatever the
	 *  row has left and truncates only when that runs out — a fixed cap there cut
	 *  it short while space sat empty beside it. */
	const nameCap = density === 'full' ? 'max-w-[180px]' : 'max-w-full';
	const activeTierToken =
		tiers.find((l) => l.value === activeThinkingLevel)?.token ??
		activeThinkingLevel;

	return (
		<DropdownMenu open={open} onOpenChange={setOpen}>
			<DropdownMenuTrigger asChild>
				<button
					aria-label={
						configured
							? `当前模型: ${activeItem ? modelLabel(activeItem) : activeModel || '未选择'} (${activeProvider}/${activeModel})${fastActive ? '，Fast 已开启' : ''}`
							: '当前模型: 未配置，点击设置模型'
					}
					className={cn(
						`flex min-w-0 cursor-pointer select-none items-center gap-1.5
						rounded-lg px-2.5 py-1 text-[12px] font-medium text-neutral-600
						transition-colors hover:bg-neutral-100`,
						// Below full the trigger is the one thing on the row that gives way:
						// it may shrink, and only its name truncates inside it.
						density !== 'full' && 'shrink'
					)}
				>
					<span
						className={cn(
							'min-w-0 truncate',
							nameCap,
							configured
								? // No display name → the label IS the id; keep the old mono
									// look instead of re-styling an identifier as prose.
									(!activeItem || modelLabel(activeItem) === activeModel) &&
										'font-mono text-[11.5px]'
								: 'text-neutral-500'
						)}
					>
						{configured
							? activeItem
								? modelLabel(activeItem)
								: activeModel || '选择模型'
							: '设置模型'}
					</span>
					{/* The thinking level rides with the model — one control picks both
              what answers and how hard it thinks. Its word stays for as long as
              the chip is there: the name truncates first, the chip gives way
              only when the row folds into tight and the whole chip goes. */}
					{thinkingAvailable && density !== 'tight' && (
						<span
							className="flex shrink-0 items-center gap-1 border-l
								border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)]
								pl-1.5"
						>
							<Brain
								className={cn(
									'h-3 w-3 shrink-0',
									activeThinkingLevel === 'off' ? 'text-neutral-400' : FAST_INK
								)}
							/>
							<span
								className="max-w-[5em] truncate text-[11px] text-neutral-500"
							>
								{activeTierToken}
							</span>
						</span>
					)}
					{fastActive && <FastIndicator className="size-3" />}
					<ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
				</button>
			</DropdownMenuTrigger>

			<DropdownMenuContent
				align="end"
				side="top"
				className="max-h-[min(20rem,var(--radix-dropdown-menu-content-available-height))]
					w-80 overflow-hidden"
				// A drag that starts on the effort slider is one gesture, wherever it
				// ends: dismissing on the release would make the last tiers unreachable.
				onPointerDownOutside={(e) => {
					if (effortDragging.current) e.preventDefault();
				}}
				onFocusOutside={(e) => {
					if (effortDragging.current) e.preventDefault();
				}}
				onInteractOutside={(e) => {
					if (effortDragging.current) e.preventDefault();
				}}
			>
				<LevelSwitch view={view} onReadyChange={handleLevelReady}>
					{!configured ? (
						// No provider in the config: there is no list to show, and a made-up
						// `default` row here is exactly the entry nothing can delete.
						<>
							<DropdownMenuLabel className={OVERLAY_HEADING}>
								模型与思考
							</DropdownMenuLabel>
							<DropdownMenuSeparator />
							<div
								className={cn(
									'flex flex-col gap-2 px-3 py-3 text-[12px]',
									INK_OVERLAY_SECONDARY
								)}
							>
								<span>尚未配置模型。请先在设置里添加提供商和模型。</span>
								<button
									type="button"
									onClick={() => {
										setOpen(false);
										onConfigureModel?.();
									}}
									className={cn(
										`self-start rounded-lg border border-neutral-200 px-2.5 py-1
											text-[12px] font-medium text-neutral-800
											transition-colors`,
										SURFACE_OVERLAY_HOVER
									)}
								>
									打开模型设置
								</button>
							</div>
						</>
					) : (
						view === 'root' && (
							<>
								<DropdownMenuLabel className={OVERLAY_HEADING}>
									模型与思考
								</DropdownMenuLabel>
								<DropdownMenuSeparator />
								<DropdownMenuGroup>
									<SummaryRow
										icon={<Bot className="size-3.5" />}
										label="模型"
										value={
											activeItem
												? modelLabel(activeItem)
												: activeModel || '未选择'
										}
										onOpen={() => setView('models')}
									/>
									<SummaryRow
										icon={<Brain className="size-3.5" />}
										label="思考"
										value={
											thinkingAvailable
												? tierLabel(activeThinkingLevel)
												: '该模型不支持思考等级'
										}
										status={
											fastActive ? (
												<FastIndicator className="size-3.5" />
											) : undefined
										}
										disabled={!thinkingAvailable && !fastAvailable}
										onOpen={() => setView('thinking')}
									/>
								</DropdownMenuGroup>
							</>
						)
					)}

					{view === 'models' && (
						<>
							<LevelHeader title="模型" onBack={() => setView('root')} />
							<DropdownMenuSeparator />
							{models.length > 0 ? (
								<DropdownMenuGroup
									className="max-h-[17rem] overflow-y-auto overflow-x-hidden"
								>
									{/* One section per provider, alphabetical — the provider map
                    upstream carries no order of its own. */}
									{modelSections.map((section, i) => (
										<div key={section.group ?? `solo-${i}`}>
											{i > 0 && <DropdownMenuSeparator />}
											{section.group && (
												<DropdownMenuLabel
													className={cn(
														'truncate text-[11px] font-medium',
														INK_OVERLAY_SECONDARY
													)}
												>
													{section.group}
												</DropdownMenuLabel>
											)}
											{section.items.map((item) => {
												const active =
													item.model === activeModel &&
													item.provider === activeProvider;
												return (
													<DropdownMenuItem
														key={`${item.provider}/${item.model}`}
														onClick={() => {
															void onSelectModel(item.provider, item.model);
															setView('root');
														}}
														className="flex min-w-0 cursor-pointer items-center
															justify-between overflow-hidden py-2 text-xs"
													>
														<div className="flex min-w-0 flex-1 flex-col pr-2">
															<TruncatedText
																text={modelLabel(item)}
																className="truncate font-medium
																	text-neutral-900"
															/>
															{modelSubLabel(item) && (
																<TruncatedText
																	text={modelSubLabel(item)}
																	className={cn(
																		'truncate font-mono text-[10px]',
																		INK_OVERLAY_SECONDARY
																	)}
																/>
															)}
														</div>
														{active && (
															<Check
																className="size-3.5 shrink-0 text-neutral-800"
															/>
														)}
													</DropdownMenuItem>
												);
											})}
										</div>
									))}
								</DropdownMenuGroup>
							) : (
								<div
									className={cn(
										'px-3 py-3 text-center text-xs',
										INK_OVERLAY_SECONDARY
									)}
								>
									未检测到模型
								</div>
							)}
						</>
					)}

					{view === 'thinking' && (
						<>
							<LevelHeader
								title="思考"
								onBack={() => setView('root')}
								trailing={
									<>
										{thinkingOverride && (
											<span
												className={cn(
													'max-w-28 truncate font-mono text-[10px]',
													INK_OVERLAY_SECONDARY
												)}
											>
												→ {thinkingOverride}
											</span>
										)}
										{fastToggle}
									</>
								}
							/>
							<DropdownMenuSeparator />
							<div className="px-2 pb-2 pt-1.5">
								{/* The stepped control: stops follow the active model's own
                  tiers, and one gesture reports the tier it landed on. */}
								{thinkingAvailable ? (
									<ReasoningEffort
										accent={fastActive ? 'purple' : 'blue'}
										effectsEnabled={readyView === 'thinking'}
										stops={tiers.map((tier) => tier.label)}
										levelTones={tiers.map((tier) => thinkingTone(tier.value))}
										value={Math.max(
											0,
											tiers.findIndex(
												(tier) => tier.value === activeThinkingLevel
											)
										)}
										label={activeItem ? modelLabel(activeItem) : activeModel}
										ariaLabel="思考等级"
										onDraggingChange={(dragging) => {
											effortDragging.current = dragging;
										}}
										onChange={(index) => {
											const tier = tiers[index];
											if (tier && tier.value !== activeThinkingLevel) {
												void onSelectThinkingLevel(tier.value);
											}
										}}
									/>
								) : (
									<p className={cn('py-1 text-xs', INK_OVERLAY_SECONDARY)}>
										此模型没有可调思考等级。
									</p>
								)}
							</div>
						</>
					)}
				</LevelSwitch>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
