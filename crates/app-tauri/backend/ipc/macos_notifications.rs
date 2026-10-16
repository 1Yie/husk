//! macOS local notifications via UserNotifications (macOS 11+).
//!
//! The notification center retains the navigation target in `userInfo`. One
//! process-lifetime delegate handles all responses, including notifications
//! delivered before this process started; there are no per-notification waiters.

use std::{
    cell::OnceCell,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use block2::{DynBlock, RcBlock};
use objc2::{
    define_class, msg_send, rc::autoreleasepool, rc::Retained, runtime::Bool,
    runtime::ProtocolObject, AnyThread, DefinedClass, MainThreadMarker,
};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString, NSUUID,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSettings, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use serde_json::Value;

const CATEGORY: &str = "husk.session";
const OPEN_ACTION: &str = "husk.open-session";
const TARGET_KEY: &str = "husk.session-target";
const BUNDLE_REQUIRED: &str = "macOS notifications require a signed Husk.app bundle; build a local app with `cargo tauri build --debug --bundles app -c tauri.dev.conf.json5` and launch that app";

// `UNUserNotificationCenter.delegate` is weak. Retain ours on the main thread
// for the entire event loop; callbacks only use an immutable, thread-safe Fn.
thread_local! {
    static DELEGATE: OnceCell<Retained<NotificationDelegate>> = const { OnceCell::new() };
}
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static AUTHORIZATION: Mutex<AuthorizationQueue> = Mutex::new(AuthorizationQueue {
    requesting: false,
    pending: Vec::new(),
});

struct DelegateState {
    on_click: Box<dyn Fn(Value) + Send + Sync>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HuskNotificationDelegate"]
    #[ivars = DelegateState]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // The frontend has already suppressed the visible conversation's
            // own events. A different session can still need a banner here.
            completion
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let target = autoreleasepool(|_| target_from_response(response));
            // Acknowledge every response, including dismissals/malformed data,
            // without waiting for the window or frontend to finish opening.
            completion.call(());
            if let Some(target) = target {
                (self.ivars().on_click)(target);
            }
        }
    }
);

/// Install before the application finishes launching. This never prompts for
/// permission; authorization is requested only when a notification is sent.
pub fn initialize(on_click: impl Fn(Value) + Send + Sync + 'static) -> Result<(), String> {
    MainThreadMarker::new().ok_or("notification delegate must be installed on the main thread")?;
    autoreleasepool(|_| {
        let bundle = NSBundle::mainBundle();
        // Calling currentNotificationCenter without a real application bundle
        // can raise an Objective-C exception. Reject that context first.
        if bundle.bundleIdentifier().is_none() || !bundle.bundlePath().to_string().ends_with(".app")
        {
            return Err(BUNDLE_REQUIRED.to_owned());
        }
        DELEGATE.with(|slot| {
            slot.get_or_init(|| {
                let delegate = NotificationDelegate::alloc().set_ivars(DelegateState {
                    on_click: Box::new(on_click),
                });
                // SAFETY: NSObject's initializer is valid for this subclass;
                // its Rust ivars were installed before invoking it.
                let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(delegate), init] };
                let center = UNUserNotificationCenter::currentNotificationCenter();
                center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                let action = UNNotificationAction::actionWithIdentifier_title_options(
                    &NSString::from_str(OPEN_ACTION),
                    &NSString::from_str("打开会话"),
                    UNNotificationActionOptions::Foreground,
                );
                let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                    &NSString::from_str(CATEGORY),
                    &NSArray::from_retained_slice(&[action]),
                    &NSArray::<NSString>::new(),
                    UNNotificationCategoryOptions::empty(),
                );
                center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
                delegate
            });
        });
        INITIALIZED.store(true, Ordering::Release);
        Ok(())
    })
}

#[derive(Clone)]
struct Delivery {
    title: String,
    body: Option<String>,
    target: Option<Value>,
}

#[derive(Default)]
struct AuthorizationQueue {
    requesting: bool,
    pending: Vec<Delivery>,
}

