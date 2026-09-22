/**
 * Theme-colour guard. Run: `bun scripts/check-theme-colors.mjs` (or node).
 *
 * `tailwind.config.js` maps `white`/`black`/`neutral-*` onto the `--husk-*`
 * tokens, and those invert between themes. Three mistakes keep coming back, so
 * they are checked here instead of relying on review:
 *
 *   1. `dark:…-[#hex]` — a private colour. Dark values belong in
 *      `src/lib/theme.ts` (surfaces/inks) or come from a token.
 *   2. A copy of the overlay surface signature — menus/popovers must import
 *      `SURFACE_OVERLAY` so they stay one definition.
 *   3. `text-white dark:text-[#…]` — ink that flips twice: light ink in light
 *      mode, light ink again in dark, i.e. invisible on an inverted surface.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("../src", import.meta.url).pathname;
const RULES = [
  ["private dark hex", /dark:(?:hover:|focus:|active:|group-hover\/[\w]+:)*(?:bg|text|border|shadow)-\[#/g],
  ["copied overlay surface", /dark:shadow-\[0_18px_44px/g],
  ["double-flipped ink", /text-white[^"]*dark:text-\[#/g],
];
const ALLOW = new Set(["lib/theme.ts"]);

const files = [];
(function walk(dir) {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.(tsx?|mjs)$/.test(e)) files.push(p);
  }
})(ROOT);

let bad = 0;
for (const f of files) {
  const rel = relative(ROOT, f);
  if (ALLOW.has(rel)) continue;
  const src = readFileSync(f, "utf8");
  for (const [name, re] of RULES) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(src))) {
      const line = src.slice(0, m.index).split("\n").length;
      console.log(`${rel}:${line}  ${name}: ${m[0]}`);
      bad++;
    }
  }
}
console.log(bad === 0 ? `theme colours ok — ${files.length} files scanned` : `${bad} problem(s)`);
process.exit(bad === 0 ? 0 : 1);
