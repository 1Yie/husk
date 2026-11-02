import { useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getAppearance } from '@/lib/agent-ipc/sessions';
import { APPEARANCE_CHANGED } from '@/lib/appearance';
import { resolveStripOrder, type StatusStripLayout } from './strip';

/** Read the workspace title bar's saved layout. */
export function useStatusStrip(enabled = true) {
	const [layout, setLayout] = useState<StatusStripLayout | null>(null);

	useEffect(() => {
		if (!enabled) return;
		let alive = true;
		let revision = 0;
		const read = () => {
			const requested = ++revision;
			void getAppearance()
				.then((cfg) => {
					if (alive && requested === revision)
						setLayout(cfg?.status_strip ?? null);
				})
				.catch(() => {
					/* A failed read leaves the last layout in place. */
				});
		};
		const unlisten = listen(APPEARANCE_CHANGED, read).catch(() => null);
		read();
		return () => {
			alive = false;
			void unlisten.then((stop) => stop?.());
		};
	}, [enabled]);

	const order = useMemo(() => resolveStripOrder(layout), [layout]);

	return { order };
}
