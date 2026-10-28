import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ModelPicker } from "../../src/features/chat/components/composer-bar/model-picker";
import { TooltipProvider } from "../../src/components/ui/tooltip";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", query.get("theme") === "dark");
if (query.has("font")) document.documentElement.style.setProperty("--husk-ui-font", "Georgia");
const noReasoning = query.has("noReasoning");
const levels = noReasoning ? [] : [
  { value: "low", token: "Low" },
  { value: "medium", token: "Med" },
  { value: "high", token: "High" },
  { value: "xhigh", token: "Max" },
];

function Fixture() {
  const [fast, setFast] = useState(false);
  const [thinking, setThinking] = useState("xhigh");
  return (
    <main className="flex h-full flex-col justify-between gap-6 p-4">
      <input aria-label="其他输入" placeholder="点击此处检查菜单关闭与焦点" />
      <div className="flex justify-end">
        <ModelPicker
          activeModel="model-a"
          activeProvider="test"
          models={[{
            provider: "test", model: "model-a",
            name: query.has("long") ? "用户配置的很长的快速推理模型名称 / A long custom model name" : "GPT 6 Astra",
            supports_fast_mode: true,
          }]}
          onSelectModel={async () => {}}
          hasReasoning={!noReasoning}
          activeThinkingLevel={thinking}
          effectiveThinkingLevels={levels}
          onSelectThinkingLevel={async (level) => setThinking(level)}
          activeFastMode={fast}
          onSelectFastMode={async (enabled) => setFast(enabled)}
          density={query.has("long") ? "tight" : "full"}
        />
      </div>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<TooltipProvider><Fixture /></TooltipProvider>);
