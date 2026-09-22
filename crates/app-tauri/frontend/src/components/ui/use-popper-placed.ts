// Radix popper placement gate.
//
// Radix parks popper content above the viewport until its first measurement, and
// that measurement can be wrong: under StrictMode the double mount resets its
// anchor, so the content measures a zero rect and reports `translate(2px, 0px)` —
// a real-looking position at the viewport's top-left corner, painted for one frame
// by WebKitGTK while `isPositioned` is already true.
//
// The content therefore stays `invisible` until the placement SETTLES: the same
// wrapper transform on two consecutive samples. Cost: one frame (~16ms). The signal
// is the DOM — Radix drops unknown props through the popper chain, and
// `[data-radix-popper-content-wrapper]` carries the applied transform.

import * as React from "react";

const POPPER_WRAPPER = "[data-radix-popper-content-wrapper]";

/** `placed` + the ref to put on the popper content. */
export function usePopperPlaced(): {
  placed: boolean;
  attach: (node: HTMLElement | null) => void;
} {
  const [placed, setPlaced] = React.useState(false);

  const attach = React.useCallback((node: HTMLElement | null) => {
    if (!node) return;
    const wrapper = (node.closest(POPPER_WRAPPER) as HTMLElement | null) ?? node;

    let last = "";
    let measuredSamples = 0;
    let settled = false;
    const reveal = () => {
      if (settled) return;
      settled = true;
      observer.disconnect();
      setPlaced(true);
    };
    const sample = () => {
      if (wrapper.getBoundingClientRect().top < 0) return false; // still parked
      measuredSamples += 1;
      const transform = wrapper.style.transform;
      // Settled = the same placement twice running. The sample cap is the safety
      // net: a position that keeps moving must still end up visible rather than
      // hidden forever (a menu that never appears is worse than a jumpy one).
      if ((transform !== "" && transform === last) || measuredSamples >= 4) {
        reveal();
        return true;
      }
      last = transform;
      return false;
    };

    // Radix writes the placement as an inline transform on the wrapper, so a
    // style mutation is the exact moment to re-check; the animation frame keeps
    // sampling for updates that arrive without a style write.
    const observer = new MutationObserver(() => {
      sample();
    });
    observer.observe(wrapper, { attributes: true, attributeFilter: ["style"] });
    const tick = () => {
      if (settled || sample()) return;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, []);

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
