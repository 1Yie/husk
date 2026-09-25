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

/** Linux covers everything that isn't macOS or Windows — including the
 *  BSDs. We don't have a Tauri backend for them today, but the
 *  developer-settings UI gates the X11/Wayland picker on this flag so a
 *  non-Linux user can't write a meaningless flag into `settings.toml`. */
export const isLinux = !isMac && !isWindows;
