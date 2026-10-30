//! Opt-in desktop delivery, independent of GPUI, the Shell and kernel events.
//!
//! Constructing a notifier never contacts the OS. It starts **disabled**: the
//! application injects the persisted desktop-notification preference at startup
//! and after settings changes via [`DesktopNotifications::set_enabled`].
//! That preference defaults to true in husk-host; we deliberately fail closed
//! until it has been read. Window-focus policy, event selection, click routing
//! and the independent `notification_sound` preference belong to the caller.
//!
//! Only explicitly supplied public copy crosses the OS boundary. The default
//! notification is just "Husk", without a body, session identifier, prompt,
//! workspace path, tool arguments or result. Even the title must be safe for a
//! lock screen. Text limits/control removal are NOT secret redaction.
//!
//! Enqueueing uses a bounded, nonblocking queue. One dedicated worker performs
//! all OS calls; no Tokio runtime or UI executor is required. Await the returned
//! receipt with `receipt.recv().await`, or use `try_recv` from a synchronous
//! caller. A receipt reports submission, not whether the user saw a banner.
//! Backend errors are returned, never logged with potentially sensitive copy.
//!
//! Unit tests cannot enable native delivery. An isolated native smoke must set
//! `HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS=1`; this explicit kill switch cannot
//! be overridden by an enabled preference. No credential environment variables
//! or application-profile heuristics affect normal product behavior.
//!
//! Linux needs a session D-Bus notification service (not a display connection).
//! Windows needs an installer-registered AUMID matching the native application.
//! macOS uses UserNotifications with bundle identity and alert authorization,
//! not the deprecated NSUserNotificationCenter backend. There is no foreground
//! presentation delegate or click handler here. System permissions, DND/focus
//! and notification-server policy may suppress all platforms' banners.

use std::{
    ffi::OsStr,
    fmt, io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
};

const QUEUE_CAPACITY: usize = 16;
const TITLE_LIMIT: usize = 128;
const BODY_LIMIT: usize = 512;

/// Explicit, caller-reviewed lock-screen-safe copy; never construct from raw
/// session titles/prompts/tool events without a separate privacy decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    title: String,
    body: Option<String>,
}

impl Default for Notification {
    fn default() -> Self {
        Self::new("Husk")
    }
}

impl Notification {
    /// A public title only, bounded to 128 Unicode scalar values.
    pub fn new(safe_title: impl AsRef<str>) -> Self {
        let title = plain_text(safe_title.as_ref(), TITLE_LIMIT);
        Self {
            title: if title.trim().is_empty() {
                "Husk".into()
            } else {
                title
            },
            body: None,
        }
    }

    /// Explicitly opt into a public body (at most 512 Unicode scalar values).
    /// This is not a redaction function: the caller must exclude private data.
    pub fn with_safe_body(mut self, safe_body: impl AsRef<str>) -> Self {
        let body = plain_text(safe_body.as_ref(), BODY_LIMIT);
        self.body = (!body.trim().is_empty()).then_some(body);
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }
}

fn plain_text(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

/// Windows needs a registered AppUserModelID. No default PowerShell identity
/// is used. Linux ignores this; macOS obtains identity from its app bundle.
#[derive(Clone, Debug, Default)]
pub struct NativeNotificationOptions {
    #[cfg(target_os = "windows")]
    pub windows_app_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// Accepted by the OS API, not a guarantee of presentation under DND/focus.
    Submitted,
    /// Disabled/dropped after enqueueing, before native submission.
    Suppressed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Some failure categories are produced only on other platforms.
pub enum NotificationError {
    Unavailable(&'static str),
    PermissionDenied,
    TimedOut,
    /// Native diagnostics may contain user data. Do not put these in telemetry.
    Backend(String),
    BackendPanicked,
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "Desktop notifications unavailable: {reason}"),
            Self::PermissionDenied => f.write_str("Desktop notification permission denied"),
            Self::TimedOut => f.write_str("Desktop notification backend timed out"),
            Self::Backend(error) => write!(f, "Desktop notification backend: {error}"),
            Self::BackendPanicked => f.write_str("Desktop notification backend panicked"),
        }
    }
}

impl std::error::Error for NotificationError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueError {
    Disabled,
    QueueFull,
    WorkerStopped,
}

