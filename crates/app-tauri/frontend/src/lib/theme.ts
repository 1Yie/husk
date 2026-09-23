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

/** Menus, popovers, the composer's mention popup: one step above a card in dark. */
export const SURFACE_OVERLAY = [
  "rounded-xl border border-neutral-200 bg-white",
  "dark:bg-[var(--husk-hover)] dark:border-[var(--husk-border)]",
  "dark:shadow-[0_18px_44px_-12px_rgba(0,0,0,0.85),0_2px_8px_-2px_rgba(0,0,0,0.6)]",
].join(" ");

/** Hover/focus fill INSIDE an overlay — must read lighter than the surface in dark. */
export const SURFACE_OVERLAY_FOCUS = "focus:bg-neutral-100 dark:focus:bg-[var(--husk-border)]";

/** The same fill for a row that stays open (submenu trigger). */
export const SURFACE_OVERLAY_OPEN =
  "data-[state=open]:bg-neutral-100 dark:data-[state=open]:bg-[var(--husk-border)]";

/** Separator inside an overlay. */
export const OVERLAY_DIVIDER = "bg-neutral-100 dark:bg-[var(--husk-border)]";

/** Always-dark bubble (tooltip, hover card): token surface, literal ink. */
export const SURFACE_BUBBLE = "bg-neutral-900 dark:bg-[var(--husk-card)]";
export const INK_ON_BUBBLE = "text-zinc-50";
export const BORDER_ON_BUBBLE = "border-zinc-700/60";

/** Muted label ink that stays legible on both themes' panels. */
export const INK_MUTED = "text-neutral-500 dark:text-[#9a9aa4]";

/** Form controls (input, textarea, quiet buttons): a visible well on both themes.
 *  `bg-transparent` left them identical to the page background in dark mode. */
export const SURFACE_INPUT = "bg-white dark:bg-[var(--husk-card)] border-hairline";
// To override this, cancel the same variant (`bg-transparent dark:bg-transparent`);
// a plain `bg-transparent` leaves the dark surface in place.
