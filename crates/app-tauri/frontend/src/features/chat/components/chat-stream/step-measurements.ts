/** A single owner for virtual-step geometry. Native scroll anchoring must be
 * disabled on the scroll root: it would otherwise apply the same delta twice.
 *
 * ResizeObserver delivers a batch after layout and before paint. Read the
 * whole batch first, reconstruct its old positions, then write scrollTop once.
 * Per-step layout effects used to interleave layout reads and scroll writes,
 * and positions of later steps already included earlier steps' size changes.
 */
export interface StepSizeChange {
  /** Client top AFTER all changes in this batch, before our scroll correction. */
  top: number;
  previous: number;
  height: number;
}

export function scrollAnchorDelta(top: number, previous: number, height: number, viewTop: number): number {
  const delta = height - previous;
  return top + previous <= viewTop + 0.5 ? delta : 0;
}

/** Sizes may change above AND inside the viewport in the same delivery.
 * Classify each step by its OLD bottom, not by positions already shifted by
 * preceding changes. Equal tops must already be in DOM order (zero-height
 * steps can coincide); other input order is arbitrary. */
export function stepBatchScrollDelta(changes: readonly StepSizeChange[], viewTop: number): number {
  let precedingDelta = 0;
  let correction = 0;
  for (const change of [...changes].sort((a, b) => a.top - b.top)) {
    correction += scrollAnchorDelta(
      change.top - precedingDelta,
      change.previous,
      change.height,
      viewTop,
    );
    precedingDelta += change.height - change.previous;
  }
  return correction;
}

/** Keep fractions. offsetHeight rounds a 123.5px row into a 124px placeholder. */
export function stepBoxHeight(el: Element): number {
  const exact = el.getBoundingClientRect().height;
  return exact > 0 ? exact : (el as HTMLElement).offsetHeight ?? 0;
}

function observedHeight(entry: ResizeObserverEntry): number {
  const size = entry.borderBoxSize?.[0];
  return size ? size.blockSize : stepBoxHeight(entry.target);
}

export function createStepMeasurements(root: HTMLElement, shouldAnchor: () => boolean) {
  const tracked = new Map<Element, { height: number; onHeight: (height: number) => void }>();
  let carry = 0;
  const observer = new ResizeObserver((entries) => {
    const changed: (StepSizeChange & {
      target: Element;
      tracked: { height: number; onHeight: (height: number) => void };
    })[] = [];
    const anchoring = shouldAnchor();
    for (const entry of entries) {
      const record = tracked.get(entry.target);
      if (!record) continue;
      const height = observedHeight(entry);
      if (height === record.height || !Number.isFinite(height)) continue;
      changed.push({
        target: entry.target,
        tracked: record,
        previous: record.height,
        height,
        top: anchoring ? entry.target.getBoundingClientRect().top : 0,
      });
    }

    // No scroll writes until every size and position has been read. Mount
    // swaps and later text/image/font growth use this exact same path.
    if (anchoring && changed.length) {
      changed.sort((a, b) => a.top - b.top || (
        a.target.compareDocumentPosition(b.target) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1
      ));
      const correction = stepBatchScrollDelta(changed, root.getBoundingClientRect().top);
      if (correction !== 0) {
        const before = root.scrollTop;
        root.scrollTop = before + correction + carry;
        carry += correction - (root.scrollTop - before);
        // A clamped edge is not sub-pixel rounding; never carry its debt.
        if (Math.abs(carry) >= 1) carry = 0;
      }
    } else {
      carry = 0;
    }
    for (const change of changed) {
      change.tracked.height = change.height;
      change.tracked.onHeight(change.height);
    }
  });
  return {
    observe(el: HTMLElement, previousHeight: number, onHeight: (height: number) => void) {
      tracked.set(el, { height: previousHeight, onHeight });
      observer.observe(el);
      return () => {
        tracked.delete(el);
        observer.unobserve(el);
      };
    },
    /** Prepending/revealing history already restored scrollTop in a layout
     * effect. Consume the sizes it restored, before the observer can apply
     * those mount deltas again. Only currently mounted steps are scanned. */
    settle() {
      const sizes = [...tracked].map(([el, record]) => ({ record, height: stepBoxHeight(el) }));
      for (const { record, height } of sizes) {
        record.height = height;
        record.onHeight(height);
      }
      carry = 0;
    },
    disconnect() {
      observer.disconnect();
      tracked.clear();
      carry = 0;
    },
  };
}