impl fmt::Display for EnqueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disabled => "Desktop notifications disabled",
            Self::QueueFull => "Desktop notification queue full",
            Self::WorkerStopped => "Desktop notification worker stopped",
        })
    }
}

impl std::error::Error for EnqueueError {}

pub type DeliveryReceipt = async_channel::Receiver<Result<DeliveryStatus, NotificationError>>;

/// A generation prevents disabled, then re-enabled queues from replaying stale
/// notifications. The low bit means enabled. Already-submitted OS notifications
/// cannot be recalled; disabling can race with the final native submission.
struct DeliveryPermit {
    policy: Arc<AtomicU64>,
    generation: u64,
}

impl DeliveryPermit {
    fn is_current(&self) -> bool {
        self.generation & 1 == 1 && self.policy.load(Ordering::Acquire) == self.generation
    }
}

struct PendingNotification {
    notification: Notification,
    permit: DeliveryPermit,
    result: async_channel::Sender<Result<DeliveryStatus, NotificationError>>,
}

/// Drop is nonblocking, invalidates queued work and never joins the worker.
/// Dropping an individual receipt does not cancel its notification.
pub struct DesktopNotifications {
    queue: SyncSender<PendingNotification>,
    policy: Arc<AtomicU64>,
    native_suppressed: bool,
}

impl DesktopNotifications {
    /// Starts a worker but does not ask for OS permission or send anything.
    /// Windows must supply the installer-registered AUMID. macOS must run from
    /// an app bundle; the first enabled delivery may request alert permission.
    pub fn native(options: NativeNotificationOptions) -> io::Result<Self> {
        let suppressed = cfg!(test)
            || native_delivery_disabled(
                std::env::var_os("HUSK_GPUI_DISABLE_DESKTOP_NOTIFICATIONS").as_deref(),
            );
        Self::with_backend(QUEUE_CAPACITY, suppressed, move |notification, permit| {
            send_native(notification, permit, &options)
        })
    }

    /// Inject the existing desktop notification preference; do not use the
    /// separate sound preference here. Native delivery stays off in tests/smoke.
    pub fn set_enabled(&self, enabled: bool) {
        let enabled = enabled && !self.native_suppressed;
        let _ = self
            .policy
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                Some(if (generation & 1 == 1) == enabled {
                    generation
                } else {
                    generation.wrapping_add(1)
                })
            });
    }

    #[cfg(test)]
    pub fn is_enabled(&self) -> bool {
        self.policy.load(Ordering::Acquire) & 1 == 1
    }

    /// Never waits for capacity, OS calls, callbacks, permission or user action.
    /// Queue overflow is explicit (no unbounded per-notification thread spawn).
    pub fn enqueue(&self, notification: Notification) -> Result<DeliveryReceipt, EnqueueError> {
        let generation = self.policy.load(Ordering::Acquire);
        if generation & 1 == 0 {
            return Err(EnqueueError::Disabled);
        }
        let (result, receipt) = async_channel::bounded(1);
        let pending = PendingNotification {
            notification,
            permit: DeliveryPermit {
                policy: self.policy.clone(),
                generation,
            },
            result,
        };
        self.queue.try_send(pending).map_err(|error| match error {
            TrySendError::Full(_) => EnqueueError::QueueFull,
            TrySendError::Disconnected(_) => EnqueueError::WorkerStopped,
        })?;
        Ok(receipt)
    }

    fn with_backend(
        capacity: usize,
        native_suppressed: bool,
        mut backend: impl FnMut(
            &Notification,
            &DeliveryPermit,
        ) -> Result<DeliveryStatus, NotificationError>
        + Send
        + 'static,
    ) -> io::Result<Self> {
        let (queue, pending) = mpsc::sync_channel::<PendingNotification>(capacity);
        let policy = Arc::new(AtomicU64::new(0));
        std::thread::Builder::new()
            .name("husk-desktop-notify".into())
            .spawn(move || {
                while let Ok(pending) = pending.recv() {
                    let outcome = if pending.permit.is_current() {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            backend(&pending.notification, &pending.permit)
                        }))
                        .unwrap_or(Err(NotificationError::BackendPanicked))
                    } else {
                        Ok(DeliveryStatus::Suppressed)
                    };
                    let _ = pending.result.try_send(outcome);
                }
            })?;
        Ok(Self {
            queue,
            policy,
            native_suppressed,
        })
    }
}

