// Radix can briefly measure a zero anchor under StrictMode and place a menu
// at the viewport origin. Wait for its transform to settle before painting.
import * as React from "react";

const POPPER_WRAPPER = "[data-radix-popper-content-wrapper]";

// visibility:hidden blocks Radix autofocus. Opacity preserves focus while the
// positioning gate prevents painting and pointer hits. !important also keeps
// the entry animation from painting an unpositioned frame.
export const POPPER_PENDING = "!opacity-0 !pointer-events-none";

export function usePopperPlaced(): {
  placed: boolean;
  attach: (node: HTMLElement | null) => void;
} {
  const [placed, setPlaced] = React.useState(false);
  const [node, attach] = React.useState<HTMLElement | null>(null);

  React.useLayoutEffect(() => {
    setPlaced(false);
    if (!node) return;
    const wrapper = (node.closest(POPPER_WRAPPER) as HTMLElement | null) ?? node;
    let last = "";
    let measuredSamples = 0;
    let frame = 0;
    const tick = () => {
      const transform = wrapper.style.transform;
      // Radix parks unpositioned content with a percentage transform. A
      // negative pixel coordinate can be a valid placement above the viewport.
      if (!transform || transform.includes("-200%")) {
        last = "";
        measuredSamples = 0;
      } else {
        measuredSamples += 1;
        // Poll once per frame: repeated style mutations in one frame do not
        // prove that placement has settled. A moving anchor gets four frames.
        if (transform === last || measuredSamples >= 4) {
          setPlaced(true);
          return;
        }
        last = transform;
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [node]);

  return { placed, attach };
}

/** Compose the placement probe with the ref the caller forwarded. */
export function usePopperPlacedRef<T extends HTMLElement>(
  forwarded: React.ForwardedRef<T>,
): { placed: boolean; setRef: (node: T | null) => void } {
  const { placed, attach } = usePopperPlaced();
  const setRef = React.useCallback(
    (node: T | null) => {
      attach(node);
      if (typeof forwarded === "function") forwarded(node);
      else if (forwarded) forwarded.current = node;
    },
    [attach, forwarded],
  );
  return { placed, setRef };
}
