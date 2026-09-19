import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app";
import { SettingsWindow } from "./components/settings/settings-window";
import "./app.css";

const isSettings =
  window.location.search.includes("window=settings") ||
  window.location.hash.includes("settings");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isSettings ? <SettingsWindow /> : <App />}
  </React.StrictMode>,
);
