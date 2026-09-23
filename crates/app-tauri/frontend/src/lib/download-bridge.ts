// Exports streamdown draws itself — diagram SVG/PNG/MMD, table CSV/Markdown/TSV,
// image files — are built as a Blob, turned into an object URL and clicked
// through an `<a download>`. Inside Tauri that anchor never reaches the
// filesystem (the webview wires no download handler), so every one of those
// buttons silently did nothing. Catch the click, read the blob the anchor
// points at, and hand the bytes to the native save dialog instead.

import { toast } from "sonner";
import { saveDownload } from "../invoke/agent/sessions";

/** Blob URLs headed for the save dialog. streamdown revokes the URL in the same
 *  tick it clicks the anchor, while the bytes are read asynchronously — those
 *  lives are extended here. */
const held = new Set<string>();
const originalRevoke = URL.revokeObjectURL.bind(URL);

async function bytesToBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  // Chunked: `String.fromCharCode(...bytes)` would blow the argument limit on a
  // multi-MB export.
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/** Installed once, from `main.tsx`. A plain browser (vite dev harness, preview)
 *  already saves `<a download>` natively, so the bridge stays out of the way
 *  there. */
export function installDownloadBridge() {
  if (!("__TAURI_INTERNALS__" in window)) return;

  URL.revokeObjectURL = (url: string) => {
    if (!held.delete(url)) return originalRevoke(url);
    // Release well after the save round-trip; a missed release costs one blob's
    // memory, never correctness.
    window.setTimeout(() => originalRevoke(url), 60_000);
  };

  document.addEventListener(
    "click",
    (event) => {
      const anchor = (event.target as Element | null)?.closest?.("a[download]");
      const href = anchor?.getAttribute("href") ?? "";
      if (!anchor || !href.startsWith("blob:")) return;
      // Capturing phase: this runs before the click finishes dispatching, so the
      // URL is in `held` by the time streamdown revokes it.
      event.preventDefault();
      event.stopPropagation();
      held.add(href);
      const name = anchor.getAttribute("download") || "download";
      void (async () => {
        try {
          const blob = await (await fetch(href)).blob();
          if (blob.size === 0) throw new Error("空文件");
          await saveDownload(name, await bytesToBase64(blob));
        } catch (err) {
          console.error("download failed:", err);
          toast.error("保存文件失败");
        }
      })();
    },
    true,
  );
}
