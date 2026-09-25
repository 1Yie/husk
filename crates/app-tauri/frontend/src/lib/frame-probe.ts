/** Dev-feature bootstrap — reads `~/.config/husk/settings.toml` once at
 *  boot via `agent_session get_dev_config` and installs the gated knobs:
 *
 *   - `monitor_panel = true` → Ctrl+Shift+P toggles a tiny HUD showing
 *     frame gaps (the lag signature) + event rate.
 *   - `devtools = true` → F12 / Ctrl+Shift+I/C/J/K reach the WebKitGTK
 *     inspector in a packaged build. Implemented via
 *     `window.__huskDevToolsEnabled`, which `App.tsx` consults inside its
 *     keydown guard.
 *
 *  Both default to `import.meta.env.DEV` so `cargo tauri dev` keeps the
 *  debugging surface without needing the file. Flipping either in the
 *  config needs a reload — same cost as the renderer/gpu flags the same
 *  file carries. */
import { invoke } from "@tauri-apps/api/core";

declare global {
  interface Window {
    __huskMonitorEnabled?: boolean;
    __huskDevToolsEnabled?: boolean;
  }
}

interface DevConfigPayload {
  monitor_panel?: boolean | null;
  devtools?: boolean | null;
}

// Read once at boot — settings changes take effect on next launch, matching
// the renderer/gpu flags that the same file controls. Failures fall back to
// the dev-build default so a broken backend never strips the probe.
async function readDevFlags(): Promise<DevConfigPayload> {
  try {
    return await invoke<DevConfigPayload>("agent_session", {
      op: "get_dev_config",
    });
  } catch {
    return {};
  }
}

readDevFlags().then((cfg) => {
  const monitor = cfg.monitor_panel ?? import.meta.env.DEV;
  const devtools = cfg.devtools ?? import.meta.env.DEV;
  window.__huskMonitorEnabled = monitor;
  window.__huskDevToolsEnabled = devtools;
  if (!monitor) return;

  let hud: HTMLDivElement | null = null;
  let frames = 0;
  let slow = 0;
  let worst = 0;
  let last = performance.now();
  let rafId = 0;
  let running = false;

  const loop = (t: number) => {
    const dt = t - last;
    last = t;
    frames++;
    if (dt > 16.7) slow++;
    if (dt > worst) worst = dt;
    rafId = requestAnimationFrame(loop);
  };

  const paint = () => {
    if (!hud) return;
    hud.textContent = `frames/s=${frames} slow=${slow} worst=${worst.toFixed(0)}ms`;
    frames = 0;
    slow = 0;
    worst = 0;
  };

  const toggle = () => {
    running = !running;
    if (running) {
      hud = document.createElement("div");
      hud.style.cssText =
        "position:fixed;bottom:4px;left:4px;z-index:99999;background:#000;color:#0f0;" +
        "font:11px monospace;padding:2px 6px;pointer-events:none";
      document.body.appendChild(hud);
      last = performance.now();
      rafId = requestAnimationFrame(loop);
      setInterval(paint, 1000);
    } else {
      cancelAnimationFrame(rafId);
      hud?.remove();
      hud = null;
    }
  };

  window.addEventListener("keydown", (e) => {
    if (e.ctrlKey && e.shiftKey && e.key === "P") toggle();
  });
});
export {};
