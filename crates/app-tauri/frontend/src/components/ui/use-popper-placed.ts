// Radix popper placement gate.
//
// Radix positions popper content in a wrapper it parks above the viewport
// (`transform: translate(0, -200%)`, left/top 0) until its first measurement.
// Two hazards live in that window, and both read as "the menu flashes in the
// top-left corner before appearing":
//
//   1. The parking transform is the ONLY thing hiding the unpositioned state.
//      An engine that paints the insertion before applying it shows the menu at
//      the viewport's corner (WebKitGTK does).
//   2. The FIRST measurement can be wrong. With React StrictMode the double
//      mount resets Radix's anchor point, so the content measures against a
//      zero rectangle and reports a real-looking position at the origin —
//      `translate(2px, 0px)`, i.e. the side offset applied to (0,0) — and one
//      frame later the true anchor lands and it jumps to the cursor.
//      `isPositioned` is already true in that frame, so gating on "unpositioned"
//      alone never catches it. (Measured in WebKitGTK: that frame is painted at
//      opacity ~0.08 — a brief flash in the corner.)
//
// Both are handled here: the content renders `invisible` until the placement has
// SETTLED — the same transform on two consecutive samples — so a
// wrong-but-plausible measurement never becomes visible, and a painted
// unpositioned frame has nothing to paint. Cost: one frame (~16ms) between
// Radix placing the menu and it appearing.
//
// The signal is the DOM itself, not a Radix prop: the popper chain
// (context-menu → menu → popper) drops unknown props, and
// `[data-radix-popper-content-wrapper]` is the one element whose style says
// where the menu actually landed.

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
