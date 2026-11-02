// Dropped deltas represent lost text; control events are never dropped.

import { useEffect, useRef, useState } from 'react';
import { Activity, TriangleAlert } from 'lucide-react';
import {
	CardBody,
	CardDivider,
	CardHeader,
	CardRow,
	StatPopover,
} from '@/components/title-bar/stat-popover';
import { getUiStats, onAgentEvent } from '@/lib/agent-ipc/index';
import { INK_OVERLAY_SECONDARY } from '@/lib/theme';
import type { UiStats } from '@/types';

/** Kernel counters, polled — they are cumulative atomics, so a slow poll
 * loses nothing. */
function useUiStats(intervalMs: number, enabled: boolean): UiStats | null {
	const [stats, setStats] = useState<UiStats | null>(null);
	useEffect(() => {
		if (!enabled) return;
		let dead = false;
		const tick = () => {
			void getUiStats()
				.then((s) => {
					if (!dead) setStats(s);
				})
				.catch(() => {});
		};
		tick();
		const timer = setInterval(tick, intervalMs);
		return () => {
			dead = true;
			clearInterval(timer);
		};
	}, [intervalMs, enabled]);
	return stats;
}

/** Events/s arriving from the kernel — how loud the IPC hop actually is.
 * Dev-only: it costs one extra listener on `agent://event`. */
function useIpcRate(enabled: boolean): number {
	const [rate, setRate] = useState(0);
	const countRef = useRef(0);
	useEffect(() => {
		if (!enabled || !import.meta.env.DEV) return;
		let dead = false;
		const un = onAgentEvent(() => {
			countRef.current += 1;
		});
		const timer = setInterval(() => {
			if (dead) return;
			setRate(countRef.current);
			countRef.current = 0;
		}, 1000);
		return () => {
			dead = true;
			clearInterval(timer);
			un.then((f) => f());
		};
	}, [enabled]);
	return rate;
}

const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);

export function StreamHealth({
	preview = false,
	generating = false,
	compact = false,
}: {
	preview?: boolean;
	generating?: boolean;
	compact?: boolean;
}) {
	const liveStats = useUiStats(3000, !preview);
	const ipc = useIpcRate(!preview);
	const stats = preview
		? { sent: 0, coalesced: 0, dropped: 0, depth_peak: 0 }
		: liveStats;
	const dropped = stats?.dropped ?? 0;
	const lost = dropped > 0;
	const state = lost
		? `文本丢失 ${fmt(dropped)}`
		: generating
			? '生成中'
			: '就绪';

	return (
		<StatPopover
			label={`生成状态 · ${state}`}
			trigger={
				<>
					{lost ? (
						<TriangleAlert
							className="h-[13px] w-[13px] flex-none text-amber-500"
						/>
					) : (
						<span
							aria-hidden
							data-generation-signal
							className="flex h-[13px] w-[13px] flex-none items-center
								justify-center gap-0.5"
						>
							{generating ? (
								[5, 11, 8, 4].map((height, index) => (
									<span
										key={index}
										className="w-0.5 rounded-full bg-accent
											motion-safe:animate-pulse"
										style={{ height, animationDelay: `${index * -250}ms` }}
									/>
								))
							) : (
								<span
									className="h-1.5 w-1.5 rounded-full bg-emerald-500 ring-[3px]
										ring-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]"
								/>
							)}
						</span>
					)}
					{!compact && <span>{state}</span>}
				</>
			}
		>
			<CardHeader
				icon={<Activity className="h-4 w-4" />}
				title="生成状态"
				value={state}
			/>
			<CardDivider />
			{stats ? (
				<CardBody>
					<CardRow label="队列新增" value={stats.sent} />
					<CardRow label="队列合并" value={stats.coalesced} />
					<CardRow label="峰值深度" value={stats.depth_peak} />
					<CardRow label="文本丢失" value={dropped} muted={!lost} />
					{import.meta.env.DEV && (
						<CardRow label="IPC 事件 / 秒" value={ipc} muted />
					)}
				</CardBody>
			) : (
				<p className={`px-3.5 py-3 ${INK_OVERLAY_SECONDARY}`}>
					队列统计暂不可用
				</p>
			)}
			{lost && (
				<p className="px-3.5 pb-3 text-neutral-800">
					文本增量已丢失，控制事件保持完整。界面消费速度落后于生成速度。
				</p>
			)}
		</StatPopover>
	);
}
