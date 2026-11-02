import { invoke, isTauri } from '@tauri-apps/api/core';
import { useEffect, type RefObject } from 'react';
import { framedShell } from '@/lib/platform';

interface Surface {
	x: number;
	y: number;
	width: number;
	height: number;
	viewportWidth: number;
	viewportHeight: number;
	background: number[];
	dark: boolean;
}

interface Update {
	revision: number;
	surface: Surface | null;
	settle: (active: boolean) => void;
}
interface Queue {
	busy: boolean;
	pending: Update | null;
	revision: number;
}

// One document owns one native surface, including across StrictMode and HMR.
const queue: Queue = import.meta.hot?.data?.sidebarGlassQueue ?? {
	busy: false,
	pending: null,
	revision: 0,
};
if (import.meta.hot) {
	import.meta.hot.dispose((data) => {
		data.sidebarGlassQueue = queue;
	});
}

async function drain() {
	if (queue.busy || !queue.pending) return;
	const next = queue.pending;
	queue.pending = null;
	queue.busy = true;
	try {
		const active = await invoke<boolean>('sidebar_glass_update', {
			revision: next.revision,
			surface: next.surface,
		});
		next.settle(active);
	} catch (error) {
		console.error('Sidebar glass native command failed', error);
		next.settle(false);
	} finally {
		queue.busy = false;
		void drain();
	}
}

function enqueue(surface: Surface | null, settle: Update['settle']) {
	queue.revision = Math.max(
		performance.timeOrigin + performance.now(),
		queue.revision + 0.01
	);
	queue.pending = { revision: queue.revision, surface, settle };
	void drain();
	return queue.revision;
}

export function useSidebarGlass(ref: RefObject<HTMLDivElement>) {
	useEffect(() => {
		const shell = ref.current;
		if (!shell || !framedShell || !isTauri()) return;
		const root = document.documentElement;
		let dead = false;
		let frame = 0;
		let wanted = 0;
		let suspended = false;
		let lastKey = '';
		let colour = '';
		let rgb = [1, 1, 1];
		const canvas = document.createElement('canvas');
		canvas.width = canvas.height = 1;
		const context = canvas.getContext('2d', { willReadFrequently: true });

		const measure = () => {
			frame = 0;
			if (dead || (suspended && !document.hidden)) return;
			let surface: Surface | null = null;
			if (!document.hidden) {
				const rect = shell.getBoundingClientRect();
				if (!context) return;
				const background = getComputedStyle(root)
					.getPropertyValue('--husk-panel')
					.trim();
				if (background !== colour && context) {
					context.clearRect(0, 0, 1, 1);
					context.fillStyle = background;
					context.fillRect(0, 0, 1, 1);
					rgb = Array.from(context.getImageData(0, 0, 1, 1).data)
						.slice(0, 3)
						.map((v) => v / 255);
					colour = background;
				}
				surface = {
					x: rect.x,
					y: rect.y,
					width: rect.width,
					height: rect.height,
					viewportWidth: innerWidth,
					viewportHeight: innerHeight,
					background: rgb,
					dark: root.classList.contains('dark'),
				};
			}
			const key = JSON.stringify(surface);
			if (key === lastKey) return;
			lastKey = key;
			const sent = enqueue(surface, (active) => {
				if (dead || wanted !== sent) return;
				root.toggleAttribute('data-sidebar-glass', active);
				suspended = !active && surface !== null;
			});
			wanted = sent;
		};
		const schedule = () => {
			if (!dead && !frame) frame = requestAnimationFrame(measure);
		};
		const nativeChanged = () => {
			suspended = false;
			lastKey = '';
			schedule();
		};
		const resize = new ResizeObserver(schedule);
		resize.observe(shell);
		const theme = new MutationObserver(schedule);
		theme.observe(root, {
			attributes: true,
			attributeFilter: ['class', 'style'],
		});
		window.addEventListener('resize', schedule);
		window.addEventListener('husk-sidebar-glass-change', nativeChanged);
		window.visualViewport?.addEventListener('resize', schedule);
		document.addEventListener('visibilitychange', nativeChanged);
		schedule();
		return () => {
			dead = true;
			cancelAnimationFrame(frame);
			resize.disconnect();
			theme.disconnect();
			window.removeEventListener('resize', schedule);
			window.removeEventListener('husk-sidebar-glass-change', nativeChanged);
			window.visualViewport?.removeEventListener('resize', schedule);
			document.removeEventListener('visibilitychange', nativeChanged);
			root.removeAttribute('data-sidebar-glass');
			enqueue(null, () => {});
		};
	}, [ref]);
}
