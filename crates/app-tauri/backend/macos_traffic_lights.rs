use objc2::{
    class, define_class, ffi, msg_send,
    rc::{Retained, Weak},
    runtime::AnyObject,
    sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSString};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    ptr,
};

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

static KEY: u8 = 0;
// The standard controls share the application's 36-point title row.
const CENTER_Y: f64 = 18.0;
const FRAME_CHANGED: &str = "NSViewFrameDidChangeNotification";

struct State {
    window: Weak<AnyObject>,
    views: RefCell<Vec<Weak<AnyObject>>>,
    aligning: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HuskTrafficLightsOwner"]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct Owner;

    unsafe impl NSObjectProtocol for Owner {}

    impl Owner {
        #[unsafe(method(changed:))]
        fn changed(&self, _notification: &AnyObject) {
            self.refresh();
        }
    }
);

impl Owner {
    fn refresh(&self) {
        if self.ivars().aligning.replace(true) {
            return;
        }
        if let Some(window) = self.ivars().window.load() {
            // AppKit owns the fullscreen toolbar and can replace its native views.
            unsafe {
                let style: usize = msg_send![&*window, styleMask];
                if style & (1 << 14) == 0 {
                    self.bind_views(&window);
                    align(&window);
                }
            }
        }
        self.ivars().aligning.set(false);
    }

    unsafe fn bind_views(&self, window: &AnyObject) {
        unsafe {
            let close: *mut AnyObject = msg_send![window, standardWindowButton: 0usize];
            if close.is_null() {
                return;
            }
            let parent: *mut AnyObject = msg_send![close, superview];
            if parent.is_null() {
                return;
            }
            let container: *mut AnyObject = msg_send![parent, superview];
            let mut views = [parent, container]
                .into_iter()
                .filter_map(|view| Retained::retain(view))
                .collect::<Vec<_>>();
            for kind in 0usize..3 {
                let button: *mut AnyObject = msg_send![window, standardWindowButton: kind];
                if let Some(button) = Retained::retain(button) {
                    views.push(button);
                }
            }
            let unchanged = {
                let old = self.ivars().views.borrow();
                old.len() == views.len()
                    && old
                        .iter()
                        .zip(&views)
                        .all(|(old, new)| old.load().is_some_and(|old| ptr::eq(&*old, &**new)))
            };
            if unchanged {
                return;
            }
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            let name = NSString::from_str(FRAME_CHANGED);
            let _: () = msg_send![&*center, removeObserver: self,
                name: &*name, object: ptr::null::<AnyObject>()];
            *self.ivars().views.borrow_mut() =
                views.iter().map(|view| Weak::new(&**view)).collect();
            for view in views {
                let _: () = msg_send![&*view, setPostsFrameChangedNotifications: true];
                let _: () = msg_send![&*center, addObserver: self, selector: sel!(changed:),
                    name: &*name, object: &*view];
            }
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // The window releases its owner after selector observations are removed.
        unsafe {
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![&*center, removeObserver: &*self];
        }
    }
}

unsafe fn align(window: &AnyObject) {
    unsafe {
        let win: NSRect = msg_send![window, frame];
        if !win.size.height.is_finite() || win.size.height <= 0.0 {
            return;
        }
        for kind in 0usize..3 {
            let button: *mut AnyObject = msg_send![window, standardWindowButton: kind];
            if button.is_null() {
                continue;
            }
            let parent: *mut AnyObject = msg_send![button, superview];
            if parent.is_null() {
                continue;
            }
            let owner: *mut AnyObject = msg_send![button, window];
            if !ptr::eq(owner, window) {
                continue;
            }
            let bounds: NSRect = msg_send![button, bounds];
            let rect: NSRect =
                msg_send![button, convertRect: bounds, toView: ptr::null::<AnyObject>()];
            let center = NSPoint::new(
                rect.origin.x + rect.size.width / 2.0,
                rect.origin.y + rect.size.height / 2.0,
            );
            if !center.x.is_finite() || rect.size.height <= 0.0 {
                continue;
            }
            let Some(shift) = window_shift(win.size.height - center.y) else {
                continue;
            };
            if shift.abs() <= 0.25 {
                continue;
            }
            let desired = NSPoint::new(center.x, center.y + shift);
            let current: NSPoint =
                msg_send![parent, convertPoint: center, fromView: ptr::null::<AnyObject>()];
            let desired: NSPoint =
                msg_send![parent, convertPoint: desired, fromView: ptr::null::<AnyObject>()];
            let mut frame: NSRect = msg_send![button, frame];
            let y = frame.origin.y + desired.y - current.y;
            if y.is_finite() {
                frame.origin.y = y;
                let _: () = msg_send![button, setFrameOrigin: frame.origin];
            }
        }
    }
}

fn window_shift(center_from_top: f64) -> Option<f64> {
    center_from_top
        .is_finite()
        .then_some(center_from_top - CENTER_Y)
}

/// Observe a live NSWindow on the main thread until that window is destroyed.
pub unsafe fn install(window: *mut AnyObject) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("traffic lights require the main thread")?;
    let window = unsafe { window.as_ref() }.ok_or("missing NSWindow")?;
    let key = (&KEY as *const u8).cast::<c_void>();
    unsafe {
        if !ffi::objc_getAssociatedObject(window as *const _ as *mut _, key).is_null() {
            return Ok(());
        }
        let owner = Owner::alloc(mtm).set_ivars(State {
            window: Weak::new(window),
            views: RefCell::new(Vec::new()),
            aligning: Cell::new(false),
        });
        let owner: Retained<Owner> = msg_send![super(owner), init];
        ffi::objc_setAssociatedObject(
            window as *const _ as *mut _,
            key,
            Retained::as_ptr(&owner).cast_mut().cast(),
            ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
        let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
        for name in [
            "NSWindowDidResizeNotification",
            "NSWindowDidChangeBackingPropertiesNotification",
            "NSWindowDidExitFullScreenNotification",
            "NSWindowDidBecomeKeyNotification",
            "NSWindowDidResignKeyNotification",
            "NSWindowDidUpdateNotification",
        ] {
            let _: () = msg_send![&*center, addObserver: &*owner, selector: sel!(changed:),
                name: &*NSString::from_str(name), object: window];
        }
        owner.refresh();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::window_shift;

    #[test]
    fn handles_classic_glass_and_appkit_relayout_centers() {
        assert_eq!(window_shift(22.0), Some(4.0));
        assert_eq!(window_shift(18.0), Some(0.0));
        assert_eq!(window_shift(14.0), Some(-4.0));
        assert_eq!(window_shift(f64::NAN), None);
    }
}
