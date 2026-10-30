// Runs against production-bundled CSS and the real xterm in Native WebKit.
// Local font candidates are test fixtures, not the application's font catalog.
import { useLayoutStore } from "@/app/layout/layout-store";
import { quoteFontFamily, BUNDLED_TERMINAL_FONT } from "@/features/terminal/fonts";
import { useTerminalPrefs } from "@/features/terminal/preferences";

const assert = (value: unknown, message: string) => { if (!value) throw new Error(message); };
const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
async function until(check: () => boolean) {
  const deadline = performance.now() + 10000;
  while (!check()) {
    if (performance.now() > deadline) {
      const fonts = [...document.querySelectorAll<HTMLElement>(".xterm-rows")].map((node) => ({
        family: getComputedStyle(node).fontFamily, width: node.getBoundingClientRect().width,
        hidden: !!node.closest("[hidden]"),
      }));
      throw new Error(`Font probe timed out waiting for xterm: ${JSON.stringify({ open: useLayoutStore.getState().terminalOpen, fonts })}`);
    }
    await frame();
  }
  await frame();
}

function raster(text: string, family: string) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 48;
  const context = canvas.getContext("2d")!;
  context.font = `28px ${family}`;
  context.fillText(text, 4, 34);
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  let hash = 2166136261, ink = 0;
  for (let i = 3; i < pixels.length; i += 4) {
    hash = Math.imul(hash ^ pixels[i], 16777619);
    ink += pixels[i];
  }
  return { hash: hash >>> 0, ink, width: context.measureText(text).width };
}

export async function checkTerminalFonts() {
  const original = useTerminalPrefs.getState().fontFamily;
  const root = document.documentElement;
  const originalCode = root.style.getPropertyValue("--husk-code-font");
  const rows = () => [...document.querySelectorAll<HTMLElement>(".terminal-panel .xterm-rows")]
    .find((node) => !node.closest("[hidden]") && node.getBoundingClientRect().width > 0);
  const currentStack = () => rows() ? getComputedStyle(rows()!).fontFamily : "";
  try {
    // The preceding close-all check ends at data-open=false. Let its reveal
    // teardown finish before starting a separate font-rendering scenario.
    await until(() => !document.querySelector(".terminal-panel__inner"));
    root.style.setProperty("--husk-code-font", "serif");
    useTerminalPrefs.getState().update({ fontFamily: "" });
    useLayoutStore.setState({ terminalOpen: true });
    await until(() => currentStack().includes(BUNDLED_TERMINAL_FONT));
    const faces = await document.fonts.load(`28px "${BUNDLED_TERMINAL_FONT}"`, "Mi0\uf120\ue0b0\uf09b");
    assert(faces.length > 0 && faces.every((face) => face.status === "loaded"), "Bundled font must actually load from production assets");
    await document.fonts.ready;
    await frame();
    const bundled = quoteFontFamily(BUNDLED_TERMINAL_FONT);
    const ascii = "Mi0Wl";
    const baseline = raster(ascii, bundled);
    assert(baseline.ink > 0, "Bundled ASCII must paint pixels");
    assert(raster(ascii, currentStack()).hash === baseline.hash, "Default xterm font must render bundled JB, not the app code font");
    const cell = raster("M", bundled).width;
    const missingGlyph = raster("\u{10ffff}", bundled).hash;
    const icons = ["\uf120", "\ue0b0", "\uf09b"].map((icon) => {
      const sample = raster(icon, bundled);
      assert(sample.ink > 0 && sample.hash !== missingGlyph, "Nerd icons must paint real glyphs, not missing-glyph boxes");
      assert(Math.abs(sample.width - cell) < 0.1, "Nerd Mono icons must occupy one ASCII cell");
      assert(raster(icon, currentStack()).hash === sample.hash, "Default xterm icon fallback must render bundled Nerd Mono");
      return { codepoint: icon.codePointAt(0)!.toString(16), width: sample.width, hash: sample.hash };
    });

    let systemFamily = "";
    for (const family of ["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono"]) {
      try {
        await new FontFace("terminal-local-probe", `local(${quoteFontFamily(family)})`).load();
        if (raster(ascii, quoteFontFamily(family)).hash !== baseline.hash) { systemFamily = family; break; }
      } catch { /* Try the next standard Linux test font. */ }
    }
    assert(systemFamily, "Native font switching needs an installed Linux monospace test font");
    useTerminalPrefs.getState().update({ fontFamily: quoteFontFamily(systemFamily) });
    await until(() => currentStack().includes(systemFamily));
    const selected = raster(ascii, currentStack());
    assert(selected.hash !== baseline.hash, "Changing the system font must actually change rasterized text");
    assert(selected.hash === raster(ascii, quoteFontFamily(systemFamily)).hash, "Selected system family must be primary");

    useTerminalPrefs.getState().update({ fontFamily: '"Husk Missing Font Fixture 1937"' });
    await until(() => currentStack().includes("Husk Missing Font Fixture 1937"));
    assert(raster(ascii, currentStack()).hash === baseline.hash, "Unavailable selections must fall back to bundled JB");
    assert(raster("\uf120", currentStack()).hash === icons[0].hash, "Unavailable selections must preserve Nerd icons");
    return { loadedFaces: faces.length, systemFamily, defaultAsciiHash: baseline.hash,
      selectedAsciiHash: selected.hash, icons, unavailableFallback: true, codeFontIndependent: true };
  } finally {
    useTerminalPrefs.getState().update({ fontFamily: original });
    if (originalCode) root.style.setProperty("--husk-code-font", originalCode);
    else root.style.removeProperty("--husk-code-font");
    useLayoutStore.setState({ terminalOpen: false });
  }
}
