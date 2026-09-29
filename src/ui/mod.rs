pub mod browser_select;
pub mod console_view;
mod dialogs;
pub mod main_view;
pub mod scraper_log;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Window};

/// Short helper so views get localized strings without repeating the
/// `rust_i18n::t!(...).to_string()` dance.
pub(crate) fn t(key: &'static str) -> String {
    rust_i18n::t!(key).to_string()
}

/// Pushes a notification onto the given window (shared by all views).
pub(crate) fn push_notification(window: &mut Window, cx: &mut App, notification: Notification) {
    window.push_notification(notification, cx);
}
