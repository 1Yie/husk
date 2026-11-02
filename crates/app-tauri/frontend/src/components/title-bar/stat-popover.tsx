// Title-bar stat chip with a detail card. Hover previews the card, a click
// pins it open (outside click / Esc closes). Touch and pen skip the hover
// path, so a tap is just the pinned open.

import * as React from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { cn } from '@/lib/utils';
import {
	INK_OVERLAY_SECONDARY,
	OVERLAY_DIVIDER,
	OVERLAY_HEADING,
	SURFACE_OVERLAY,
	SURFACE_OVERLAY_HOVER,
	SURFACE_OVERLAY_OPEN,
} from '@/lib/theme';
import {
	POPPER_PENDING,
	usePopperPlacedRef,
} from '@/components/ui/use-popper-placed';

const OPEN_DELAY = 120;
const CLOSE_DELAY = 160;

const StatPopoverContext = React.createContext<{
	active: string | null;
	setActive: React.Dispatch<React.SetStateAction<string | null>>;
} | null>(null);

export const StatPopoverGroup = StatPopoverContext.Provider;

export function StatPopover({
	label,
	trigger,
	children,
	className,
}: {
	/** Accessible name of the chip. */
	label: string;
	trigger: React.ReactNode;
	children: React.ReactNode;
	className?: string;
}) {
	const id = React.useId();
	const group = React.useContext(StatPopoverContext);
	if (!group) throw new Error('StatPopover requires a StatPopoverGroup');
	const open = group.active === id;
	const setOpen = (next: boolean) => {
		group.setActive((active) => (next ? id : active === id ? null : active));
	};
	const [pinned, setPinned] = React.useState(false);
	const triggerRef = React.useRef<HTMLButtonElement>(null);
	const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
	// Only Escape returns focus to the chip; hover and outside dismissal keep
	// focus where the reader put it. WebKit pointer clicks leave buttons unfocused.
	const escClosed = React.useRef(false);
	const { placed, setRef } = usePopperPlacedRef<HTMLDivElement>(null);

	const clear = () => {
		if (timer.current) clearTimeout(timer.current);
		timer.current = null;
	};
	React.useEffect(() => clear, []);

	const hoverOpen = (e: React.PointerEvent) => {
		if (e.pointerType !== 'mouse') return;
		clear();
		if (open) return;
		timer.current = setTimeout(() => {
			setPinned(false);
			setOpen(true);
		}, OPEN_DELAY);
	};
	const hoverClose = (e: React.PointerEvent) => {
		if (e.pointerType !== 'mouse') return;
		clear();
		if (pinned && open) return;
		timer.current = setTimeout(() => setOpen(false), CLOSE_DELAY);
	};
	const keepOpen = () => clear();

	return (
		<PopoverPrimitive.Root
			open={open}
			onOpenChange={(next) => {
				clear();
				setOpen(next);
				setPinned(next);
			}}
		>
			<PopoverPrimitive.Trigger asChild>
				<button
					ref={triggerRef}
					type="button"
					data-tauri-drag-region="false"
					aria-label={label}
					data-pinned={(open && pinned) || undefined}
					onPointerEnter={hoverOpen}
					onPointerLeave={hoverClose}
					onClick={(e) => {
						// Own the toggle: a click on a hover-opened card pins it instead
						// of closing it.
						e.preventDefault();
						clear();
						if (open && !pinned) {
							setPinned(true);
						} else {
							setOpen(!open);
							setPinned(!open);
						}
					}}
					className={cn(
						`flex h-6 flex-none cursor-default items-center gap-1.5 rounded-md
						px-1.5 text-[11px] tabular-nums leading-none transition-colors`,
						INK_OVERLAY_SECONDARY,
						`outline-none focus-visible:ring-2
						focus-visible:ring-[color-mix(in_srgb,var(--husk-accent)_45%,transparent)]`,
						SURFACE_OVERLAY_HOVER,
						SURFACE_OVERLAY_OPEN,
						'hover:text-neutral-800 data-[state=open]:text-neutral-800'
					)}
				>
					{trigger}
				</button>
			</PopoverPrimitive.Trigger>
			<PopoverPrimitive.Portal>
				<PopoverPrimitive.Content
					ref={setRef}
					side="bottom"
					align="end"
					sideOffset={6}
					collisionPadding={12}
					onOpenAutoFocus={(e) => {
						if (!pinned) e.preventDefault();
					}}
					onEscapeKeyDown={() => {
						escClosed.current = true;
					}}
					onCloseAutoFocus={(e) => {
						e.preventDefault();
						if (escClosed.current)
							triggerRef.current?.focus({ preventScroll: true });
						escClosed.current = false;
					}}
					onPointerEnter={keepOpen}
					onPointerLeave={hoverClose}
					className={cn(
						`z-50 w-72 select-none p-0 text-[12px] text-neutral-700 shadow-popup
						outline-none`,
						`data-[state=open]:animate-in data-[state=open]:fade-in-0
						data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-1`,
						SURFACE_OVERLAY,
						!placed && POPPER_PENDING,
						className
					)}
				>
					{children}
				</PopoverPrimitive.Content>
			</PopoverPrimitive.Portal>
		</PopoverPrimitive.Root>
	);
}

/** Card header: icon + title on the left, the headline figure on the right. */
export function CardHeader({
	icon,
	title,
	value,
}: {
	icon?: React.ReactNode;
	title: React.ReactNode;
	value?: React.ReactNode;
}) {
	return (
		<div className="flex items-center gap-2 px-3.5 pb-2.5 pt-3">
			{icon && (
				<span className={cn('flex-none', INK_OVERLAY_SECONDARY)}>{icon}</span>
			)}
			<span className={OVERLAY_HEADING}>{title}</span>
			{value != null && (
				<span className="ml-auto tabular-nums text-neutral-800">{value}</span>
			)}
		</div>
	);
}

export function CardDivider() {
	return <div className={cn('mx-3.5 h-px', OVERLAY_DIVIDER)} />;
}

/** A label/value row; `swatch` prefixes a legend colour square. */
export function CardRow({
	label,
	value,
	swatch,
	muted,
}: {
	label: React.ReactNode;
	value: React.ReactNode;
	swatch?: string;
	muted?: boolean;
}) {
	return (
		<div className="flex items-center gap-2 px-3.5 py-1">
			{swatch && (
				<span className={cn('h-2 w-2 flex-none rounded-[3px]', swatch)} />
			)}
			<span className={INK_OVERLAY_SECONDARY}>{label}</span>
			<span
				className={cn(
					'ml-auto tabular-nums',
					muted ? INK_OVERLAY_SECONDARY : 'text-neutral-800'
				)}
			>
				{value}
			</span>
		</div>
	);
}

export function CardBody({ children }: { children: React.ReactNode }) {
	return <div className="py-2">{children}</div>;
}
