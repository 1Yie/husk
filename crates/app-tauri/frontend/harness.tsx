import React from 'react';
import { createRoot } from 'react-dom/client';
import './src/index.css';
import { MoreHorizontal, GitFork, Bin } from '@keyline-icons/react';
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { SettingSelect } from '@/components/settings';

/* Scratch visual-check page (untracked, next to harness.html):
   `?state=hover` paints the last menu item's hover/`:focus` background without
   needing a focused window, and renders a page scrollbar behind the popup so
   the popup's shadow token can be judged against real background chrome.
   Usage: `vite --config vite.harness.config.ts`, then open /harness.html. */

const q = new URLSearchParams(location.search);
const hover = q.get('state') === 'hover';

function SessionMenu() {
	return (
		<DropdownMenu open>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					className="absolute inset-0 flex items-center justify-center rounded
						text-neutral-400"
				>
					<MoreHorizontal className="h-3.5 w-3.5" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				side="right"
				align="start"
				sideOffset={4}
				className="min-w-[140px]"
			>
				<DropdownMenuItem className="cursor-pointer gap-2 text-xs">
					<GitFork className="h-3.5 w-3.5" />
					Fork 会话
				</DropdownMenuItem>
				<DropdownMenuSeparator />
				<DropdownMenuItem
					id="last-item"
					className={
						'cursor-pointer gap-2 text-xs text-red-600 ' +
						(hover ? 'bg-red-50' : 'focus:bg-red-50 focus:text-red-600')
					}
				>
					<Bin className="h-3.5 w-3.5" />
					删除会话
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function Page() {
	return (
		<div
			className="relative"
			style={{ width: 700, height: 320, background: '#ffffff' }}
		>
			{/* sidebar panel + its 6px scrollbar, sitting behind the popup */}
			<div
				className="absolute left-0 top-0 h-full"
				style={{ width: 250, background: '#f8f8f8' }}
			/>
			<div
				className="absolute"
				style={{
					left: 37,
					top: 0,
					width: 6,
					height: '100%',
					background: '#d4d4d4',
				}}
			/>

			<div className="relative ml-5 mt-20 h-5 w-5">
				<SessionMenu />
			</div>

			<div
				className="left-300 top-110 absolute"
				style={{ left: 300, top: 110 }}
			>
				<SettingSelect
					value="husk"
					onChange={() => {}}
					placeholder="选择主题"
					prefix={
						<span
							className="rounded bg-[#e0edff] px-1.5 py-0.5 text-[10.5px]
								font-bold leading-none text-[#2563eb]"
						>
							Aa
						</span>
					}
					options={[
						{ value: 'husk', label: 'Husk' },
						{ value: 'light', label: '浅色' },
						{ value: 'dark', label: '深色' },
					]}
				/>
			</div>
		</div>
	);
}

createRoot(document.getElementById('root')!).render(<Page />);
