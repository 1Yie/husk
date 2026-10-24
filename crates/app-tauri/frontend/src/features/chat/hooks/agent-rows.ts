// The delegated children as tool-chip rows, keyed by child id — the same rows the
// chat draws in its agent cards, built from the stream items the page already
// holds. The sidebar reads a child's transcript from here, so it sees what the
// card sees.

import type { ToolChipRow } from "@/features/chat/components/tool-chips/index";
import { toolRowId, type StreamItem } from "@/features/chat/hooks/stream-view";

type ToolItem = Extract<StreamItem, { kind: "tool" }>;

/** One tool item's status, read the way the chat's rows read it. */
function statusOf(item: ToolItem): ToolChipRow["status"] {
  if (item.uiType === "rejected") return "rejected";
  if (item.content === undefined) return "running";
  return item.ok === false ? "aborted" : "done";
}

/** Every delegated child's card, keyed by its id, with the calls it made attached
 *  in arrival order. One forward pass: a child's calls come after its card in the
 *  stream, so the card is already on the map when they arrive. */
export function agentRowsByChild(items: StreamItem[]): Map<number, ToolChipRow> {
  const rows = new Map<number, ToolChipRow>();
  items.forEach((item, idx) => {
    if (item.kind !== "tool" || item.childId == null) return;
    if (item.parent === "delegate") {
      rows.set(item.childId, {
        id: toolRowId(item, idx),
        kind: "agent",
        childId: item.childId,
        segments: item.segments,
        label: item.name,
        chip: item.args ?? "",
        status: statusOf(item),
        detail: item.content ? [item.content] : item.live ? [item.live] : undefined,
        waiting: item.waiting,
        uiType: item.uiType,
      });
      return;
    }
    const card = item.parent ? rows.get(item.childId) : undefined;
    if (!card) return;
    const call: ToolChipRow = {
      id: toolRowId(item, idx),
      label: item.name,
      chip: item.args && item.args !== item.name ? item.args : "",
      status: statusOf(item),
      detail: item.content ? [item.content] : item.live ? [item.live] : undefined,
      waiting: item.waiting,
      uiType: item.uiType,
    };
    card.children = [...(card.children ?? []), call];
  });
  return rows;
}