impl Drop for DesktopNotifications {
    fn drop(&mut self) {
        self.set_enabled(false);
    }
}

fn native_delivery_disabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

#[cfg(any(test, target_os = "macos"))]
fn require_authorization(granted: bool) -> Result<(), NotificationError> {
    if granted {
        Ok(())
    } else {
        Err(NotificationError::PermissionDenied)
    }
}

// Defense in depth: the OS implementation is not even compiled into unit tests.
#[cfg(test)]
fn send_native(
    _: &Notification,
    _: &DeliveryPermit,
    _: &NativeNotificationOptions,
) -> Result<DeliveryStatus, NotificationError> {
    Err(NotificationError::Unavailable(
        "unit tests never contact the OS",
    ))
}

#[cfg(all(not(test), any(target_os = "linux", target_os = "windows")))]
fn send_native(
    notification: &Notification,
    permit: &DeliveryPermit,
    options: &NativeNotificationOptions,
) -> Result<DeliveryStatus, NotificationError> {
    let mut native = notify_rust::Notification::new();
    native.summary(notification.title());
    #[cfg(target_os = "linux")]
    {
        let _ = options;
        // The freedesktop body supports markup, unlike titles/Windows/macOS.
        // Escape it rather than letting a caller inject markup/links.
        if let Some(body) = notification.body() {
            native.body(&escape_body_markup(body));
        }
        native
            .appname("Husk")
            .icon("husk")
            .hint(notify_rust::Hint::SuppressSound(true));
    }
    #[cfg(target_os = "windows")]
    {
        let app_id = options
            .windows_app_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or(NotificationError::Unavailable(
                "Windows requires a registered AppUserModelID",
            ))?;
        native.app_id(app_id);
        if let Some(body) = notification.body() {
            native.body(body);
        }
        // notify-rust's default None sound produces a silent Windows toast.
    }
    if !permit.is_current() {
        return Ok(DeliveryStatus::Suppressed);
    }
    native
        .show()
        .map(|_| DeliveryStatus::Submitted)
        .map_err(|error| NotificationError::Backend(error.to_string()))
}

#[cfg(any(test, target_os = "linux"))]
fn escape_body_markup(body: &str) -> String {
    body.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(all(not(test), target_os = "macos"))]
fn send_native(
    notification: &Notification,
    permit: &DeliveryPermit,
    _: &NativeNotificationOptions,
) -> Result<DeliveryStatus, NotificationError> {
    // UserNotifications must not be initialized by an unbundled CLI binary
    // (Apple can raise an Objective-C exception instead of returning NSError).
    objc2::rc::autoreleasepool(|_| macos::send(notification, permit))
}

#[cfg(all(not(test), target_os = "macos"))]
mod macos {
    use super::*;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString, NSUUID};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNUserNotificationCenter,
    };
    use std::time::Duration;

    pub(super) fn send(
        notification: &Notification,
        permit: &DeliveryPermit,
    ) -> Result<DeliveryStatus, NotificationError> {
        let bundle = NSBundle::mainBundle();
        if bundle.bundleIdentifier().is_none() || !bundle.bundlePath().to_string().ends_with(".app")
        {
            return Err(NotificationError::Unavailable(
                "macOS requires an application bundle",
            ));
        }
        if !permit.is_current() {
            return Ok(DeliveryStatus::Suppressed);
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (done, result) = mpsc::channel();
        let authorized = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let outcome =
                native_error(error).and_then(|()| require_authorization(granted.as_bool()));
            let _ = done.send(outcome);
        });
        // Do not request sound: the independent cue-tone policy is not ours.
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &authorized,
        );
        result
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| NotificationError::TimedOut)??;
        // In particular, do not send after a delayed permission dialog if the
        // user disabled notifications or the notifier was dropped meanwhile.
        if !permit.is_current() {
            return Ok(DeliveryStatus::Suppressed);
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(notification.title()));
        if let Some(body) = notification.body() {
            content.setBody(&NSString::from_str(body));
        }
        let identifier = NSUUID::UUID().UUIDString();
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None,
        );
        let (done, result) = mpsc::channel();
        let submitted = RcBlock::new(move |error: *mut NSError| {
            let _ = done.send(native_error(error));
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&submitted));
        result
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| NotificationError::TimedOut)??;
        Ok(DeliveryStatus::Submitted)
    }

    fn native_error(error: *mut NSError) -> Result<(), NotificationError> {
        // SAFETY: UserNotifications supplies either null or a valid NSError
        // for the duration of its callback; copy the description before return.
        match unsafe { error.as_ref() } {
            Some(error) => Err(NotificationError::Backend(
                error.localizedDescription().to_string(),
            )),
            None => Ok(()),
        }
    }
}

