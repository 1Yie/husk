/**
 * Theme surface layers — the ONE place the dark-mode colours of raised UI live.
 *
 * Why this file exists: `tailwind.config.js` maps `white` / `black` / `neutral-*`
 * onto the `--husk-*` tokens, and those **invert** between themes. That is right
 * for page and card surfaces, and wrong for two other cases:
 *
 *   1. Surfaces that are dark by design (tooltip bubbles, hover cards). A
 *      `bg-neutral-900` bubble turns near-white in dark mode, and `text-white`
 *      ink on it turns near-black.
 *   2. Surfaces stacked *on* a card (menus, popovers). `bg-white` resolves to the
 *      same tone as the card underneath, so the layer disappears.
 *
 * Components import these instead of carrying private hexes, so a layer is
 * defined once and every consumer stays in step.
 */

/** Translucent material shared by floating controls and layered surfaces. */
export const SURFACE_FROSTED = [
  "backdrop-blur-md",
  "bg-[color-mix(in_srgb,var(--husk-white)_82%,transparent)]",
  "dark:bg-[color-mix(in_srgb,var(--husk-hover)_82%,transparent)]",
].join(" ");

/** Hover fill retains the translucent material on floating controls. */
export const SURFACE_FROSTED_HOVER = [
  "hover:bg-[color-mix(in_srgb,var(--husk-white)_90%,transparent)]",
  "dark:hover:bg-[color-mix(in_srgb,var(--husk-active)_90%,transparent)]",
].join(" ");

/** Menus, popovers, the composer's mention popup: one step above a card in dark. */
export const SURFACE_OVERLAY = [
  "rounded-xl border border-neutral-200",
  SURFACE_FROSTED,
  "dark:border-[var(--husk-border)]",
  "dark:shadow-[0_18px_44px_-12px_rgba(0,0,0,0.85),0_2px_8px_-2px_rgba(0,0,0,0.6)]",
].join(" ");

/** Hover/focus fill INSIDE an overlay — must read lighter than the surface in dark. */
export const SURFACE_OVERLAY_FOCUS = "focus:bg-[var(--husk-overlay-highlight)]";
export const SURFACE_OVERLAY_HOVER = "hover:bg-[var(--husk-overlay-highlight)]";
export const SURFACE_OVERLAY_SELECTED = "bg-[var(--husk-overlay-highlight)]";

/** The same fill for a row that stays open (submenu trigger). */
export const SURFACE_OVERLAY_OPEN =
  "data-[state=open]:bg-[var(--husk-overlay-highlight)]";

/** Separator inside an overlay. */
export const OVERLAY_DIVIDER = "bg-[var(--husk-overlay-divider)]";
export const BORDER_OVERLAY_DIVIDER = "border-[color:var(--husk-overlay-divider)]";
export const OVERLAY_HEADING = "font-medium text-neutral-800";

/** Secondary and status ink remains readable over the overlay's translucent fill. */
export const INK_OVERLAY_SECONDARY = "text-[color:var(--husk-overlay-secondary)]";
export const INK_OVERLAY_WARNING = "text-[color:var(--husk-overlay-warning)]";
export const INK_OVERLAY_SUCCESS = "text-[color:var(--husk-overlay-success)]";
export const INK_OVERLAY_ERROR = "text-[color:var(--husk-overlay-error)]";
export const INK_OVERLAY_DESTRUCTIVE =
  `${INK_OVERLAY_ERROR} focus:text-[color:var(--husk-overlay-error)]`;

/** Always-dark bubble (tooltip, hover card): token surface, literal ink. */
export const SURFACE_BUBBLE = "bg-neutral-900 dark:bg-[var(--husk-card)]";
export const INK_ON_BUBBLE = "text-zinc-50";
export const BORDER_ON_BUBBLE = "border-zinc-700/60";

/** Muted label ink that stays legible on both themes' panels. */
export const INK_MUTED = "text-neutral-500 dark:text-[#9a9aa4]";

/** The activity heat-map's "nothing happened" cell — a flat neutral in BOTH
 *  themes. `--husk-n800` belongs to the inverted dark ramp, so it would paint
 *  a near-white square on the dark card. Bumped from `#26262c` — that was a
 *  near-exact match for the `--husk-card` (`#26262d`) the empty cell sits on,
 *  so in dark mode the zero-activity days dissolved into the card and only
 *  the active (green) squares showed. `#33333c` stays one step above the card. */
export const SURFACE_ACTIVITY_EMPTY = "bg-neutral-100 dark:bg-[#33333c]";

/** Form controls (input, textarea, quiet buttons): a visible well on both themes.
 *  `bg-transparent` left them identical to the page background in dark mode. */
export const SURFACE_INPUT = "bg-white dark:bg-[var(--husk-card)] border-hairline";
// To override this, cancel the same variant (`bg-transparent dark:bg-transparent`);
// a plain `bg-transparent` leaves the dark surface in place.
