import { useState } from "react";
import { createRoot } from "react-dom/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import { TerminalPanel } from "@/features/terminal/terminal-panel";
import "../../src/index.css";

function Fixture() {
  const [open, setOpen] = useState(true);
  return <TooltipProvider>
    <div style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <button style={{ flex: 1 }} onClick={() => setOpen(true)}>打开终端</button>
      <TerminalPanel open={open} onOpenChange={setOpen} cwd="/work/terminal-long-name" />
    </div>
  </TooltipProvider>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
