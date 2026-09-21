// Dev-only session-load timing — `loadReset()` starts a cycle at the
// sidebar click, `loadStamp(label)` logs ms since the previous stamp.
// Answers "where did the unresponsive window go" without guessing.

let t0 = 0;

export function loadReset() {
  if (!import.meta.env.DEV) return;
  t0 = performance.now();
  console.log("[load] ── start ──");
}

export function loadStamp(label: string) {
  if (!import.meta.env.DEV) return;
  const now = performance.now();
  console.log(`[load] ${label}: +${Math.round(now - t0)}ms`);
  t0 = now;
}