/// Enqueue delivery without blocking a Tokio worker or the GUI. Querying the
/// current settings on every send also respects changes in System Settings.
pub fn show(title: String, body: Option<String>, target: Option<Value>) -> Result<(), String> {
    if !INITIALIZED.load(Ordering::Acquire) {
        return Err(BUNDLE_REQUIRED.to_owned());
    }
    let delivery = Delivery {
        title,
        body,
        target,
    };
    autoreleasepool(|_| {
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&RcBlock::new(
                move |settings: NonNull<UNNotificationSettings>| {
                    // SAFETY: Apple supplies a non-null settings object valid
                    // for the lifetime of this completion callback.
                    let status = unsafe { settings.as_ref() }.authorizationStatus();
                    match status {
                        UNAuthorizationStatus::NotDetermined => request_authorization(delivery.clone()),
                        UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional => {
                            deliver(&delivery);
                        }
                        _ => eprintln!("[notify] macOS notifications are disabled; enable Husk in System Settings > Notifications"),
                    }
                },
            ));
    });
    Ok(())
}

fn request_authorization(delivery: Delivery) {
    {
        let mut queue = AUTHORIZATION.lock().unwrap_or_else(|e| e.into_inner());
        queue.pending.push(delivery);
        if queue.requesting {
            return;
        }
        queue.requesting = true;
    }
    // Concurrent first notifications share one native permission request. Keep
    // no lock while invoking Apple's API: the completion may run immediately.
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &RcBlock::new(|granted: Bool, error: *mut NSError| {
                let error = error_message(error);
                let pending = {
                    let mut queue = AUTHORIZATION.lock().unwrap_or_else(|e| e.into_inner());
                    queue.requesting = false;
                    std::mem::take(&mut queue.pending)
                };
                if let Some(error) = error {
                    eprintln!("[notify] macOS notification authorization failed: {error}");
                } else if granted.as_bool() {
                    for delivery in pending {
                        deliver(&delivery);
                    }
                } else {
                    eprintln!("[notify] macOS notification permission was denied; enable Husk in System Settings > Notifications");
                }
            }),
        );
}

fn deliver(delivery: &Delivery) {
    autoreleasepool(|_| {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&delivery.title));
        if let Some(body) = &delivery.body {
            content.setBody(&NSString::from_str(body));
        }
        if let Some(target) = &delivery.target {
            content.setCategoryIdentifier(&NSString::from_str(CATEGORY));
            let key = NSString::from_str(TARGET_KEY);
            let value = NSString::from_str(&target.to_string());
            let info = NSDictionary::from_slices(&[&*key], &[&*value]);
            // SAFETY: the dictionary contains only property-list-compatible
            // NSString keys/values, and is copied by UNNotificationContent.
            // Erase the dictionary's generic types; NSString inherits AnyObject.
            unsafe { content.setUserInfo(info.cast_unchecked()) };
        }
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSUUID::new().UUIDString(),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(
                &request,
                Some(&RcBlock::new(|error: *mut NSError| {
                    if let Some(error) = error_message(error) {
                        eprintln!("[notify] macOS notification delivery failed: {error}");
                    }
                })),
            );
    });
}

fn error_message(error: *mut NSError) -> Option<String> {
    // SAFETY: callers pass the optional NSError received from Apple's
    // completion handler and consume it before that callback returns.
    NonNull::new(error).map(|error| unsafe { error.as_ref() }.localizedDescription().to_string())
}

fn target_from_response(response: &UNNotificationResponse) -> Option<Value> {
    let action = response.actionIdentifier();
    // SAFETY: this framework constant is a non-null process-lifetime NSString.
    let default_action = unsafe { UNNotificationDefaultActionIdentifier };
    if &*action != default_action && action.to_string() != OPEN_ACTION {
        return None;
    }
    let info = response.notification().request().content().userInfo();
    let key = NSString::from_str(TARGET_KEY);
    let value = info.objectForKey(key.as_ref())?;
    let json = value.downcast_ref::<NSString>()?.to_string();
    decode_target(&json)
}

fn decode_target(json: &str) -> Option<Value> {
    let target: Value = serde_json::from_str(json).ok()?;
    target.get("root")?.as_str()?;
    if target.get("session")?.as_i64()? <= 0 {
        return None;
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::decode_target;
    use serde_json::json;

    #[test]
    fn target_round_trips_paths_without_using_a_url() {
        let target = json!({ "root": "/Users/me/项目 #1: branch", "session": 7 });
        assert_eq!(decode_target(&target.to_string()), Some(target));
    }

    #[test]
    fn malformed_or_missing_targets_do_not_navigate() {
        for input in [
            "invalid",
            "null",
            "{}",
            r#"{"root":"/a","session":0}"#,
            r#"{"root":"/a","session":"7"}"#,
            r#"{"session":7}"#,
        ] {
            assert_eq!(decode_target(input), None);
        }
    }
}
