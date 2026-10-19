import { describe, expect, test } from "vitest";
import { groupOptions } from "../group-options";

describe("groupOptions", () => {
  test("sections are alphabetical, ungrouped rows lead, items keep their order", () => {
    const sections = groupOptions(
      [
        { id: "free", group: undefined },
        { id: "z", group: "zeta" },
        { id: "a", group: "alpha" },
        { id: "b", group: "alpha" },
      ],
      (i) => i.group,
    );
    expect(sections.map((s) => s.group)).toEqual([undefined, "alpha", "zeta"]);
    expect(sections[0].items.map((i) => i.id)).toEqual(["free"]);
    expect(sections[1].items.map((i) => i.id)).toEqual(["a", "b"]);
  });

  test("items are sorted inside a section when a comparator is given", () => {
    const sections = groupOptions(
      [
        { id: "b", group: "p" },
        { id: "a", group: "p" },
      ],
      (i) => i.group,
      (a, b) => a.id.localeCompare(b.id),
    );
    expect(sections[0].items.map((i) => i.id)).toEqual(["a", "b"]);
  });
});
