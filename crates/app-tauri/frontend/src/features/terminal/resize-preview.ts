/** Coalesce pointer input to one transform update per frame. Never resize
 * terminal contents while the pointer is held; release commits the final size. */

export function createTerminalResizePreview(paint: (height: number, resizeContent: boolean) => void) {
  let latest: number | undefined;
  let frame: number | undefined;

  const cancel = () => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = latest = undefined;
  };
  const scheduleFrame = () => {
    if (frame !== undefined) return;
    frame = requestAnimationFrame(() => {
      frame = undefined;
      if (latest === undefined) return;
      paint(latest, false);
    });
  };
  return {
    move(height: number) {
      if (height === latest) return;
      latest = height;
      scheduleFrame();
    },
    flush(height: number) {
      cancel();
      paint(height, true);
    },
    cancel,
  };
}
