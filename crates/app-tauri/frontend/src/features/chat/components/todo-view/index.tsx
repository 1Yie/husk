// TodoView — Clean, structured UI presentation for agent task/todo tracking.
// Parses `todo` tool output and renders an interactive-looking, progress-tracked checklist.

import { useMemo } from 'react';
import { Check, ListCheck } from '@keyline-icons/react';
import { cn } from '@/lib/utils';

interface Props {
	content: string;
}

export type TodoStatus = 'pending' | 'in_progress' | 'completed';

export interface TodoItem {
	id: number;
	/** The label the tool printed: `4` for a task, `4-1` for its first subtask. */
	label: string;
	text: string;
	status: TodoStatus;
	/** Nesting depth the tool rendered: 0 for a task, 1 for its subtask. */
	depth: number;
	/** How the agent says the task is being carried out, when it wrote one. */
	note?: string;
}

/** One task line: a mark, an optional `#label` (`4`, or `4-1` for a subtask),
 *  the text, and an optional ` — note` progress line the tool appends. */
const TASK_LINE = /^\[([ xX~])\]\s*#?([\d]+(?:-[\d]+)?)\s+(.+)$/;

/** Split the text from the ` — note` tail the tool appends. A dash inside the
 *  task's own words stays in the text: the note is the LAST such separator. */
function splitNote(raw: string): { text: string; note?: string } {
	const at = raw.lastIndexOf(' — ');
	if (at <= 0) return { text: raw };
	const text = raw.slice(0, at).trim();
	const note = raw.slice(at + 3).trim();
	return note ? { text, note } : { text: raw };
}

function statusOf(mark: string): TodoStatus {
	if (mark === '~') return 'in_progress';
	return mark.toLowerCase() === 'x' ? 'completed' : 'pending';
}

export function parseTodos(raw: string): {
	items: TodoItem[];
	doneCount: number;
	totalCount: number;
	headerMessage?: string;
} {
	const items: TodoItem[] = [];
	const lines = raw.split('\n');
	let headerMessage: string | undefined;

	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed) continue;

		// `[~] #id text` is in progress; two spaces of indent make it a subtask.
		const taskMatch = trimmed.match(TASK_LINE);
		if (taskMatch) {
			const leading = line.length - line.trimStart().length;
			const status = statusOf(taskMatch[1]);
			// The tool only writes a note while a task is in progress, so a dash
			// inside any other row's own words stays part of its text.
			const { text, note } =
				status === 'in_progress'
					? splitNote(taskMatch[3].trim())
					: { text: taskMatch[3].trim(), note: undefined };
			items.push({
				id: parseInt(taskMatch[2].split('-')[0], 10),
				label: taskMatch[2],
				text,
				status,
				depth: Math.min(1, Math.floor(leading / 2)),
				note,
			});
			continue;
		}

		// Capture leading messages like "Added #1 ...", "Removed #2 ...", "Cleared ... todos"
		if (
			trimmed.startsWith('Added #') ||
			trimmed.startsWith('Removed #') ||
			trimmed.startsWith('Cleared ') ||
			trimmed.endsWith('done')
		) {
			if (!trimmed.endsWith('done')) {
				headerMessage = trimmed;
			}
		}
	}

	const doneCount = items.filter((t) => t.status === 'completed').length;
	const totalCount = items.length;

	return { items, doneCount, totalCount, headerMessage };
}

export function TodoView({ content }: Props) {
	const { items, doneCount, totalCount, headerMessage } = useMemo(
		() => parseTodos(content),
		[content]
	);

	if (items.length === 0) {
		return (
			<div
				className="w-full rounded-xl border border-neutral-200 bg-neutral-50 p-3
					font-mono text-xs text-neutral-500"
			>
				{content || '暂无任务项'}
			</div>
		);
	}

	const percentage =
		totalCount > 0 ? Math.round((doneCount / totalCount) * 100) : 0;

	return (
		<div
			className="shadow-2xs w-full select-text rounded-xl border
				border-neutral-200 bg-neutral-50 p-3"
		>
			<div
				className="mb-2 flex select-none items-center justify-between gap-3
					border-b border-[color-mix(in_srgb,var(--husk-n200)_70%,transparent)]
					pb-2.5"
			>
				<div className="flex min-w-0 items-center gap-2">
					<ListCheck
						className="h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400"
					/>
					<span className="shrink-0 text-xs font-semibold text-neutral-800">
						任务清单
					</span>
					{headerMessage && (
						<span className="truncate text-[11px] font-normal text-neutral-500">
							· {headerMessage}
						</span>
					)}
				</div>
				<div className="flex shrink-0 items-center gap-2.5">
					<div
						className="h-1.5 w-24 overflow-hidden rounded-full bg-neutral-200"
					>
						<div
							className="h-full rounded-full bg-emerald-500 transition-all
								duration-300"
							style={{ width: `${percentage}%` }}
						/>
					</div>
					<span className="font-mono text-[11px] font-medium text-neutral-500">
						{doneCount}/{totalCount} ({percentage}%)
					</span>
				</div>
			</div>

			<div className="flex flex-col gap-1">
				{items.map((item) => (
					<div
						key={item.label}
						className={cn(
							`group flex items-start gap-2.5 rounded-lg px-1.5 py-1
							transition-colors`,
							item.depth > 0 && 'ml-4',
							item.status === 'completed'
								? 'text-neutral-500'
								: `text-neutral-800
									hover:bg-[color-mix(in_srgb,var(--husk-n100)_60%,transparent)]`
						)}
					>
						<span className="flex h-5 w-4 shrink-0 items-center justify-center">
							{item.status === 'completed' ? (
								<span
									className="inline-flex h-3.5 w-3.5 items-center justify-center
										rounded-full bg-emerald-500/20 text-emerald-600
										dark:text-emerald-400"
								>
									<Check className="h-2.5 w-2.5 stroke-[2.8]" />
								</span>
							) : item.status === 'in_progress' ? (
								<span
									className="inline-flex h-3.5 w-3.5 items-center justify-center
										rounded-full border border-amber-500/70 bg-amber-500/10"
								>
									<span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
								</span>
							) : (
								<span
									className="inline-flex h-3.5 w-3.5 items-center justify-center
										rounded-full border border-neutral-300 transition-colors
										group-hover:border-neutral-400"
								/>
							)}
						</span>
						<span
							className="flex h-5 shrink-0 select-none items-center font-mono
								text-[11px] text-neutral-500"
						>
							#{item.label}
						</span>
						<span className="flex min-w-0 flex-1 flex-col">
							<span
								className={cn(
									'break-words text-[13px] leading-5',
									item.status === 'completed'
										? 'line-through opacity-75'
										: item.status === 'in_progress'
											? 'font-medium'
											: 'font-normal'
								)}
							>
								{item.text}
							</span>
							{item.note && (
								<span
									className="break-words text-[11.5px] leading-5
										text-neutral-500"
								>
									{item.note}
								</span>
							)}
						</span>
					</div>
				))}
			</div>
		</div>
	);
}
