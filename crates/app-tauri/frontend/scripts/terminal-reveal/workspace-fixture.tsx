import { useState } from "react";
import { createRoot } from "react-dom/client";
import "../../src/index.css";
import { DockTabs } from "@/features/chat/components/dock/dock-tabs";
import { WorkspaceTools } from "@/features/chat/components/dock/workspace-tools";
import type { DockTab } from "@/features/chat/components/dock/state";

function Fixture() {
  const [active, setActive] = useState<DockTab>("tools");
  return <div data-workspace-fixture data-active={active} style={{ width: "100vw", height: "100vh" }}
    className="flex flex-col bg-[var(--husk-panel)]">
    <DockTabs tabs={["files", "changes", "artifacts", "jobs", "subagents"]}
      counts={{ files: 0, changes: 104, artifacts: 3, jobs: 4, subagents: 12, tools: 0 }}
      active={active} onSelect={setActive} />
    <div className="min-h-0 flex-1 overflow-y-auto">
      <WorkspaceTools root="/home/ichiyo/Workspace/agent-rs" hasChanges
        onFiles={() => setActive("files")} onReview={() => setActive("changes")} />
    </div>
  </div>;
}

createRoot(document.getElementById("root")!).render(<Fixture />);
