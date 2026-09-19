/**
 * Platform detection without plugin-os.
 *
 * WKWebView on macOS always includes "Macintosh" in the UA string,
 * and `navigator.platform` returns "MacIntel" on Intel Macs.
 * This is reliable inside Tauri's WebView.
 */
const ua = navigator.userAgent;
const platform = navigator.platform ?? "";

export const isMac =
  /Macintosh|Mac OS X/.test(ua) || /^Mac/.test(platform);

export const isWindows =
  /Windows/.test(ua) || /^Win/.test(platform);

export const isLinux = !isMac && !isWindows;