import { createRoot } from "react-dom/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import { TerminalSettings } from "@/features/settings/pages/terminal";
import { TerminalPanel } from "@/features/terminal/terminal-panel";
import { TerminalSession } from "@/features/terminal/terminal-session";
import "../../src/index.css";

const metrics = { applications: [] as number[], fits: 0 };
(window as unknown as { __settingsPerf: typeof metrics }).__settingsPerf = metrics;
const originalAppearance = TerminalSession.prototype.setAppearance;
TerminalSession.prototype.setAppearance = function (appearance) {
  const start = performance.now();
  try { return originalAppearance.call(this, appearance); }
  finally { metrics.applications.push(performance.now() - start); }
};
const prototype = TerminalSession.prototype as unknown as { fitNow(): boolean };
const originalFit = prototype.fitNow;
prototype.fitNow = function () {
  metrics.fits++;
  return originalFit.call(this);
};

createRoot(document.getElementById("root")!).render(
  <TooltipProvider>
    <div style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <div data-terminal-settings-fixture style={{ flex: 1, overflow: "auto" }}>
        <TerminalSettings />
      </div>
      <TerminalPanel open onOpenChange={() => {}} cwd="/work/settings-performance" />
    </div>
  </TooltipProvider>,
);
