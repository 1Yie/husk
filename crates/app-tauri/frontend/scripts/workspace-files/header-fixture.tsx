import { createRoot } from "react-dom/client";
import { TooltipProvider, GlobalTooltip } from "../../src/components/ui/tooltip";
import { WorkspaceFiles } from "../../src/features/workspace-files";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", query.get("theme") === "dark");
if (query.has("font")) document.documentElement.style.setProperty("--husk-ui-font", "Georgia");
createRoot(document.getElementById("root")!).render(
  <TooltipProvider>
    <GlobalTooltip />
    <main className="h-full bg-panel">
      <WorkspaceFiles root="/fixture/workspace" onBack={() => {}} />
    </main>
  </TooltipProvider>,
);
