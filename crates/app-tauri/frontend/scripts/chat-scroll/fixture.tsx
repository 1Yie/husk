import { createRoot } from "react-dom/client";
import "../../src/index.css";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ChatStream } from "@/features/chat/components/chat-stream";
import { viewFromHistory } from "@/features/chat/hooks/view-from-history";
import type { ChatMessage } from "@/types";

// One long agentic turn deliberately exceeds the cross-turn mount budget.
// Reasoning breaks the tools into the same alternating folded rows as a
// real coding session; occasional prose changes the placeholder's estimate.
const history: ChatMessage[] = [{ role: "user", content: "检查长会话的混排滚动" }];
for (let i = 0; i < 160; i++) {
  history.push({
    role: "assistant",
    reasoning: `Reasoning phase ${i}: inspect the diff and check the boundary cases. `.repeat(5),
    content: i % 5 === 0 ? `第 ${i} 步，先检查变更，再验证边界情况。\n\n${"普通文本说明 ".repeat(25)}` : "",
    tool_calls: [{
      id: `tool-${i}`, type: "function",
      function: { name: "bash", arguments: JSON.stringify({ command: `cargo test -p example-${i} --lib` }) },
    }],
  });
  history.push({
    role: "tool", tool_call_id: `tool-${i}`,
    content: `tool phase ${i}\n${"test unit ... ok\n".repeat(15)}`,
  });
}
history.push({ role: "assistant", content: "检查完成。\n\n```ts\nconst ready = true;\n```\n" });
const view = viewFromHistory(history, null, 0);
createRoot(document.getElementById("root")!).render(
  <TooltipProvider>
    <div style={{ height: "100vh", width: "100vw", display: "flex", flexDirection: "column", position: "relative" }}>
      <ChatStream view={view} sessionKey="/scroll:regression" bottomPad={32} suppressPills />
    </div>
  </TooltipProvider>,
);
