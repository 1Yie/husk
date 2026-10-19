// Section ordering shared by every picker that groups its rows — provider
// names upstream come out of a map, so a stable order has to be imposed once
// here instead of in each pane.

export interface OptionSection<T> {
  /** Section heading; `undefined` for the rows that belong to no section. */
  group?: string;
  items: T[];
}

/**
 * Group `items` by `groupOf` into sections: ungrouped rows first (their own
 * section, heading-less), then one section per group in alphabetical order.
 * `sortItems` orders the rows inside every section; omitted, input order wins.
 */
export function groupOptions<T>(
  items: T[],
  groupOf: (item: T) => string | undefined,
  sortItems?: (a: T, b: T) => number,
): OptionSection<T>[] {
  const sections: OptionSection<T>[] = [];
  const ungrouped = items.filter((i) => !groupOf(i));
  if (ungrouped.length) sections.push({ items: ungrouped });
  const byGroup = new Map<string, T[]>();
  for (const item of items) {
    const group = groupOf(item);
    if (!group) continue;
    const bucket = byGroup.get(group);
    if (bucket) bucket.push(item);
    else byGroup.set(group, [item]);
  }
  for (const group of [...byGroup.keys()].sort((a, b) => a.localeCompare(b))) {
    sections.push({ group, items: byGroup.get(group)! });
  }
  if (!sortItems) return sections;
  for (const section of sections) section.items = [...section.items].sort(sortItems);
  return sections;
}
