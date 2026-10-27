import { expect, test } from "vitest";
import { visibleDockTabs } from "../dock-tabs";
import type { DockTab } from "../state";

const tabs: DockTab[] = ["files", "changes", "artifacts", "jobs", "subagents"];
const widths = [60, 90, 80, 70, 90];

test("keeps every full label when the lane is wide enough", () => {
  expect(visibleDockTabs(tabs, widths, 406, "files")).toEqual(tabs);
});

test("reserves the more control and prioritizes the current tab", () => {
  expect(visibleDockTabs(tabs, widths, 260, "subagents")).toEqual(["files", "jobs", "subagents"]);
});

test("tools home does not take a slot from content tabs", () => {
  expect(visibleDockTabs(tabs, widths, 190, "tools")).toEqual(["files", "changes"]);
});

test("an exceptionally narrow lane can put all tabs in the menu", () => {
  expect(visibleDockTabs(tabs, widths, 50, "files")).toEqual([]);
});

test("changing counts or growing the lane restores full tabs", () => {
  expect(visibleDockTabs(tabs, [60, 160, 80, 70, 90], 406, "changes")).not.toHaveLength(5);
  expect(visibleDockTabs(tabs, widths, 500, "changes")).toEqual(tabs);
});
