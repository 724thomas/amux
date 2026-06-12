//! Notification policy and desktop dispatch.
//!
//! Policy: a notification for the pane the user is currently looking at
//! (active pane of the active workspace, window focused) is suppressed —
//! they can already see it. Everything else gets a desktop notification
//! via DBus (org.freedesktop.Notifications), which behaves identically on
//! Wayland and X11.

use cmux_protocol::NotifyKind;
use notify_rust::{Notification, Urgency};

pub fn send_desktop(kind: NotifyKind, title: &str, body: &str) {
    let urgency = match kind {
        NotifyKind::Attention => Urgency::Critical,
        NotifyKind::Done | NotifyKind::Progress => Urgency::Normal,
        NotifyKind::Bell | NotifyKind::Idle => Urgency::Low,
    };
    let mut notification = Notification::new();
    notification
        .appname("cmux")
        .summary(title)
        .body(body)
        .urgency(urgency);
    // DBus can block; never stall the PTY read thread on it.
    let _ = std::thread::Builder::new().name("notify-dispatch".into()).spawn(move || {
        if let Err(e) = notification.show() {
            tracing::warn!("desktop notification failed: {e}");
        }
    });
}

pub fn default_title(kind: NotifyKind) -> &'static str {
    match kind {
        NotifyKind::Attention => "에이전트가 입력을 기다립니다",
        NotifyKind::Done => "작업 완료",
        NotifyKind::Progress => "진행 중",
        NotifyKind::Bell => "터미널 벨",
        NotifyKind::Idle => "대기 중",
    }
}
