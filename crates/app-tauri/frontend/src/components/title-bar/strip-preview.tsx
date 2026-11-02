// The status strip's preview editor: the bar itself, editable. Chips are drawn
// as they really are (values included) but inert — a press here is a drag, not a
// hover card — and the modules that are switched off wait in a tray below.
import {
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
	type PointerEvent as ReactPointerEvent,
	type ReactNode,
} from 'react';
import { RotateCcw, X } from '@keyline-icons/react';
import { cn } from '@/lib/utils';
import { STATUS_MODULES, moveStatusItem } from './strip';

type DropTarget = { zone: 'shown' | 'hidden'; id: string | null };
type DragGesture = {
	id: string;
	pointer: number;
	x: number;
	y: number;
	offsetX: number;
	offsetY: number;
	scale: number;
	source: HTMLElement;
	ghost: HTMLElement | null;
	target: DropTarget | null;
	active: boolean;
};

export function StripPreview({
	order,
	hidden,
	nodes,
	onCommit,
	onToggle,
	onReset,
	renderBar,
}: {
	/** Every known module, in display order (hidden ones last). */
	order: readonly string[];
	hidden: Set<string>;
	/** The title bar supplies the same chips it renders in the workspace. */
	nodes: Record<string, ReactNode>;
	/** The finished order after a drag. One write per gesture. */
	onCommit: (order: string[], shownId?: string) => void;
	onToggle: (id: string, visible: boolean) => void;
	onReset: () => void;
	renderBar: (row: ReactNode) => ReactNode;
}) {
	// Drop targets stay stationary until the pointer is released.
	const [draft, setDraft] = useState<string[]>([...order]);
	const [dragging, setDragging] = useState<string | null>(null);
	const [dropTarget, setDropTarget] = useState<string | null>(null);
	const desktop = useRef<HTMLDivElement>(null);
	const editor = useRef<HTMLDivElement>(null);
	const gesture = useRef<DragGesture | null>(null);
	const suppressClick = useRef(false);
	const externalLayout = useRef({ order: [...order], hidden: new Set(hidden) });

	function cancelDrag(repaint = true) {
		const current = gesture.current;
		if (!current) return;
		gesture.current = null;
		if (current.active) suppressClick.current = true;
		current.ghost?.remove();
		if (editor.current?.hasPointerCapture(current.pointer)) {
			editor.current.releasePointerCapture(current.pointer);
		}
		if (repaint) {
			setDragging(null);
			setDropTarget(null);
		}
	}

	useLayoutEffect(() => {
		const el = desktop.current;
		if (!el) return;
		let frame = 0;
		const measure = () => {
			const scale = String(Math.max(0.8, Math.min(1, el.clientWidth / 840)));
			if (el.style.getPropertyValue('--status-preview-scale') === scale) return;
			el.style.setProperty('--status-preview-scale', scale);
		};
		measure();
		const observer = new ResizeObserver(() => {
			cancelAnimationFrame(frame);
			frame = requestAnimationFrame(measure);
		});
		observer.observe(el);
		return () => {
			observer.disconnect();
			cancelAnimationFrame(frame);
		};
	}, []);

	useEffect(() => {
		const previous = externalLayout.current;
		if (
			order.length === previous.order.length &&
			order.every((id, i) => id === previous.order[i]) &&
			hidden.size === previous.hidden.size &&
			[...hidden].every((id) => previous.hidden.has(id))
		)
			return;
		externalLayout.current = { order: [...order], hidden: new Set(hidden) };
		cancelDrag();
		setDraft([...order]);
	}, [order, hidden]);

	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key !== 'Escape') return;
			if (gesture.current) {
				e.preventDefault();
				e.stopPropagation();
				cancelDrag();
				return;
			}
		};
		const onBlur = () => cancelDrag();
		const onVisibility = () => {
			if (document.hidden) cancelDrag();
		};
		window.addEventListener('keydown', onKey, true);
		window.addEventListener('blur', onBlur);
		document.addEventListener('visibilitychange', onVisibility);
		return () => {
			window.removeEventListener('keydown', onKey, true);
			window.removeEventListener('blur', onBlur);
			document.removeEventListener('visibilitychange', onVisibility);
			cancelDrag(false);
		};
	}, []);

	const shown = draft.filter((id) => !hidden.has(id));
	const tray = draft.filter((id) => hidden.has(id));
	const label = (id: string) =>
		STATUS_MODULES.find((m) => m.id === id)?.label ?? id;

	/** Drop `id` at `index`, keeping the rest in order. */
	const drop = (id: string, index: number) => {
		const next = moveStatusItem(draft, id, index);
		setDraft(next);
		setDragging(null);
		setDropTarget(null);
		if (hidden.has(id)) onCommit(next, id);
		else onCommit(next);
	};

	function targetAt(x: number, y: number): DropTarget | null {
		const hit = document.elementFromPoint(x, y);
		if (!hit || !editor.current?.contains(hit)) return null;
		const zone = hit?.closest<HTMLElement>('[data-strip-zone]');
		if (!zone)
			return desktop.current?.contains(hit)
				? { zone: 'hidden', id: null }
				: null;
		return {
			zone: zone.dataset.stripZone === 'hidden' ? 'hidden' : 'shown',
			id:
				hit?.closest<HTMLElement>('[data-strip-item]')?.dataset.stripItem ??
				null,
		};
	}

	function startDrag(event: ReactPointerEvent<HTMLElement>, id: string) {
		if (event.button !== 0 || event.isPrimary === false || gesture.current)
			return;
		const button = (event.target as Element).closest('button');
		if (button && button !== event.currentTarget) return;
		event.preventDefault();
		const source = event.currentTarget;
		const rect = source.getBoundingClientRect();
		source.focus({ preventScroll: true });
		editor.current!.setPointerCapture(event.pointerId);
		gesture.current = {
			id,
			pointer: event.pointerId,
			x: event.clientX,
			y: event.clientY,
			offsetX: event.clientX - rect.left,
			offsetY: event.clientY - rect.top,
			scale: rect.height / Math.max(1, source.offsetHeight),
			source,
			ghost: null,
			target: null,
			active: false,
		};
	}

	function moveDrag(event: ReactPointerEvent) {
		const current = gesture.current;
		if (!current || current.pointer !== event.pointerId) return;
		if (!current.active) {
			if (Math.hypot(event.clientX - current.x, event.clientY - current.y) < 5)
				return;
			current.active = true;
			const ghost = current.source.cloneNode(true) as HTMLElement;
			ghost.setAttribute('data-strip-drag-ghost', '');
			ghost.setAttribute('aria-hidden', 'true');
			ghost.setAttribute('inert', '');
			ghost.removeAttribute('id');
			ghost.querySelectorAll('[id]').forEach((el) => el.removeAttribute('id'));
			ghost.classList.add('bg-panel', 'shadow-popup');
			Object.assign(ghost.style, {
				position: 'fixed',
				top: '0',
				left: '0',
				zIndex: '60',
				width: `${current.source.offsetWidth}px`,
				height: `${current.source.offsetHeight}px`,
				pointerEvents: 'none',
				transformOrigin: 'top left',
				transition: 'none',
			});
			document.body.appendChild(ghost);
			current.ghost = ghost;
			setDragging(current.id);
		}
		current.ghost!.style.transform = `translate3d(${event.clientX - current.offsetX}px, ${event.clientY - current.offsetY}px, 0) scale(${current.scale})`;
		current.target = targetAt(event.clientX, event.clientY);
		setDropTarget(
			current.target
				? current.target.zone === 'hidden'
					? 'tray'
					: (current.target.id ?? 'bar')
				: null
		);
	}

	function finishDrag(event: ReactPointerEvent) {
		const current = gesture.current;
		if (!current || current.pointer !== event.pointerId) return;
		const target = targetAt(event.clientX, event.clientY);
		suppressClick.current = current.active;
		cancelDrag();
		if (!target) return;
		if (!current.active) {
			if (
				hidden.has(current.id) &&
				target.zone === 'hidden' &&
				target.id === current.id
			) {
				onToggle(current.id, true);
			}
			return;
		}
		if (target.zone === 'hidden') {
			if (!hidden.has(current.id)) onToggle(current.id, false);
		} else if (target.id !== current.id) {
			drop(current.id, target.id ? draft.indexOf(target.id) : draft.length);
		}
	}

	const renderChip = (id: string) => (
		<span
			{...{ inert: '' }}
			aria-hidden
			className="pointer-events-none contents"
		>
			{nodes[id]}
		</span>
	);

	const row = (
		<div className="flex min-h-7 items-center gap-0.5">
			{shown.length === 0 && (
				<span className="px-1 text-[11px] text-neutral-400">
					把下面的模块拖进来
				</span>
			)}
			{shown.map((id, index) => (
				<span
					key={id}
					data-strip-item={id}
					data-tauri-drag-region="false"
					role="group"
					aria-label={label(id)}
					aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight Delete"
					title={`拖动调整「${label(id)}」的位置`}
					onPointerDown={(event) => startDrag(event, id)}
					onKeyDown={(e) => {
						if (e.key === 'Delete' || e.key === 'Backspace') {
							e.preventDefault();
							onToggle(id, false);
							return;
						}
						if (!e.altKey) return;
						const step =
							e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0;
						if (step === 0) return;
						e.preventDefault();
						const target = shown[index + step];
						if (target) drop(id, draft.indexOf(target));
					}}
					tabIndex={0}
					className={cn(
						`group/stripchip relative flex flex-none cursor-grab touch-none
						items-center rounded-md outline-none active:cursor-grabbing`,
						`hover:bg-[color-mix(in_srgb,var(--husk-n200)_50%,transparent)]
						focus-visible:ring-2
						focus-visible:ring-[color-mix(in_srgb,var(--husk-accent)_45%,transparent)]`,
						dragging === id && 'opacity-40',
						dragging &&
							dropTarget === id &&
							dragging !== id && [
								`after:pointer-events-none after:absolute after:inset-y-0
								after:w-0.5 after:rounded-full after:bg-accent`,
								draft.indexOf(dragging) < draft.indexOf(id)
									? 'after:-right-0.5'
									: 'after:-left-0.5',
							]
					)}
				>
					{renderChip(id)}
					<button
						type="button"
						aria-label={`隐藏${label(id)}`}
						onClick={() => onToggle(id, false)}
						className={cn(
							`rounded p-0.5 text-neutral-500 opacity-0 transition-opacity
							hover:text-neutral-800 focus-visible:opacity-100
							group-focus-within/stripchip:opacity-100
							group-hover/stripchip:opacity-100`,
							'absolute -right-1 -top-1.5 z-10 bg-panel'
						)}
					>
						<X className="h-2.5 w-2.5" />
					</button>
				</span>
			))}
		</div>
	);

	return (
		<div
			ref={editor}
			data-strip-editor
			data-tauri-drag-region="false"
			data-nodrag="true"
			role="region"
			aria-label="自定义状态栏"
			onPointerDownCapture={() => {
				suppressClick.current = false;
			}}
			onPointerMove={moveDrag}
			onPointerUp={finishDrag}
			onPointerCancel={() => cancelDrag()}
			onLostPointerCapture={() => cancelDrag()}
			onClickCapture={(event) => {
				if (!suppressClick.current || event.detail === 0) return;
				event.preventDefault();
				event.stopPropagation();
				suppressClick.current = false;
			}}
			className="flex w-full min-w-0 flex-col gap-2"
		>
			<div className="flex items-center justify-between gap-3 px-0.5">
				<span className="text-[11px] text-neutral-500">
					直接拖动预览中的项目排序，移出状态栏即可隐藏
				</span>
				<div className="flex flex-none items-center gap-1">
					<button
						type="button"
						onClick={onReset}
						className="flex items-center gap-1 rounded-md px-1.5 py-1
							text-[11px] text-neutral-500
							hover:bg-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
							hover:text-neutral-800"
					>
						<RotateCcw className="h-3 w-3" />
						恢复默认
					</button>
				</div>
			</div>
			<div
				ref={desktop}
				data-status-preview-desktop
				className="overflow-x-auto rounded-xl border border-hairline
					bg-gradient-to-br from-sky-200 via-indigo-200 to-rose-200
					dark:from-slate-800 dark:via-indigo-950 dark:to-slate-900"
			>
				<div
					className="min-w-[840px] px-6 pt-8"
					style={{ zoom: 'var(--status-preview-scale, 1)' }}
				>
					<div
						data-status-preview-window
						className="overflow-hidden rounded-t-2xl border border-b-0
							border-hairline
							bg-[color-mix(in_srgb,var(--husk-panel)_80%,transparent)]
							shadow-popup backdrop-blur-md"
					>
						<div
							data-strip-zone="shown"
							className={cn(
								'relative shrink-0 rounded-t-2xl',
								dragging &&
									dropTarget &&
									dropTarget !== 'tray' &&
									`ring-2 ring-inset
									ring-[color-mix(in_srgb,var(--husk-accent)_45%,transparent)]`
							)}
						>
							{renderBar(row)}
						</div>
						<div aria-hidden className="flex h-44 gap-2 px-2">
							<div
								className="flex min-w-0 flex-1 flex-col gap-3 rounded-t-2xl
									bg-workspace p-5"
							>
								<div className="h-2 w-28 rounded-full bg-neutral-200" />
								<div className="mt-1 h-2 w-2/3 rounded-full bg-neutral-100" />
								<div className="h-2 w-1/2 rounded-full bg-neutral-100" />
							</div>
							<div className="w-[28%] rounded-t-2xl bg-workspace">
								<div
									className="flex h-10 items-center gap-2 border-b
										border-hairline px-4 text-xs text-neutral-600"
								>
									<span className="h-2 w-14 rounded-full bg-neutral-200" />
								</div>
								<div className="space-y-3 p-4">
									<div className="h-2 w-3/4 rounded-full bg-neutral-100" />
									<div className="h-2 w-1/2 rounded-full bg-neutral-100" />
								</div>
							</div>
						</div>
					</div>
				</div>
			</div>
			<div
				className="flex flex-wrap items-center justify-between gap-1 px-0.5
					text-[11px] text-neutral-500"
			>
				<span>未显示</span>
				{tray.length > 0 && <span>拖回上方状态栏即可显示</span>}
			</div>
			<div
				data-strip-zone="hidden"
				className={cn(
					`flex min-h-12 flex-wrap items-center gap-2 rounded-xl border
					border-dashed border-hairline p-2`,
					dragging &&
						!hidden.has(dragging) &&
						dropTarget === 'tray' &&
						`border-accent ring-2
						ring-[color-mix(in_srgb,var(--husk-accent)_30%,transparent)]`
				)}
			>
				{tray.length === 0 && (
					<span className="px-1 text-[11px] text-neutral-500">
						全部项目已显示，拖到这里可隐藏
					</span>
				)}
				{tray.map((id) => (
					<span
						key={id}
						data-strip-item={id}
						data-tauri-drag-region="false"
						role="button"
						tabIndex={0}
						aria-label={label(id)}
						onPointerDown={(event) => startDrag(event, id)}
						onClick={(event) => {
							if (event.detail === 0) onToggle(id, true);
						}}
						onKeyDown={(event) => {
							if (event.key !== 'Enter' && event.key !== ' ') return;
							event.preventDefault();
							onToggle(id, true);
						}}
						title={`点击显示「${label(id)}」`}
						className={cn(
							`flex flex-none cursor-grab touch-none items-center rounded-lg
							border border-hairline bg-white px-2 py-1 outline-none
							hover:bg-hover focus-visible:ring-2 focus-visible:ring-accent
							active:cursor-grabbing`,
							dragging === id && 'opacity-40'
						)}
					>
						{renderChip(id)}
					</span>
				))}
			</div>
		</div>
	);
}
