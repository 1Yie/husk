// 智能体 panes — one page per nav item under the 「智能体」 group.
import { useEffect, useState } from "react";
import { type AgentOverview, getAgentOverview } from "@/lib/agent-ipc/sessions";
import { InstructionsPane } from "@/features/settings/pages/agent/instructions/index";
import { RulesPane } from "@/features/settings/pages/agent/rules/index";
import { ModelPane } from "@/features/settings/pages/agent/model/index";
import { MemoryPane } from "@/features/settings/pages/agent/memory/index";
import { SkillsPane } from "@/features/settings/pages/agent/skills/index";
import { PluginsPane } from "@/features/settings/pages/agent/plugins/index";
import { McpPane } from "@/features/settings/pages/agent/mcp/index";
import { SubagentPane } from "@/features/settings/pages/agent/subagent/index";

export type AgentTab =
  | "instructions"
  | "rules"
  | "model"
  | "memory"
  | "skills"
  | "plugins"
  | "mcp"
  | "subagent";

/* ============================ 入口 ============================ */

export function AgentSettings({ tab }: { tab: AgentTab }) {
  const [ov, setOv] = useState<AgentOverview | null>(null);

  const reload = () => {
    void getAgentOverview()
      .then(setOv)
      .catch(() => {});
  };
  useEffect(reload, []);

  // Each pane decides for itself which part waits on `ov` — the frame, the
  // headers and the controls stay usable while the overview is in flight.
  return (
    <div className="flex flex-col gap-8 w-full">
      {tab === "instructions" && <InstructionsPane />}
      {tab === "rules" && <RulesPane ov={ov} reload={reload} />}
      {tab === "model" && <ModelPane ov={ov} reload={reload} />}
      {tab === "memory" && <MemoryPane />}
      {tab === "skills" && <SkillsPane ov={ov} reload={reload} />}
      {tab === "plugins" && <PluginsPane ov={ov} onReload={reload} />}
      {tab === "mcp" && <McpPane ov={ov} onReload={reload} />}
      {tab === "subagent" && <SubagentPane ov={ov} reload={reload} />}
    </div>
  );
}
