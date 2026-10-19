/** Cache-hit share of a prompt. Truncated (not rounded) to two decimals so a
 *  live 99.9x% reading can never be displayed as 100% before it is exact.
 *  Integer basis points avoid float artefacts (29/100*100 = 28.999…). */
export function formatCacheRate(cached: number, prompt: number): string | null {
  if (!(prompt > 0)) return null;
  const hit = Math.min(Math.max(cached, 0), prompt);
  if (hit >= prompt) return "100%";
  const bp = Math.floor((hit * 10000) / prompt);
  const text = (bp / 100).toFixed(2).replace(/\.?0+$/, "");
  return `${text}%`;
}
