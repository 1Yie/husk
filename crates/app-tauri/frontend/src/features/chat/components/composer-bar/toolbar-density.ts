// The composer toolbar's density tiers. The row is measured, never the window:
// the main column narrows when the sidebar or the dock widens, and the toolbar
// only ever knows the width it is given. Explicit tiers beat letting flexbox
// squeeze every control until its label disappears.

export type ToolbarDensity = "full" | "compact" | "tight";

/** From here the labelled pills and the model's name plus thinking chip fit. */
export const TOOLBAR_FULL_MIN = 560;

/** Below this even icons are crowded — the mode controls fold into one button
 *  and the model name gets the tightest cap. */
export const TOOLBAR_COMPACT_MIN = 400;

export function toolbarDensity(width: number): ToolbarDensity {
  if (width >= TOOLBAR_FULL_MIN) return "full";
  if (width >= TOOLBAR_COMPACT_MIN) return "compact";
  return "tight";
}
