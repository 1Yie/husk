// Splitter previews change viewport geometry continuously. Expensive scroll
// following and virtualization bookkeeping can settle once after release.
// This is deliberately not React state: pointer moves must not render a store.
let active = 0;
const ended = new Set<() => void>();

export function isLayoutResizing(): boolean {
  return active > 0;
}

export function beginLayoutResize(): () => void {
  active++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    active--;
    if (active === 0) for (const listener of [...ended]) listener();
  };
}

export function onLayoutResizeEnd(listener: () => void): () => void {
  ended.add(listener);
  return () => { ended.delete(listener); };
}
