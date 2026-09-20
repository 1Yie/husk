import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { SettingsWindow } from "./pages/settings";
import { TooltipProvider, GlobalTooltip } from "@/components/ui/tooltip";
import "./index.css";

const isSettings =
  window.location.search.includes("window=settings") ||
  window.location.hash.includes("settings");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <TooltipProvider delayDuration={200}>
      {isSettings ? <SettingsWindow /> : <App />}
      <GlobalTooltip />
    </TooltipProvider>
  </React.StrictMode>,
);
