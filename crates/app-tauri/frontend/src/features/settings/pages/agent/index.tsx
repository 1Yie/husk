// 智能体 panes — one page per nav item under the 「智能体」 group.
import { useEffect, useState } from "react";
import { type AgentOverview, getAgentOverview } from "@/lib/agent-ipc/sessions";
import { InstructionsPane } from "@/features/settings/pages/agent/instructions/index";
import { ModelPane } from "@/features/settings/pages/agent/model/index";
import { SkillsPane } from "@/features/settings/pages/agent/skills/index";
import { McpPane } from "@/features/settings/pages/agent/mcp/index";
import { SubagentPane } from "@/features/settings/pages/agent/subagent/index";

export type AgentTab = "instructions" | "model" | "skills" | "mcp" | "subagent";

/* ============================ 入口 ============================ */

export function AgentSettings({ tab }: { tab: AgentTab }) {
  const [ov, setOv] = useState<AgentOverview | null>(null);

  const reload = () => {
    void getAgentOverview()
      .then(setOv)
      .catch(() => {});
  };
  useEffect(reload, []);

  return (
    <div className="flex flex-col gap-8 w-full">
      {tab === "instructions" && <InstructionsPane />}
      {tab === "model" && <ModelPane ov={ov} reload={reload} />}
      {tab === "skills" && <SkillsPane ov={ov} reload={reload} />}
      {tab === "mcp" && <McpPane ov={ov} reload={reload} />}
      {tab === "subagent" && <SubagentPane ov={ov} reload={reload} />}
    </div>
  );
}
