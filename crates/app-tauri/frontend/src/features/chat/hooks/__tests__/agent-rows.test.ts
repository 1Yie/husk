import { describe, expect, test } from "vitest";
import { agentRowsByChild } from "@/features/chat/hooks/agent-rows";
import type { StreamItem } from "@/features/chat/hooks/stream-view";

type ToolItem = Extract<StreamItem, { kind: "tool" }>;

const card = (childId: number, extra: Partial<ToolItem> = {}): ToolItem => ({
  kind: "tool",
  name: `review #${childId}`,
  args: "检查鉴权",
  parent: "delegate",
  childId,
  childIndex: childId,
  hi: 3,
  ...extra,
});

const call = (childId: number, name: string, extra: Partial<ToolItem> = {}): ToolItem => ({
  kind: "tool",
  name,
  args: "src/auth.ts",
  parent: `review #${childId}`,
  childId,
  childIndex: childId,
  hi: 3,
  ...extra,
});

describe("agentRowsByChild", () => {
  test("each delegated child gets a card keyed by its id", () => {
    const rows = agentRowsByChild([card(1), card(2, { content: "结论", ok: true })]);
    expect([...rows.keys()]).toEqual([1, 2]);
    expect(rows.get(1)).toMatchObject({ kind: "agent", childId: 1, label: "review #1", status: "running" });
    expect(rows.get(2)).toMatchObject({ status: "done", detail: ["结论"] });
  });

  test("a child's own calls attach to its card in arrival order", () => {
    const rows = agentRowsByChild([
      card(1),
      call(1, "read_file"),
      call(1, "smart_grep", { content: "命中", ok: true }),
    ]);
    const children = rows.get(1)?.children ?? [];
    expect(children.map((c) => c.label)).toEqual(["read_file", "smart_grep"]);
    expect(children[1]).toMatchObject({ status: "done", detail: ["命中"] });
    // A call is a tool line, not a second agent.
    expect(children[0].kind).toBeUndefined();
  });

  test("calls attach to the child they name, not a sibling", () => {
    const rows = agentRowsByChild([card(1), card(2), call(2, "smart_grep")]);
    expect(rows.get(1)?.children).toBeUndefined();
    expect(rows.get(2)?.children?.map((c) => c.label)).toEqual(["smart_grep"]);
  });

  test("a call whose child has no card is dropped, not drawn loose", () => {
    const rows = agentRowsByChild([call(9, "read_file")]);
    expect(rows.size).toBe(0);
  });

  test("a child's row id names its parent row and its own id", () => {
    const rows = agentRowsByChild([{ kind: "user", text: "委派" }, card(4)]);
    expect(rows.get(4)?.id).toBe("tool-3-c4");
  });

  test("a parked child carries its question and its running state", () => {
    const rows = agentRowsByChild([card(5, { waiting: "用哪条分支？" })]);
    expect(rows.get(5)).toMatchObject({ waiting: "用哪条分支？", status: "running" });
  });

  test("a rejected call reads as rejected; a failed one as aborted", () => {
    const rows = agentRowsByChild([
      card(6),
      call(6, "apply_patch", { uiType: "rejected", content: "拒绝", ok: false }),
      call(6, "smart_read", { content: "错误", ok: false }),
    ]);
    const children = rows.get(6)?.children ?? [];
    expect(children.map((c) => c.status)).toEqual(["rejected", "aborted"]);
  });

  test("a live answer stands in for the report while the card runs", () => {
    const rows = agentRowsByChild([card(7, { live: "先看入口" })]);
    expect(rows.get(7)?.detail).toEqual(["先看入口"]);
  });

  test("the chip hides a call whose args only repeat its name", () => {
    const rows = agentRowsByChild([card(8), call(8, "read_file", { args: "read_file" })]);
    expect(rows.get(8)?.children?.[0].chip).toBe("");
  });
});
