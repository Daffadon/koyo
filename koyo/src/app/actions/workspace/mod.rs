use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{
    DeleteToPreviousWordStart, Enter, IndentInline, InputState, TextareaState,
};
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::database;
use koyo_core::models::{ConnectionInfo, ConnectionProfile, Engine};
use koyo_core::{ProfileStore, SessionManager};
use tokio::runtime::Runtime;
use uuid::Uuid;

use super::ai::restore_ai_threads;
use super::notifications::ToastCategory;
use crate::app::completion::{CompletionCatalog, SqlCompletionProvider};
use crate::app::*;

mod columns;
mod editor;
mod persistence;
mod sessions;
mod tabs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisconnectReason {
    Manual,
    Lost,
    SessionClosed,
}

pub(crate) fn same_connection_target(a: &ConnectionProfile, b: &ConnectionProfile) -> bool {
    a.engine == b.engine
        && a.host == b.host
        && a.port == b.port
        && a.database == b.database
        && a.username == b.username
        && a.ssl_mode == b.ssl_mode
        && a.sqlite_path == b.sqlite_path
}

impl KoyoApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, runtime: Arc<Runtime>) -> Self {
        let store = ProfileStore::from_default_dir();
        let settings = SettingsStore::from_default_dir();
        let (saved, state_warning, mut persistence_error) = match settings.load() {
            Ok(loaded) => (loaded.state, loaded.warning, None),
            Err(error) => {
                log::error!("could not load Koyo state: {error}");
                (PersistedState::default(), Some(error.clone()), Some(error))
            }
        };
        let (profiles, mut status) = match store.load_profiles() {
            Ok(profiles) => {
                let status = if profiles.is_empty() {
                    "Add a connection to get started.".to_string()
                } else {
                    "Choose a saved connection.".to_string()
                };
                (profiles, status)
            }
            Err(error) => {
                log::error!("could not load saved connection profiles: {error}");
                (Vec::new(), error)
            }
        };
        if let Some(warning) = store.take_migration_warning() {
            log::warn!("{warning}");
            status = warning;
        }
        if let Some(warning) = state_warning {
            log::warn!("{warning}");
            status = warning;
        }
        let ai_config_json = saved.ai_config_json.clone();
        let theme_choice = ThemeChoice::from_persisted(&saved.theme);
        let ai_config_error = AiConfig::parse(&ai_config_json).err().unwrap_or_default();
        let ai_model = AiConfig::parse(&ai_config_json)
            .map(|config| config.model)
            .unwrap_or_else(|_| "Not configured".into());
        let (ai_threads, ai_active_thread, next_ai_thread_id) = restore_ai_threads(&saved);
        let save_queue = Arc::new(StateSaveQueue::default());
        let (save_error_tx, mut save_error_rx) = tokio::sync::mpsc::unbounded_channel();
        if let Err(error) =
            spawn_state_save_worker(save_queue.clone(), settings.clone(), save_error_tx)
        {
            log::error!("{error}");
            status = error.clone();
            persistence_error = Some(error);
        }
        let ai_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask about your database...")
                .submit_on_enter(true)
                .auto_grow(1, 6)
        });
        let ai_config_seed = ai_config_json.clone();
        let ai_config_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(ai_config_seed)
                .auto_grow(6, 20)
        });
        let explorer_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search tables"));
        let profile_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search saved connections"));
        let history_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search history"));
        let mut app = Self {
            store,
            save_queue,
            history: saved.history.clone(),
            history_filter,
            history_collapsed: saved.history_collapsed,
            history_scroll: ScrollHandle::new(),
            allow_close: false,
            editor_persist_task: None,
            inactive_since: None,
            catalog_generations: HashMap::new(),
            session_transactions: HashMap::new(),
            persistence_disabled: persistence_error.is_some(),
            manager: Arc::new(SessionManager::default()),
            runtime,
            profiles,
            parked_connections: Vec::new(),
            tabs: Vec::new(),
            primary_tabs: Vec::new(),
            secondary_tabs: Vec::new(),
            primary_active: 0,
            secondary_active: 0,
            primary_tab_scroll: ScrollHandle::new(),
            secondary_tab_scroll: ScrollHandle::new(),
            explorer_filter,
            profile_filter,
            split: None,
            split_ratio: saved.split_ratio.clamp(0.15, 0.85),
            focus: Pane::Primary,
            root_focus: cx.focus_handle(),
            window_handle: window.window_handle(),
            sidebar_open: saved.sidebar_open,
            sidebar_width: saved
                .sidebar_width
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
            sidebar_resizing: false,
            sidebar_tab: None,
            ai_open: saved.ai_open,
            ai_width: saved.ai_width.clamp(MIN_AI_WIDTH, MAX_AI_WIDTH),
            ai_resizing: false,
            ai_settings_open: false,
            settings_open: false,
            settings_focus: cx.focus_handle(),
            theme_choice,
            settings_section: SettingsSection::default(),
            notifications: saved.notifications.clone(),
            idle_pool_size: saved.idle_pool_size.min(MAX_IDLE_POOL_SIZE),
            idle_pool_ttl_secs: saved
                .idle_pool_ttl_secs
                .clamp(MIN_IDLE_POOL_TTL_SECS, MAX_IDLE_POOL_TTL_SECS),
            page_limit: saved.page_limit.clamp(MIN_PAGE_LIMIT, MAX_PAGE_LIMIT),
            query_timeout_secs: saved
                .query_timeout_secs
                .clamp(MIN_QUERY_TIMEOUT_SECS, MAX_QUERY_TIMEOUT_SECS),
            history_limit: saved
                .history_limit
                .clamp(MIN_HISTORY_LIMIT, MAX_HISTORY_LIMIT),
            ai_config_json,
            ai_model,
            ai_config_error,
            ai_threads,
            ai_active_thread,
            next_ai_thread_id,
            ai_task: None,
            ai_generation: 0,
            ai_compacting: false,
            ai_input,
            ai_config_input,
            ai_pending: false,
            ai_error: String::new(),
            ai_scroll: ScrollHandle::new(),
            status,
            dialog: None,
            confirm_focus: cx.focus_handle(),
            cell_edit: None,
            confirm: None,
            export: None,
            quick_open: None,
            next_tab_id: 1,
        };
        app.history.truncate(app.history_limit);
        cx.spawn(async move |this, cx| {
            while let Some(error) = save_error_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.status = error;
                    cx.notify();
                });
            }
        })
        .detach();
        apply_theme(theme_choice, cx);
        if let Some(error) = persistence_error {
            app.push_toast(
                ToastCategory::Connections,
                "persistence-disabled",
                Notification::warning(error)
                    .title("Settings will not be saved")
                    .autohide(false),
                cx,
            );
        }
        app.restore_workspace(&saved.workspace, window, cx);
        app.ensure_workspace_tab(window, cx);
        app.focus_active_query_editor(window, cx);

        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let mut allow = true;
            let _ = weak.update(cx, |app, cx| {
                if app.allow_close {
                    return;
                }
                app.persist_state_sync(cx);
                if app.has_unsaved_work(cx) {
                    allow = false;
                    app.confirm_close(window, cx);
                }
            });
            allow
        });
        cx.observe(&app.explorer_filter, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&app.profile_filter, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&app.history_filter, |_, _, cx| cx.notify())
            .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            this.handle_window_activation(window.is_window_active(), cx);
        })
        .detach();
        app
    }

    pub(crate) fn workspace_shortcuts_blocked(&self) -> bool {
        self.dialog.is_some()
            || self.settings_open
            || self.cell_edit.is_some()
            || self.confirm.is_some()
            || self.export.is_some()
            || self.quick_open.is_some()
            || self.ai_settings_open
    }

    pub(crate) fn ensure_workspace_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.primary_tabs.is_empty() {
            return;
        }
        let parked = loop {
            let Some(parked) = self.parked_connections.pop() else {
                break None;
            };
            if parked.parked_at.elapsed() > std::time::Duration::from_secs(self.idle_pool_ttl_secs)
            {
                self.disconnect_session(parked.connection.session_id);
                continue;
            }
            break Some(parked);
        };
        let has_connection = parked.is_some();
        let mut tab = self.make_query_tab(
            parked.as_ref().map(|parked| parked.profile.clone()),
            window,
            cx,
        );
        if let Some(parked) = parked {
            tab.connection = Some(parked.connection);
            tab.tables = parked.tables;
            tab.schema_objects = parked.schema_objects;
            tab.metadata_groups = parked.metadata_groups;
            tab.columns = parked.columns;
            tab.connecting = false;
        }
        let id = tab.id;
        self.tabs.push(tab);
        self.primary_tabs.push(id);
        self.primary_active = id;
        self.sidebar_tab = Some(id);
        if has_connection {
            self.load_tables(id, cx);
        }
    }

    pub(crate) fn set_split_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        if self.split.is_some() {
            let ratio = ratio.clamp(0.15, 0.85);
            if (self.split_ratio - ratio).abs() > 0.001 {
                self.split_ratio = ratio;
                cx.notify();
            }
        }
    }

    pub(crate) fn set_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        if (self.sidebar_width - width).abs() > 0.5 {
            self.sidebar_width = width;
            self.sidebar_resizing = true;
            cx.notify();
        }
    }

    pub(crate) fn set_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        if self.theme_choice == choice {
            return;
        }
        self.theme_choice = choice;
        apply_theme(choice, cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.history_collapsed = !self.history_collapsed;
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        title: impl Into<String>,
        description: impl Into<String>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) {
        self.confirm = Some(ConfirmState {
            title: title.into(),
            description: description.into(),
            action: std::rc::Rc::new(action),
            close_window: false,
            return_focus: window.focused(cx),
        });
        self.confirm_focus.focus(window, cx);
        cx.notify();
    }

    /// Confirmation raised from an action that has no `Window` parameter.
    /// Focus arrives on the next effect cycle so Escape still dismisses it.
    pub(crate) fn confirm_deferred(
        &mut self,
        title: impl Into<String>,
        description: impl Into<String>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.confirm = Some(ConfirmState {
            title: title.into(),
            description: description.into(),
            action: std::rc::Rc::new(action),
            close_window: false,
            return_focus: self
                .focused_query_editor()
                .map(|editor| editor.read(cx).focus_handle(cx).clone()),
        });
        let window_handle = self.window_handle;
        let focus = self.confirm_focus.clone();
        cx.defer(move |cx| {
            let _ = cx.update_window(window_handle, |_, window, cx| {
                focus.focus(window, cx);
            });
        });
        cx.notify();
    }

    pub(crate) fn resolve_confirm(
        &mut self,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.confirm.take() else {
            return;
        };
        if pending.close_window {
            if confirmed {
                self.persist_state_sync(cx);
                self.allow_close = true;
                window.remove_window();
            } else {
                cx.notify();
            }
            return;
        }
        if confirmed {
            (pending.action)(self, cx);
        }
        if let Some(focus) = pending.return_focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }
}

