use super::Surface;
use objc2::{
    class, define_class, ffi, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyClass, AnyObject},
    sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::{cell::RefCell, ffi::c_void, ptr};

static KEY: u8 = 0;

struct Backdrop {
    glass: Retained<AnyObject>,
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        // SAFETY: the WebView's associated owner is used and released on the main thread.
        unsafe {
            let _: () = msg_send![&*self.glass, removeFromSuperview];
        }
    }
}

struct State {
    web: Weak<AnyObject>,
    revision: f64,
    backdrop: Option<Backdrop>,
    surface: Option<Surface>,
    observing: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HuskSidebarGlassOwner"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<State>]
    struct Owner;

    unsafe impl NSObjectProtocol for Owner {}

    impl Owner {
        #[unsafe(method(changed:))]
        fn changed(&self, _notification: &AnyObject) {
            let state = self.ivars().borrow();
            if state.surface.is_none() { return; }
            if let Some(web) = state.web.load() {
                // Native changes request a fresh DOM measurement; no timer runs while idle.
                unsafe {
                    let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
                    let reduce: bool = msg_send![&*workspace, accessibilityDisplayShouldReduceTransparency];
                    let script = NSString::from_str(
                        &format!("{}window.dispatchEvent(new Event('husk-sidebar-glass-change'))",
                            if reduce { "document.documentElement.removeAttribute('data-sidebar-glass');" } else { "" }));
                    let _: () = msg_send![&*web, evaluateJavaScript: &*script,
                        completionHandler: ptr::null::<AnyObject>()];
                }
            }
        }
    }
);

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: selector observers are removed before the associated object is destroyed.
        unsafe {
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![&*center, removeObserver: &*self];
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let center: Retained<AnyObject> = msg_send![&*workspace, notificationCenter];
            let _: () = msg_send![&*center, removeObserver: &*self];
        }
    }
}

unsafe fn owner(web: &AnyObject, mtm: MainThreadMarker) -> Retained<Owner> {
    let key = (&KEY as *const u8).cast::<c_void>();
    let existing = unsafe { ffi::objc_getAssociatedObject(web as *const _ as *mut _, key) };
    if !existing.is_null() {
        return unsafe { Retained::retain(existing.cast_mut().cast::<Owner>()).unwrap() };
    }
    let owner = Owner::alloc(mtm).set_ivars(RefCell::new(State {
        web: Weak::new(web),
        revision: -1.0,
        backdrop: None,
        surface: None,
        observing: false,
    }));
    let owner: Retained<Owner> = unsafe { msg_send![super(owner), init] };
    unsafe {
        ffi::objc_setAssociatedObject(
            web as *const _ as *mut _,
            key,
            Retained::as_ptr(&owner).cast_mut().cast(),
            ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
    }
    owner
}

unsafe fn observe(owner: &Owner, web: &AnyObject, enabled: bool) {
    unsafe {
        let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let workspace_center: Retained<AnyObject> = msg_send![&*workspace, notificationCenter];
        if !enabled {
            let _: () = msg_send![&*center, removeObserver: owner];
            let _: () = msg_send![&*workspace_center, removeObserver: owner];
            return;
        }
        let window: *mut AnyObject = msg_send![web, window];
        for name in [
            "NSWindowDidResizeNotification",
            "NSWindowDidChangeBackingPropertiesNotification",
            "NSWindowDidEnterFullScreenNotification",
            "NSWindowDidExitFullScreenNotification",
            "NSWindowDidBecomeKeyNotification",
            "NSWindowDidResignKeyNotification",
        ] {
            let _: () = msg_send![&*center, addObserver: owner, selector: sel!(changed:),
                name: &*NSString::from_str(name), object: window];
        }
        let _: () = msg_send![&*workspace_center, addObserver: owner, selector: sel!(changed:),
            name: &*NSString::from_str("NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification"),
            object: ptr::null::<AnyObject>()];
    }
}

/// The caller lends a live WKWebView on the main thread.
pub unsafe fn update(
    web: *mut AnyObject,
    revision: f64,
    surface: Option<Surface>,
) -> Result<bool, String> {
    let mtm = MainThreadMarker::new().ok_or("sidebar glass requires the main thread")?;
    if !objc2::available!(macos = 26.0) {
        return Ok(false);
    }
    let Some(glass_class) = AnyClass::get(c"NSGlassEffectView") else {
        return Ok(false);
    };
    let web = unsafe { web.as_ref() }.ok_or("missing WKWebView")?;
    let owner = unsafe { owner(web, mtm) };
    let mut state = owner.ivars().borrow_mut();
    if revision <= state.revision {
        return Ok(false);
    }
    state.revision = revision;
    state.surface = surface;
    let observing = state.surface.is_some();
    if observing != state.observing {
        unsafe {
            observe(&owner, web, observing);
        }
        state.observing = observing;
    }
    unsafe {
        let window: *mut AnyObject = msg_send![web, window];
        let parent: *mut AnyObject = msg_send![web, superview];
        if window.is_null() || parent.is_null() {
            return Err("detached WKWebView".into());
        }
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: bool = msg_send![&*workspace, accessibilityDisplayShouldReduceTransparency];
        let Some(surface) = state.surface.as_ref().filter(|_| !reduce) else {
            state.backdrop = None;
            return Ok(false);
        };
        let bounds: NSRect = msg_send![web, bounds];
        let flipped: bool = msg_send![web, isFlipped];
        let [x, y, w, h] = surface.clipped(bounds.size.width, bounds.size.height, flipped);
        let local = NSRect::new(
            NSPoint::new(bounds.origin.x + x, bounds.origin.y + y),
            NSSize::new(w, h),
        );
        let frame: NSRect = msg_send![web, convertRect: local, toView: parent];
        let dark = surface.dark;
        let [r, g, b] = surface.background;
        if state.backdrop.is_none() {
            let allocated: *mut AnyObject = msg_send![glass_class, alloc];
            let glass: *mut AnyObject = msg_send![allocated, initWithFrame: frame];
            let glass = Retained::from_raw(glass).ok_or("failed to create sidebar glass")?;
            let _: () = msg_send![&*glass, setStyle: 0isize];
            let _: () = msg_send![&*glass, setCornerRadius: 0.0f64];
            let _: () =
                msg_send![parent, addSubview: &*glass, positioned: -1isize, relativeTo: web];
            state.backdrop = Some(Backdrop { glass });
        }
        let glass = &state.backdrop.as_ref().unwrap().glass;
        let _: () = msg_send![&**glass, setFrame: frame];
        let _: () = msg_send![&**glass, setHidden: w <= 0.0 || h <= 0.0];
        let appearance: Retained<AnyObject> = msg_send![class!(NSAppearance),
            appearanceNamed: &*NSString::from_str(if dark { "NSAppearanceNameDarkAqua" } else { "NSAppearanceNameAqua" })];
        let _: () = msg_send![&**glass, setAppearance: &*appearance];
        let tint: Retained<AnyObject> = msg_send![class!(NSColor),
            colorWithSRGBRed: r, green: g, blue: b, alpha: 0.58f64];
        let _: () = msg_send![&**glass, setTintColor: &*tint];
        let clear: Retained<AnyObject> = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setOpaque: false];
        let _: () = msg_send![window, setBackgroundColor: &*clear];
        // Wry uses this same WKWebView KVC property for transparent windows.
        let _: () = msg_send![web, setValue: &*NSNumber::numberWithBool(false),
            forKey: &*NSString::from_str("drawsBackground")];
        let _: () = msg_send![web, setUnderPageBackgroundColor: &*clear];
        Ok(true)
    }
}
