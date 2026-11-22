use gpui_kit::Window;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;

use super::notifications::ToastCategory;
use crate::app::{
    KoyoApp, MAX_IDLE_POOL_SIZE, MAX_IDLE_POOL_TTL_SECS, MIN_IDLE_POOL_TTL_SECS, SettingsSection,
};

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

    pub(crate) fn set_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<Self>,
    ) {
        if self.settings_section != section {
            self.settings_section = section;
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

    pub(crate) fn set_idle_pool_size(&mut self, size: usize, cx: &mut Context<Self>) {
        self.idle_pool_size = size.min(MAX_IDLE_POOL_SIZE);
        self.trim_idle_pool(cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn set_idle_pool_ttl_secs(&mut self, seconds: u64, cx: &mut Context<Self>) {
        self.idle_pool_ttl_secs = seconds.clamp(MIN_IDLE_POOL_TTL_SECS, MAX_IDLE_POOL_TTL_SECS);
        self.trim_idle_pool(cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn disconnect_idle_connections(&mut self, cx: &mut Context<Self>) {
        let idle = std::mem::take(&mut self.parked_connections);
        let count = idle.len();
        for parked in idle {
            self.disconnect_session(parked.connection.session_id);
        }
        if count > 0 {
            self.status = format!("Closed {count} idle connections.");
            self.push_toast(
                ToastCategory::Connections,
                "idle-pool",
                Notification::info(format!(
                    "Closed {count} idle {}.",
                    if count == 1 {
                        "connection"
                    } else {
                        "connections"
                    }
                ))
                .title("Idle connections closed"),
                cx,
            );
        }
        cx.notify();
    }

    fn trim_idle_pool(&mut self, cx: &mut Context<Self>) {
        while self.parked_connections.len() > self.idle_pool_size {
            let evicted = self.parked_connections.remove(0);
            self.disconnect_session(evicted.connection.session_id);
        }
        let ttl = std::time::Duration::from_secs(self.idle_pool_ttl_secs);
        let mut index = 0;
        while index < self.parked_connections.len() {
            if self.parked_connections[index].parked_at.elapsed() <= ttl {
                index += 1;
                continue;
            }
            let expired = self.parked_connections.remove(index);
            self.disconnect_session(expired.connection.session_id);
        }
        cx.notify();
    }
}
