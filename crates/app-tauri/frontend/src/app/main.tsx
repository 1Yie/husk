import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import { AppProviders } from "@/app/provider";
import "@/lib/frame-probe";
import "@/index.css";
// Boot splash handoff — #husk-splash is a SIBLING of #root (never inside it),
// so this commit does not touch the node: same parent, same stacking context,
// no animation restart. The effect waits two frames — one for this commit to
// reach the compositor, one for the app's first paint underneath the overlay —
// then flips `.leaving` so only opacity animates 1 → 0.
function AppBootstrap() {
  React.useEffect(() => {
    const splash = document.getElementById("husk-splash");
    if (!splash) return;
    const id1 = requestAnimationFrame(() => {
      const id2 = requestAnimationFrame(() => {
        splash.classList.add("leaving");
        window.setTimeout(() => splash.remove(), 350);
      });
      // Track the inner id for cleanup — the outer one is captured below.
      (AppBootstrap as { _raf2?: number })._raf2 = id2;
    });
    return () => {
      cancelAnimationFrame(id1);
      const id2 = (AppBootstrap as { _raf2?: number })._raf2;
      if (id2 !== undefined) cancelAnimationFrame(id2);
    };
  }, []);
  return <App />;
}
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AppProviders>
      <AppBootstrap />
    </AppProviders>
  </React.StrictMode>,
);
