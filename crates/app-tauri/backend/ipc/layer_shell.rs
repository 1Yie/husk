//! Wayland overlay placement via `wlr-layer-shell` (GTK3 `gtk-layer-shell`).
//!
//! A plain toplevel cannot be placed on Wayland — the client never chooses
//! its own position, so `set_position` is silently ignored, and KWin does not
//! keep a borderless toplevel out of the dock. A layer surface is the
//! protocol's answer: anchored to screen edges, above normal windows, and not
//! a toplevel, so it never appears in the taskbar.
//!
//! Everything here is a no-op unless the compositor supports the protocol
//! (`available`). X11 and compositors without layer-shell keep the original
//! toplevel path.
//!
//! Calls must happen on a window that is NOT yet realized (before `show()`):
//! `gtk_layer_init_for_window` rejects an already-mapped window.
//!
//! Uses `gtk-layer-shell-sys` (FFI) directly: the safe 0.8 wrapper keeps its
//! functions crate-private, so only `is_supported` is reachable through it.

use gtk::prelude::*;
use gtk_layer_shell_sys as ffi;

/// True when this is a Wayland session whose compositor supports layer-shell.
/// Must run on the GTK main thread (Tauri's main thread).
pub fn available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() && unsafe { ffi::gtk_layer_is_supported() != 0 }
}

/// Anchor `win` to the bottom-right corner of its monitor, `margin` px from
/// each edge, and reserve no space (`exclusive_zone = 0`).
pub fn corner_hud(win: &gtk::ApplicationWindow, margin: i32) {
    let w = win.upcast_ref::<gtk::Window>().as_ptr();
    unsafe {
        ffi::gtk_layer_init_for_window(w);
        ffi::gtk_layer_set_namespace(w, c"husk-computer-hud".as_ptr());
        ffi::gtk_layer_set_layer(w, ffi::GTK_LAYER_SHELL_LAYER_OVERLAY);
        ffi::gtk_layer_set_exclusive_zone(w, 0);
        for edge in [
            ffi::GTK_LAYER_SHELL_EDGE_BOTTOM,
            ffi::GTK_LAYER_SHELL_EDGE_RIGHT,
        ] {
            ffi::gtk_layer_set_anchor(w, edge, 1);
            ffi::gtk_layer_set_margin(w, edge, margin);
        }
    }
}

/// Cover the whole monitor: anchor all four edges with zero margins, and
/// reserve no space (`exclusive_zone = -1` = ignore other surfaces' zones).
/// Click-through is applied separately by the caller.
pub fn full_screen(win: &gtk::ApplicationWindow) {
    // tao creates every window at a default 800×600. A layer surface takes its
    // size from the request, so that default would pin the glow to a 800×600
    // box in the top-left instead of covering the monitor. Clear it to 1×1 —
    // with all four edges anchored, the compositor stretches the surface to
    // the monitor regardless of the requested size.
    gtk::prelude::WidgetExt::set_size_request(win, 1, 1);
    gtk::prelude::GtkWindowExt::set_default_size(win, 1, 1);
    let w = win.upcast_ref::<gtk::Window>().as_ptr();
    unsafe {
        ffi::gtk_layer_init_for_window(w);
        ffi::gtk_layer_set_namespace(w, c"husk-computer-glow".as_ptr());
        ffi::gtk_layer_set_layer(w, ffi::GTK_LAYER_SHELL_LAYER_OVERLAY);
        ffi::gtk_layer_set_exclusive_zone(w, -1);
        for edge in [
            ffi::GTK_LAYER_SHELL_EDGE_TOP,
            ffi::GTK_LAYER_SHELL_EDGE_BOTTOM,
            ffi::GTK_LAYER_SHELL_EDGE_LEFT,
            ffi::GTK_LAYER_SHELL_EDGE_RIGHT,
        ] {
            ffi::gtk_layer_set_anchor(w, edge, 1);
        }
    }
}
