use crate::Wake;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver},
};

/// Unread count on the app icon (macOS Dock). Must run on the main thread; elsewhere it is a no-op.
pub(crate) fn set_badge(count: usize) {
    #[cfg(target_os = "macos")]
    if let Some(main) = objc2_foundation::MainThreadMarker::new() {
        let label = (count > 0).then(|| objc2_foundation::NSString::from_str(&count.to_string()));
        objc2_app_kit::NSApplication::sharedApplication(main)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// macOS shows neither alerts nor the Dock badge until the user allows them; ask once at launch so
/// the first message is not lost to the prompt. Badge is requested too (notify-rust asks only for
/// alerts and sounds). Needs the `.app` bundle: unbundled `just dev` builds cannot ask.
#[cfg(target_os = "macos")]
fn request_permission() {
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
    if notify_rust::check_bundle().is_err() {
        return;
    }
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge,
            &block2::RcBlock::new(|allowed: objc2::runtime::Bool, _| {
                if crate::teams::tracing() {
                    eprintln!(
                        "TeamsFast notifications: {}",
                        if allowed.as_bool() {
                            "allowed"
                        } else {
                            "not allowed; enable TeamsFast in System Settings → Notifications"
                        }
                    );
                }
            }),
        );
}

pub(crate) enum NoticeEvent {
    Open { epoch: u64, chat_id: String },
    Error(String),
}

pub(crate) struct Notifications {
    pub events: Receiver<NoticeEvent>,
    sender: mpsc::Sender<NoticeEvent>,
    active: Arc<AtomicUsize>,
    wake: Wake,
}

impl Notifications {
    pub fn new(wake: Wake) -> Self {
        let (sender, events) = mpsc::channel();
        #[cfg(target_os = "macos")]
        request_permission();
        Self {
            events,
            sender,
            active: Arc::new(AtomicUsize::new(0)),
            wake,
        }
    }
    pub fn show(&self, epoch: u64, chat_id: String, title: String, body: String) {
        // Bound response observers: macOS can retain notifications for a long time.
        if self.active.fetch_add(1, Ordering::Relaxed) >= 8 {
            self.active.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        let active = Arc::clone(&self.active);
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let mut notice = notify_rust::Notification::new();
            notice
                .summary(&title)
                .body(&body)
                .appname("TeamsFast")
                .timeout(60_000)
                .action("default", "Open chat");
            match notice.show() {
                Ok(handle) => {
                    handle.wait_for_action(|action| {
                        if action == "default" {
                            let _ = sender.send(NoticeEvent::Open { epoch, chat_id });
                            let _ = wake.try_send(());
                        }
                    });
                }
                Err(_) => {
                    let _=sender.send(NoticeEvent::Error("Notifications are unavailable. On macOS, open the bundled app and allow notifications in System Settings.".into()));
                    let _ = wake.try_send(());
                }
            }
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }
}
