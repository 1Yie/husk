// Entry — mounts the App and the global chrome stylesheet. Kept minimal:
// all state lives in the `useAgent*` hooks; components are pure views.

import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app";
import "./app.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
