/** Dev-only frame probe — press Ctrl+Shift+P to toggle a tiny HUD showing
 *  frame gaps (the lag signature), event rate, and view-cache size. If the
 *  page "stays laggy" after a load, the HUD shows whether the main thread
 *  is actually busy (frames >16ms constantly) or the cost is elsewhere.
 *  No-op in production builds. */
if (import.meta.env.DEV) {
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
}
export {};
