import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./lib/frame-probe";
import { TooltipProvider, GlobalTooltip } from "@/components/ui/tooltip";
import { getAppearance } from "./invoke/agent";
import { initAppearance } from "./lib/appearance";
import { installDownloadBridge } from "./lib/download-bridge";
import "./index.css";

// Exports streamdown builds from a blob (diagram SVG/PNG, table CSV, images)
// only reach the filesystem through the native save dialog — install before
// anything can render a download button.
installDownloadBridge();

// Apply the stored appearance before first paint — the shell reads `dark`
// class + CSS vars off <html>, so waiting for React to mount would flash
// the default light theme for a frame.
void getAppearance()
  .then((cfg) => initAppearance(cfg))
  .catch(() => {});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <TooltipProvider delayDuration={200}>
      <App />
      <GlobalTooltip />
    </TooltipProvider>
  </React.StrictMode>,
);
