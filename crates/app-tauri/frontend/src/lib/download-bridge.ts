// streamdown's exports (diagram SVG/PNG/MMD, table CSV, image files) are Blob →
// object URL → `<a download>`. Tauri wires no download handler, so those clicks
// went nowhere; this reads the blob and hands the bytes to the save dialog.

import { toast } from "sonner";
import { saveDownload } from "@/lib/agent-ipc/sessions";

/** streamdown revokes the URL in the same tick it clicks — these lives are extended. */
const held = new Set<string>();
const originalRevoke = URL.revokeObjectURL.bind(URL);

async function bytesToBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  // Chunked: a spread of a multi-MB buffer blows the argument limit.
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/** Tauri only: a plain browser already saves `<a download>` natively. */
export function installDownloadBridge() {
  if (!("__TAURI_INTERNALS__" in window)) return;

  URL.revokeObjectURL = (url: string) => {
    if (!held.delete(url)) return originalRevoke(url);
    // Released long after the round-trip; a miss costs one blob's memory.
    window.setTimeout(() => originalRevoke(url), 60_000);
  };

  document.addEventListener(
    "click",
    (event) => {
      const anchor = (event.target as Element | null)?.closest?.("a[download]");
      const href = anchor?.getAttribute("href") ?? "";
      if (!anchor || !href.startsWith("blob:")) return;
      // Capture phase: the URL is in `held` before streamdown revokes it.
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
