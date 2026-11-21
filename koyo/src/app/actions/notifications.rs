use gpui_kit::ElementId;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;

use crate::app::{KoyoApp, NotificationPreferences};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToastCategory {
    Connections,
    Queries,
    Data,
    Exports,
}

struct ConnectionToast;
struct QueryToast;
struct DataToast;
struct ExportToast;

impl NotificationPreferences {
    pub(crate) fn allows(&self, category: ToastCategory) -> bool {
        self.enabled
            && match category {
                ToastCategory::Connections => self.connections,
                ToastCategory::Queries => self.queries,
                ToastCategory::Data => self.data,
                ToastCategory::Exports => self.exports,
            }
    }
}

impl KoyoApp {
    /// One gate for every in-app notification so the Settings preferences and
    /// the replacement policy live in one place.
    pub(crate) fn push_toast(
        &self,
        category: ToastCategory,
        key: impl Into<ElementId>,
        notification: Notification,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.notifications.allows(category) {
            return false;
        }
        let notification = match category {
            ToastCategory::Connections => notification.id1::<ConnectionToast>(key),
            ToastCategory::Queries => notification.id1::<QueryToast>(key),
            ToastCategory::Data => notification.id1::<DataToast>(key),
            ToastCategory::Exports => notification.id1::<ExportToast>(key),
        };
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = cx.update_window(window_handle, |_, window, cx| {
                window.push_notification(notification, cx);
            });
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::ToastCategory;
    use crate::app::NotificationPreferences;

    #[test]
    fn gate_respects_master_and_category_defaults() {
        let prefs = NotificationPreferences::default();
        assert!(prefs.allows(ToastCategory::Connections));
        assert!(!prefs.allows(ToastCategory::Queries));
        assert!(prefs.allows(ToastCategory::Data));
        assert!(prefs.allows(ToastCategory::Exports));

        let mut off = prefs.clone();
        off.enabled = false;
        assert!(!off.allows(ToastCategory::Connections));
        assert!(!off.allows(ToastCategory::Exports));

        let mut on = prefs;
        on.queries = true;
        assert!(on.allows(ToastCategory::Queries));
    }
}
