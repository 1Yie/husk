import type { KeyboardEvent, UIEvent, WheelEvent } from "react";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import type { SidebarPaging } from "@/features/chat/hooks/use-agent-session";

/** One scroll event advances at most the first reached list. A later project's
 * header cannot hide the preceding page boundary, and mounting/re-rendering a
 * sentinel never drains more pages in the absence of scrolling. */
export function loadReachedPage(event: UIEvent<HTMLDivElement>, paging?: SidebarPaging) {
  loadReachedTail(event.currentTarget, paging);
}

function loadReachedTail(container: HTMLDivElement, paging?: SidebarPaging) {
  if (!paging) return;
  const bounds = container.getBoundingClientRect();
  for (const tail of container.querySelectorAll<HTMLElement>("[data-sidebar-tail]")) {
    const rect = tail.getBoundingClientRect();
    if (rect.bottom < bounds.top || rect.top > bounds.bottom + 200) continue;
    const root = tail.dataset.root!;
    const archived = tail.dataset.archived === "true";
    const page = paging.get(root, archived);
    if (page.error || (page.loaded && !page.cursor)) continue;
    if (!page.loading) void paging.loadMore(root, archived);
    return;
  }
}

/** A tall viewport has no scrollTop to change, so no onScroll can arrive.
 * Observe explicit downward intent only; never drain on mount/resize or steal
 * editing/menu/activation gestures, and leave native defaults untouched. */
function acceptsScrollIntent(event: UIEvent<HTMLDivElement>) {
  const container = event.currentTarget;
  const target = event.target;
  return !event.defaultPrevented && container.clientHeight > 0 && container.scrollHeight <= container.clientHeight &&
    !(target instanceof Element && target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="menu"], [role="menuitem"], [role="listbox"]'));
}

export function loadOnDownwardWheel(event: WheelEvent<HTMLDivElement>, paging?: SidebarPaging) {
  if (event.deltaY > 0 && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && acceptsScrollIntent(event)) {
    loadReachedTail(event.currentTarget, paging);
  }
}

export function loadOnPageKey(event: KeyboardEvent<HTMLDivElement>, paging?: SidebarPaging) {
  if ((event.key === "PageDown" || event.key === "End") && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && acceptsScrollIntent(event)) {
    loadReachedTail(event.currentTarget, paging);
  }
}

export function SidebarPageTail({ root, archived = false, paging }: {
  root: string; archived?: boolean; paging?: SidebarPaging;
}) {
  const page = paging?.get(root, archived);
  const busy = useDelayedFlag(page?.loading === true);
  if (!page || (page.loaded && !page.cursor && !page.error)) return null;
  return <div data-sidebar-tail data-root={root} data-archived={String(archived)}
    className="min-h-px pl-7 text-[12px] text-text-secondary" data-tauri-drag-region="false">
    {page.error ? <button type="button" className="py-1" onClick={() => void paging?.retryPage(root, archived)}>
      加载失败，重试
    </button> : busy ? <span role="status" className="block py-1">加载中…</span> : null}
  </div>;
}
