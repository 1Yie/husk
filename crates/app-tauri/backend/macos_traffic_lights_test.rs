#[cfg(target_os = "macos")]
mod macos_traffic_lights;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{
        class, msg_send,
        rc::{autoreleasepool, Retained},
        runtime::AnyObject,
        MainThreadMarker,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::ptr;

    let _main_thread = MainThreadMarker::new().expect("native test runs on the main thread");
    autoreleasepool(|_| unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let _: bool = msg_send![&*app, setActivationPolicy: 1isize];
        let allocated: *mut AnyObject = msg_send![class!(NSWindow), alloc];
        let window: *mut AnyObject = msg_send![allocated,
            initWithContentRect: NSRect::new(NSPoint::new(120.0, 160.0), NSSize::new(800.0, 600.0)),
            styleMask: 1usize | 2 | 4 | 8 | (1 << 15),
            backing: 2usize,
            defer: false
        ];
        let window = Retained::from_raw(window).expect("native test window");
        let _: () = msg_send![&*window, setReleasedWhenClosed: false];
        let _: () = msg_send![&*window, setTitleVisibility: 1usize];
        let _: () = msg_send![&*window, setTitlebarAppearsTransparent: true];
        let _: () =
            msg_send![&*window, setTitle: &*NSString::from_str("Husk native layout regression")];
        let _: () = msg_send![&*window, orderFront: ptr::null::<AnyObject>()];
        let _: () = msg_send![&*app, finishLaunching];
        macos_traffic_lights::install(Retained::as_ptr(&window).cast_mut()).unwrap();
        macos_traffic_lights::install(Retained::as_ptr(&window).cast_mut()).unwrap();

        let close: *mut AnyObject = msg_send![&*window, standardWindowButton: 0usize];
        let parent: *mut AnyObject = msg_send![close, superview];
        let container: *mut AnyObject = msg_send![parent, superview];
        let win: NSRect = msg_send![&*window, frame];
        for height in [28.0, 36.0, 32.0, 34.0, 28.0, 36.0] {
            let mut frame: NSRect = msg_send![container, frame];
            frame.size.height = height;
            frame.origin.y = win.size.height - height;
            let _: () = msg_send![container, setFrame: frame];
            for kind in 0usize..3 {
                let button: *mut AnyObject = msg_send![&*window, standardWindowButton: kind];
                let mut frame: NSRect = msg_send![button, frame];
                frame.origin.x = 12.0 + kind as f64 * 20.0;
                frame.origin.y = 6.0;
                let _: () = msg_send![button, setFrameOrigin: frame.origin];
            }
            let loop_: Retained<AnyObject> = msg_send![class!(NSRunLoop), currentRunLoop];
            let date: Retained<AnyObject> =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.02f64];
            let _: () = msg_send![&*loop_, runUntilDate: &*date];
            for kind in 0usize..3 {
                let button: *mut AnyObject = msg_send![&*window, standardWindowButton: kind];
                let bounds: NSRect = msg_send![button, bounds];
                let actual: NSRect =
                    msg_send![button, convertRect: bounds, toView: ptr::null::<AnyObject>()];
                let center = win.size.height - actual.origin.y - actual.size.height / 2.0;
                assert!(
                    (center - 18.0).abs() <= 0.25,
                    "container {height}, button {kind}: center {center}"
                );
                let frame: NSRect = msg_send![button, frame];
                assert_eq!(frame.origin.x, 12.0 + kind as f64 * 20.0);
            }
        }
        let _: () = msg_send![&*window, orderOut: ptr::null::<AnyObject>()];
        let _: () = msg_send![&*window, close];
        println!("native traffic-light relayout regression passed (6 layouts, 3 controls)");
    });
}
