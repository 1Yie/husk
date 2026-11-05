import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import '../../src/index.css';
import { TooltipProvider } from '@/components/ui/tooltip';
import { ChatStream } from '@/features/chat/components/chat-stream';
import { viewFromHistory } from '@/features/chat/hooks/view-from-history';
import type { ChatMessage } from '@/types';

// One long agentic turn deliberately exceeds the cross-turn mount budget.
// Reasoning breaks the tools into the same alternating folded rows as a
// real coding session; occasional prose changes the placeholder's estimate.
const history: ChatMessage[] = [
	{ role: 'user', content: '检查长会话的混排滚动' },
];
for (let i = 0; i < 160; i++) {
	history.push({
		role: 'assistant',
		reasoning:
			`Reasoning phase ${i}: inspect the diff and check the boundary cases. `.repeat(
				5
			),
		content:
			i % 5 === 0
				? `第 ${i} 步，先检查变更，再验证边界情况。\n\n${'普通文本说明 '.repeat(25)}`
				: '',
		tool_calls: [
			{
				id: `tool-${i}`,
				type: 'function',
				function: {
					name: 'bash',
					arguments: JSON.stringify({
						command: `cargo test -p example-${i} --lib`,
					}),
				},
			},
		],
	});
	history.push({
		role: 'tool',
		tool_call_id: `tool-${i}`,
		content: `tool phase ${i}\n${'test unit ... ok\n'.repeat(15)}`,
	});
}
history.push({
	role: 'assistant',
	content: '检查完成。\n\n```ts\nconst ready = true;\n```\n',
});
const ordinaryHistory: ChatMessage[] = Array.from({ length: 72 }, (_, i) => [
	{ role: 'user' as const, content: `User turn ${i}` },
	{
		role: 'assistant' as const,
		content: `Answer ${i}. ${'A settled response with enough scroll travel. '.repeat(35)}`,
	},
]).flat();
const view = viewFromHistory(
	new URLSearchParams(location.search).has('turns') ? ordinaryHistory : history,
	null,
	0
);
function Fixture() {
	const [session, setSession] = useState(0);
	return (
		<TooltipProvider>
			<div
				style={{
					height: '100vh',
					width: '100vw',
					display: 'flex',
					flexDirection: 'column',
					position: 'relative',
				}}
			>
				<button
					data-testid="switch-session"
					style={{ position: 'absolute', right: 0, zIndex: 10 }}
					onClick={() => setSession((s) => s + 1)}
				>
					Switch session
				</button>
				<ChatStream
					key={session}
					view={view}
					sessionKey={`/scroll:regression-${session}`}
					bottomPad={32}
					suppressPills
				/>
			</div>
		</TooltipProvider>
	);
}
createRoot(document.getElementById('root')!).render(<Fixture />);
