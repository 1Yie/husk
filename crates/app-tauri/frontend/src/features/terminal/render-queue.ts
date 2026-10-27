import { isLayoutResizing } from "@/lib/layout-resize";

type Task = { start: () => void; foreground: () => boolean };
const pending = new Map<number, Task>();
let nextId = 0;
let frame: number | undefined;

function tick() {
  frame = undefined;
  try {
    if (isLayoutResizing()) return;
    const entry = [...pending].find(([, task]) => task.foreground()) ?? pending.entries().next().value;
    if (!entry) return;
    pending.delete(entry[0]);
    entry[1].start();
  } finally {
    if (pending.size && frame === undefined) frame = requestAnimationFrame(tick);
  }
}

/** Run at most one terminal initialization/fit per frame. Selection can change
 * while queued, so foreground priority is evaluated when the work runs. */
export function queueTerminalWork(start: () => void, foreground: () => boolean = () => false): () => void {
  const id = ++nextId;
  pending.set(id, { start, foreground });
  if (frame === undefined) frame = requestAnimationFrame(tick);
  return () => {
    pending.delete(id);
    if (!pending.size && frame !== undefined) {
      cancelAnimationFrame(frame);
      frame = undefined;
    }
  };
}
