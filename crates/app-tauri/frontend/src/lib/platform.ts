/**
 * Platform detection without plugin-os.
 *
 * WKWebView on macOS always includes "Macintosh" in the UA string,
 * and `navigator.platform` returns "MacIntel" on Intel Macs.
 * This is reliable inside Tauri's WebView.
 */
const ua = navigator.userAgent;
const platform = navigator.platform ?? "";

export const isMac = /Macintosh|Mac OS X/.test(ua) || /^Mac/.test(platform);

export const isWindows = /Windows/.test(ua) || /^Win/.test(platform);

/** Linux covers everything that isn't macOS or Windows — including the
 *  BSDs. We don't have a Tauri backend for them today, but the
 *  developer-settings UI gates the X11/Wayland picker on this flag so a
 *  non-Linux user can't write a meaningless flag into `settings.toml`. */
export const isLinux = !isMac && !isWindows;

/** A shortcut spelled the way this platform writes it: `⌘⇧J` on macOS,
 *  `Ctrl+Shift+J` elsewhere. `mod` is ⌘ on macOS and Ctrl otherwise. */
export function shortcutLabel(
 keys: ("mod" | "shift" | "alt" | string)[],
 mac = isMac,
): string {
 const mapped = keys.map((key) => {
  if (key === "mod") return mac ? "\u2318" : "Ctrl";
  if (key === "shift") return mac ? "\u21e7" : "Shift";
  if (key === "alt") return mac ? "\u2325" : "Alt";
  return key.length === 1 ? key.toUpperCase() : key;
 });
 return mac ? mapped.join("") : mapped.join("+");
}

/** The terminal panel's toggle (see MainLayout). */
export const TERMINAL_SHORTCUT = shortcutLabel(["mod", "shift", "j"]);
