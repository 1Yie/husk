/** Attached-file chips above the textarea — icon by kind, ✕ removes.
 * Content itself never enters the textarea; it's inlined into the
 * prompt at submit time. */

import { FileText, Image, File, X } from '@keyline-icons/react';
import type { Attachment } from '@/lib/agent-ipc/index';

export function AttachmentChips({
	items,
	onRemove,
}: {
	items: Attachment[];
	onRemove: (path: string) => void;
}) {
	if (items.length === 0) return null;
	return (
		<div className="flex flex-wrap gap-1.5 pb-1">
			{items.map((a) => (
				<span
					key={a.path}
					title={a.path}
					className="inline-flex max-w-[240px] items-center gap-1.5 rounded-md
						border border-[color-mix(in_srgb,var(--husk-n200)_60%,transparent)]
						bg-neutral-100 py-1 pl-1.5 pr-1 text-[11.5px] text-neutral-700"
				>
					<span className="shrink-0 text-neutral-500">
						{a.kind === 'image' && a.data_url ? (
							<img
								src={a.data_url}
								alt={a.name}
								className="h-4 w-4 rounded-sm object-cover"
							/>
						) : a.kind === 'image' ? (
							<Image className="h-3.5 w-3.5" />
						) : a.kind === 'text' ? (
							<FileText className="h-3.5 w-3.5" />
						) : (
							<File className="h-3.5 w-3.5" />
						)}
					</span>
					<span className="truncate">{a.name}</span>
					<button
						type="button"
						onClick={() => onRemove(a.path)}
						className="flex h-4 w-4 shrink-0 cursor-pointer items-center
							justify-center rounded-full text-neutral-500 hover:bg-neutral-200
							hover:text-neutral-700"
						aria-label={`移除 ${a.name}`}
					>
						<X className="h-2.5 w-2.5" />
					</button>
				</span>
			))}
		</div>
	);
}
