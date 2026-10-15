/** Thinking tiers, lowest → highest.
 *
 *  `value` is the wire value the provider receives. `token` is the short form
 *  the composer's popup slider shows, `label` the full name every settings
 *  surface uses — one list so the names can't drift apart.
 */
export const THINKING_LEVELS = [
  { value: "off", token: "Off", label: "Off" },
  { value: "minimal", token: "Min", label: "Minimal" },
  { value: "low", token: "Low", label: "Low" },
  { value: "medium", token: "Med", label: "Medium" },
  { value: "high", token: "High", label: "High" },
  { value: "xhigh", token: "Xhigh", label: "Xhigh" },
  { value: "max", token: "Max", label: "Max" },
] as const;

/** Pick only from the active model's declared levels. Unknown persisted tiers
 * never become a label or a command just because the global list knows them. */
export function normalizeThinkingLevel(
  requested: string | null | undefined,
  available: readonly string[] = [],
): string {
  const levels = THINKING_LEVELS.filter((level) => available.includes(level.value));
  const value = requested?.trim().toLowerCase();
  return levels.find((level) => level.value === value)?.value
    ?? levels.find((level) => level.value === "medium")?.value
    ?? levels.find((level) => level.value !== "off")?.value
    ?? "off";
}

/** The raw wire value for a tier, or `value→mapped` when the active model's
 *  thinking_level_map rewrites it (e.g. low→max means picking low sends `max`
 *  to the model). */
export function thinkingSuffix(
  value: string,
  map?: Record<string, string | null>,
): string {
  const mapped = map?.[value];
  return typeof mapped === "string" && mapped && mapped !== value
    ? `${value}→${mapped}`
    : value;
}
