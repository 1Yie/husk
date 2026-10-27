/** Wait for the real layout to reach its target for two frames. This also
 * covers reduced motion, reversed transitions and initial-open panels.
 * A null target pauses settlement during a pointer resize. */
export function afterTerminalReveal(
  panel: HTMLElement,
  targetHeight: () => number | null,
  settle: () => void,
): () => void {
  let frame: number;
  let completeFrame = false;
  let previousWidth: number | undefined;
  const check = () => {
    const target = targetHeight();
    const box = panel.getBoundingClientRect();
    const widthSettled = previousWidth === undefined || Math.abs(box.width - previousWidth) < 0.5;
    previousWidth = box.width;
    const settled = target !== null && Math.abs(box.height - target) < 0.5 && widthSettled;
    if (settled && completeFrame) {
      settle();
      return;
    }
    completeFrame = settled;
    frame = requestAnimationFrame(check);
  };
  frame = requestAnimationFrame(check);
  return () => cancelAnimationFrame(frame);
}
