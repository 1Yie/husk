import { useState } from "react";
import { createRoot } from "react-dom/client";
import "../../src/index.css";
import { DockTabs } from "@/features/chat/components/dock/dock-tabs";
import { WorkspaceTools } from "@/features/chat/components/dock/workspace-tools";
import { WorkspaceFiles } from "@/features/workspace-files";
import type { DockTab } from "@/features/chat/components/dock/state";

function Fixture() {
  const [active, setActive] = useState<DockTab>("tools");
  return <div data-workspace-fixture data-active={active}
    style={{ width: "100vw", height: "100vh", background: "color-mix(in srgb, var(--husk-n50) 70%, transparent)" }}
    className="flex flex-col">
    <DockTabs tabs={["files", "changes", "artifacts", "jobs", "subagents"]}
      counts={{ files: 0, changes: 104, artifacts: 3, jobs: 4, subagents: 12, tools: 0 }}
      active={active} onSelect={setActive} />
    <div className="min-h-0 flex-1 overflow-y-auto">
      {active === "files" ? <WorkspaceFiles root="" onBack={() => setActive("tools")} /> :
        <WorkspaceTools root="/home/ichiyo/Workspace/agent-rs" hasChanges
          onFiles={() => setActive("files")} onReview={() => setActive("changes")} />}
    </div>
  </div>;
}

createRoot(document.getElementById("root")!).render(<Fixture />);
