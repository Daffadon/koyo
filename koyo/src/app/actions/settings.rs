use gpui_kit::Window;
use gpui_kit::prelude::*;

use crate::app::KoyoApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotificationPreference {
    Enabled,
    Connections,
    Queries,
    Data,
    Exports,
}

impl KoyoApp {
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open
            || self.dialog.is_some()
            || self.confirm.is_some()
            || self.export.is_some()
        {
            return;
        }
        self.settings_open = true;
        self.settings_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            self.root_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn set_notification_preference(
        &mut self,
        preference: NotificationPreference,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        match preference {
            NotificationPreference::Enabled => self.notifications.enabled = value,
            NotificationPreference::Connections => self.notifications.connections = value,
            NotificationPreference::Queries => self.notifications.queries = value,
            NotificationPreference::Data => self.notifications.data = value,
            NotificationPreference::Exports => self.notifications.exports = value,
        }
        self.persist_state(cx);
        cx.notify();
    }
}