#[cfg(all(
    not(test),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
fn send_native(
    _: &Notification,
    _: &DeliveryPermit,
    _: &NativeNotificationOptions,
) -> Result<DeliveryStatus, NotificationError> {
    Err(NotificationError::Unavailable("unsupported platform"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Mutex, time::Duration};

    fn receive(receipt: &DeliveryReceipt) -> Result<DeliveryStatus, NotificationError> {
        // A broken worker must fail the test, not hang the suite indefinitely.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match receipt.try_recv() {
                Ok(outcome) => return outcome,
                Err(async_channel::TryRecvError::Empty) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "worker did not finish"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("worker dropped receipt: {error}"),
            }
        }
    }

    #[test]
    fn public_copy_is_opt_in_and_bounded_without_splitting_unicode() {
        let notification = Notification::default();
        assert_eq!(notification.title(), "Husk");
        assert_eq!(notification.body(), None);
        let notification = Notification::new("界".repeat(200))
            .with_safe_body(format!("\0\u{1b}{}", "文".repeat(600)));
        assert_eq!(notification.title().chars().count(), TITLE_LIMIT);
        assert_eq!(notification.body().unwrap().chars().count(), BODY_LIMIT);
        assert_eq!(Notification::new("\0\n").title(), "Husk");
        assert_eq!(Notification::default().with_safe_body("\t ").body(), None);
    }

    #[test]
    fn freedesktop_body_is_literal_not_markup() {
        assert_eq!(
            escape_body_markup("<b>&\"'</b>"),
            "&lt;b&gt;&amp;&quot;&apos;&lt;/b&gt;"
        );
    }

    #[test]
    fn missing_macos_authorization_is_an_error_not_submission_success() {
        assert_eq!(
            require_authorization(false),
            Err(NotificationError::PermissionDenied)
        );
        assert_eq!(require_authorization(true), Ok(()));
    }

    #[test]
    fn explicit_preference_controls_delivery_and_safe_body_is_forwarded() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let notifier = DesktopNotifications::with_backend(2, false, move |notification, _| {
            captured.lock().unwrap().push(notification.clone());
            Ok(DeliveryStatus::Submitted)
        })
        .unwrap();
        assert!(!notifier.is_enabled());
        assert_eq!(
            notifier.enqueue(Notification::default()).unwrap_err(),
            EnqueueError::Disabled
        );
        notifier.set_enabled(true);
        let public =
            Notification::new("Husk needs attention").with_safe_body("Open Husk to continue.");
        let receipt = notifier.enqueue(public.clone()).unwrap();
        assert_eq!(receive(&receipt), Ok(DeliveryStatus::Submitted));
        assert_eq!(*seen.lock().unwrap(), vec![public]);
        notifier.set_enabled(false);
        assert!(!notifier.is_enabled());
        assert_eq!(
            notifier.enqueue(Notification::default()).unwrap_err(),
            EnqueueError::Disabled
        );
    }

    #[test]
    fn backend_errors_are_receipted_and_do_not_stop_later_deliveries() {
        let mut fail = true;
        let notifier = DesktopNotifications::with_backend(2, false, move |_, _| {
            if std::mem::take(&mut fail) {
                Err(NotificationError::Backend(
                    "fake backend unavailable".into(),
                ))
            } else {
                Ok(DeliveryStatus::Submitted)
            }
        })
        .unwrap();
        notifier.set_enabled(true);
        assert_eq!(
            receive(&notifier.enqueue(Notification::default()).unwrap()),
            Err(NotificationError::Backend(
                "fake backend unavailable".into()
            ))
        );
        assert_eq!(
            receive(&notifier.enqueue(Notification::default()).unwrap()),
            Ok(DeliveryStatus::Submitted)
        );
    }

    #[test]
    fn backend_panics_are_receipted_without_killing_worker() {
        let mut panic_once = true;
        let notifier = DesktopNotifications::with_backend(2, false, move |_, _| {
            assert!(!std::mem::take(&mut panic_once), "fake backend panic");
            Ok(DeliveryStatus::Submitted)
        })
        .unwrap();
        notifier.set_enabled(true);
        assert_eq!(
            receive(&notifier.enqueue(Notification::default()).unwrap()),
            Err(NotificationError::BackendPanicked)
        );
        assert_eq!(
            receive(&notifier.enqueue(Notification::default()).unwrap()),
            Ok(DeliveryStatus::Submitted)
        );
    }

    #[test]
    fn delayed_permission_can_recheck_before_native_submission() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let notifier = DesktopNotifications::with_backend(1, false, move |_, permit| {
            let _ = entered.send(());
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(if permit.is_current() {
                DeliveryStatus::Submitted
            } else {
                DeliveryStatus::Suppressed
            })
        })
        .unwrap();
        notifier.set_enabled(true);
        let receipt = notifier.enqueue(Notification::default()).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        notifier.set_enabled(false);
        release.send(()).unwrap();
        assert_eq!(receive(&receipt), Ok(DeliveryStatus::Suppressed));
    }

    #[test]
    fn queue_is_nonblocking_and_preference_generation_discards_stale_work() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let notifier = DesktopNotifications::with_backend(1, false, move |_, _| {
            let _ = entered.send(());
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(DeliveryStatus::Submitted)
        })
        .unwrap();
        notifier.set_enabled(true);
        let in_flight = notifier.enqueue(Notification::default()).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = notifier.enqueue(Notification::default()).unwrap();
        assert_eq!(
            notifier.enqueue(Notification::default()).unwrap_err(),
            EnqueueError::QueueFull
        );
        notifier.set_enabled(false);
        notifier.set_enabled(true);
        release.send(()).unwrap();
        assert_eq!(receive(&in_flight), Ok(DeliveryStatus::Submitted));
        assert_eq!(receive(&queued), Ok(DeliveryStatus::Suppressed));
        assert!(
            started.try_recv().is_err(),
            "stale work reached the backend"
        );
    }

    #[test]
    fn dropping_notifier_does_not_join_blocked_backend_and_suppresses_queue() {
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let notifier = DesktopNotifications::with_backend(1, false, move |_, _| {
            let _ = entered.send(());
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(DeliveryStatus::Submitted)
        })
        .unwrap();
        notifier.set_enabled(true);
        let in_flight = notifier.enqueue(Notification::default()).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued = notifier.enqueue(Notification::default()).unwrap();
        drop(notifier);
        release.send(()).unwrap();
        assert_eq!(receive(&in_flight), Ok(DeliveryStatus::Submitted));
        assert_eq!(receive(&queued), Ok(DeliveryStatus::Suppressed));
    }

    #[test]
    fn native_delivery_cannot_be_enabled_by_unit_tests() {
        let notifier = DesktopNotifications::native(NativeNotificationOptions::default()).unwrap();
        notifier.set_enabled(true);
        assert!(!notifier.is_enabled());
        assert_eq!(
            notifier.enqueue(Notification::default()).unwrap_err(),
            EnqueueError::Disabled
        );
    }

    #[test]
    fn smoke_suppression_cannot_be_overridden_by_preferences() {
        let notifier = DesktopNotifications::with_backend(1, true, |_, _| {
            panic!("suppressed native backend was called")
        })
        .unwrap();
        notifier.set_enabled(true);
        assert_eq!(
            notifier.enqueue(Notification::default()).unwrap_err(),
            EnqueueError::Disabled
        );
    }

    #[test]
    fn explicit_kill_switch_is_parsed_without_mutating_process_environment() {
        assert!(native_delivery_disabled(Some(OsStr::new("1"))));
        for value in [None, Some(""), Some("0"), Some("true")] {
            assert!(!native_delivery_disabled(value.map(OsStr::new)));
        }
    }
}