pub(super) fn move_tab_id(tabs: &mut Vec<u64>, tab_id: u64, direction: isize) -> bool {
    let Some(from) = tabs.iter().position(|id| *id == tab_id) else {
        return false;
    };
    let Some(to) = from.checked_add_signed(direction) else {
        return false;
    };
    if to >= tabs.len() {
        return false;
    }
    let id = tabs.remove(from);
    tabs.insert(to, id);
    true
}

pub(super) fn previous_horizontal_whitespace_start(value: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(value.len());
    let before = &value[..cursor];
    let start = before.trim_end_matches([' ', '\t']).len();
    (start < cursor).then_some(start)
}

pub(super) fn neighbor_after_close(remaining: &[u64], closed_position: usize) -> u64 {
    remaining
        .get(closed_position)
        .copied()
        .or_else(|| {
            closed_position
                .checked_sub(1)
                .and_then(|index| remaining.get(index).copied())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        KoyoApp, move_tab_id, neighbor_after_close, previous_horizontal_whitespace_start,
        same_connection_target,
    };
    use koyo_core::models::{ConnectionProfile, Engine, SslMode};
    use uuid::Uuid;

    fn profile(name: &str, host: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::new_v4(),
            name: name.into(),
            engine: Engine::Postgres,
            host: host.into(),
            port: 5432,
            database: "app".into(),
            username: "koyo".into(),
            ssl_mode: SslMode::VerifyFull,
            sqlite_path: String::new(),
        }
    }

    #[test]
    fn pooled_reuse_requires_the_same_connection_target() {
        let live = profile("Local", "db.internal");
        let renamed = profile("Renamed", "db.internal");
        let moved = profile("Local", "other.internal");
        let mut same_id = renamed.clone();
        same_id.id = live.id;

        assert!(same_connection_target(&live, &renamed));
        assert!(!same_connection_target(&live, &moved));
        assert!(same_connection_target(&live, &same_id));
    }

    #[test]
    fn shared_session_stays_in_use_until_the_last_tab_closes() {
        let shared = Uuid::new_v4();
        let other = Uuid::new_v4();
        let tabs = [(1, Some(shared)), (2, Some(shared)), (3, Some(other))];

        assert!(KoyoApp::session_is_used(tabs, shared, Some(1)));
        assert!(KoyoApp::session_is_used(tabs, shared, Some(2)));
        assert!(!KoyoApp::session_is_used(
            [(1, Some(shared))],
            shared,
            Some(1)
        ));
        assert!(!KoyoApp::session_is_used(tabs, Uuid::new_v4(), None));
    }

    #[test]
    fn closing_a_tab_selects_its_right_neighbor_then_left_neighbor() {
        assert_eq!(neighbor_after_close(&[10, 30], 1), 30);
        assert_eq!(neighbor_after_close(&[10, 20], 2), 20);
        assert_eq!(neighbor_after_close(&[], 0), 0);
    }

    #[test]
    fn moves_tabs_one_step_and_stops_at_edges() {
        let mut tabs = vec![1, 2, 3];
        assert!(move_tab_id(&mut tabs, 2, -1));
        assert_eq!(tabs, vec![2, 1, 3]);
        assert!(move_tab_id(&mut tabs, 2, 1));
        assert_eq!(tabs, vec![1, 2, 3]);
        assert!(!move_tab_id(&mut tabs, 1, -1));
        assert!(!move_tab_id(&mut tabs, 3, 1));
    }

    #[test]
    fn ctrl_backspace_removes_only_trailing_horizontal_whitespace() {
        assert_eq!(previous_horizontal_whitespace_start("select ", 7), Some(6));
        assert_eq!(previous_horizontal_whitespace_start("\t  x", 3), Some(0));
        assert_eq!(previous_horizontal_whitespace_start("select", 6), None);
        assert_eq!(previous_horizontal_whitespace_start("select\n", 7), None);
    }
}
