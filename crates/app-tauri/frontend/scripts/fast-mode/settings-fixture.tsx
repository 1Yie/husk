import { createRoot } from "react-dom/client";
import { TooltipProvider } from "../../src/components/ui/tooltip";
import { ModelPane } from "../../src/features/settings/pages/agent/model";
import "../../src/index.css";

const query = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", query.get("theme") === "dark");
if (query.has("font")) document.documentElement.style.setProperty("--husk-ui-font", '"Noto Sans", sans-serif');
document.documentElement.style.fontSize = `${query.get("fontSize") ?? 16}px`;

createRoot(document.getElementById("root")!).render(
  <TooltipProvider>
    <main className="h-full overflow-y-auto bg-panel p-5">
      <ModelPane ov={null} reload={() => {}} />
    </main>
  </TooltipProvider>,
);
