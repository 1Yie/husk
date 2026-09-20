/**
 * Cross-component selection/quote bus.
 *
 * The chat stream owns the DOM selection; the composer owns the input. A
 * right-click "就选中内容提问" in the stream needs to land text inside the
 * composer without prop-drilling through ChatPage/App — a tiny pub/sub +
 * last-value slot does it (the stream and composer are mounted together,
 * so a queued value is only a safety net for mount-order edge cases).
 */

type QuoteHandler = (text: string) => void;

let handler: QuoteHandler | null = null;
let queued: string | null = null;

/** Composer registers its setter; unregisters on unmount. */
export function onQuoteRequest(fn: QuoteHandler): () => void {
  handler = fn;
  if (queued != null) {
    const q = queued;
    queued = null;
    fn(q);
  }
  return () => {
    if (handler === fn) handler = null;
  };
}

/** Stream side: hand the selected passage to the composer. */
export function requestQuote(text: string) {
  if (handler) handler(text);
  else queued = text;
}

/** Read the current window selection inside `root` (empty string = none). */
export function selectionWithin(root: HTMLElement | null): string {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || !root) return "";
  const range = sel.rangeCount > 0 ? sel.getRangeAt(0) : null;
  if (!range) return "";
  const node = range.commonAncestorContainer;
  if (!root.contains(node)) return "";
  return sel.toString();
}