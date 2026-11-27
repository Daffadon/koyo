use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::ProfileInput;
use koyo_core::database;
use koyo_core::models::{
    ColumnCatalogEntry, Engine, MetadataGroup, SchemaObject, SslMode, TableSummary,
};
use uuid::Uuid;

use super::notifications::ToastCategory;
use super::workspace::same_connection_target;
use crate::app::*;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn connect_with_timeout(
    manager: &koyo_core::SessionManager,
    profile: &koyo_core::models::ConnectionProfile,
    password: &str,
) -> Result<koyo_core::models::ConnectionInfo, String> {
    match tokio::time::timeout(
        CONNECTION_TIMEOUT,
        database::connect(manager, profile, password),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("Connection timed out after 30 seconds.".into()),
    }
}

/// Run a database operation in a task so the timeout can cancel it while it is
/// still active. Dropping a `timeout` future first makes cancellation a no-op.
pub(crate) async fn run_with_cancel<T>(
    timeout: Duration,
    timeout_message: &'static str,
    manager: &koyo_core::SessionManager,
    session_id: Uuid,
    operation_id: Uuid,
    future: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    let mut task = tokio::spawn(future);
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("Database task failed unexpectedly: {error}")),
        Err(_) => {
            let _ = database::cancel_query(manager, session_id, operation_id).await;
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut task).await;
            task.abort();
            Err(timeout_message.into())
        }
    }
}

impl ConnectionDialog {
    pub(crate) fn new(
        target_tab: Option<u64>,
        engine: Engine,
        window: &mut Window,
        cx: &mut Context<KoyoApp>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Local development"));
        let host = cx.new(|cx| InputState::new(window, cx).placeholder("localhost"));
        let port = cx.new(|cx| InputState::new(window, cx).placeholder("5432"));
        let database = cx.new(|cx| InputState::new(window, cx).placeholder("postgres"));
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("postgres"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let sqlite_path =
            cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/database.db"));
        let mut dialog = Self {
            engine,
            editing: None,
            target_tab,
            name,
            host,
            port,
            database,
            username,
            password,
            sqlite_path,
            ssl_disable: engine.is_sqlite(),
            busy: false,
            attempt_tab: None,
            error: String::new(),
        };
        apply_defaults(&mut dialog, engine, window, cx);
        dialog
    }
}

impl KoyoApp {
    pub(crate) fn load_tables(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let session_id = connection.session_id;
        let ids = self
            .tabs
            .iter()
            .filter(|tab| {
                tab.connection
                    .as_ref()
                    .is_some_and(|connection| connection.session_id == session_id)
            })
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        for tab_id in &ids {
            if let Some(index) = self.tab_index(*tab_id) {
                self.tabs[index].catalog_loading = true;
            }
        }
        let generation = {
            let generation = self.catalog_generations.entry(session_id).or_insert(0);
            *generation = generation.wrapping_add(1);
            *generation
        };
        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let tables: Vec<TableSummary> = database::list_tables(&manager, session_id).await?;
                let objects: Vec<SchemaObject> =
                    database::list_schema_objects(&manager, session_id).await?;
                let metadata: Vec<MetadataGroup> =
                    database::list_database_metadata(&manager, session_id).await?;
                let columns: Vec<ColumnCatalogEntry> =
                    database::list_column_catalog(&manager, session_id).await?;
                Ok::<_, String>((tables, objects, metadata, columns))
            },
            move |this, cx, result| {
                if this.catalog_generations.get(&session_id).copied() != Some(generation) {
                    return;
                }
                let ids = this
                    .tabs
                    .iter()
                    .filter(|tab| {
                        tab.connection
                            .as_ref()
                            .is_some_and(|connection| connection.session_id == session_id)
                    })
                    .map(|tab| tab.id)
                    .collect::<Vec<_>>();
                for tab_id in &ids {
                    if let Some(index) = this.tab_index(*tab_id) {
                        this.tabs[index].catalog_loading = false;
                    }
                }
                match result {
                    Ok((tables, objects, metadata, columns)) => {
                        let object_count = objects.len();
                        for same_session_id in ids {
                            let Some(index) = this.tab_index(same_session_id) else {
                                continue;
                            };
                            // Keep the explorer expansion state across refreshes.
                            this.tabs[index].tables = tables.clone();
                            this.tabs[index].schema_objects = objects.clone();
                            this.tabs[index].metadata_groups = metadata.clone();
                            this.tabs[index].columns = columns.clone();
                            if let TabBody::Query(query) = &mut this.tabs[index].body {
                                query.completion.borrow_mut().replace(
                                    &tables,
                                    &objects,
                                    &columns,
                                );
                            }
                        }
                        this.status = format!("{object_count} objects.");
                    }
                    Err(error) => {
                        log::error!("database object load failed on session {session_id}: {error}");
                        if database::is_connection_lost(&error) {
                            this.mark_connection_lost(
                                session_id,
                                "Connection lost while loading database objects. Reconnect to continue.",
                                cx,
                            );
                        } else {
                            for tab_id in ids {
                                if let Some(index) = this.tab_index(tab_id) {
                                    this.tabs[index].notice = error.clone();
                                }
                            }
                        }
                    }
                }
                // Table pages use try_lock and fail fast while the catalog
                // task owns the session. Start only the focused table after
                // this task releases it; every other tab loads on activation.
                if let Some(active) = this.pane_active_id(this.focus) {
                    this.ensure_table_page(active, cx);
                } else {
                    this.ensure_table_page(tab_id, cx);
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn refresh_tables(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        self.load_tables(tab_id, cx);
        cx.notify();
    }

    pub(crate) fn set_expanded(
        &mut self,
        tab_id: u64,
        schema: String,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if expanded {
            self.tabs[index].expanded.insert(schema);
        } else {
            self.tabs[index].expanded.remove(&schema);
        }
        cx.notify();
    }

    pub(crate) fn set_all_expanded(&mut self, tab_id: u64, expanded: bool, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if !expanded {
            self.tabs[index].expanded.clear();
            cx.notify();
            return;
        }

        let mut keys = BTreeSet::new();
        keys.insert("Schemas".to_string());
        for object in &self.tabs[index].schema_objects {
            keys.insert(object.schema.clone());
            keys.insert(format!("{}/{:?}", object.schema, object.kind));
        }
        for group in &self.tabs[index].metadata_groups {
            keys.insert(format!("group:{}", group.name));
        }
        self.tabs[index].expanded = keys;
        cx.notify();
    }

    pub(crate) fn connect_profile(
        &mut self,
        profile_id: Uuid,
        target: Option<u64>,
        from_dialog: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.busy = false;
                dialog.error = "Connection profile no longer exists.".into();
            }
            cx.notify();
            return;
        };

        // Live tabs are pinned to the connection they opened. Switching the
        // sidebar to another profile must not steal that tab: open a fresh SQL
        // tab for the new connection so both sessions stay live in one pane.
        // Empty/disconnected tabs may be retargeted in place.
        let target_id = target.and_then(|id| self.tab_index(id).map(|index| self.tabs[index].id));
        let retarget_live_tab = target_id.is_some_and(|id| {
            self.tab_index(id).is_some_and(|index| {
                self.tabs[index].connection.is_some()
                    && self.tabs[index]
                        .profile
                        .as_ref()
                        .is_some_and(|current| current.id != profile.id)
            })
        });
        let adopt_sidebar = from_dialog && (target.is_none() || retarget_live_tab);
        let tab_id = if retarget_live_tab {
            let tab = self.make_query_tab(Some(profile.clone()), window, cx);
            self.add_tab(tab, self.focus)
        } else {
            match target_id {
                Some(id) => id,
                None => {
                    let tab = self.make_query_tab(Some(profile.clone()), window, cx);
                    self.add_tab(tab, self.focus)
                }
            }
        };
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[index].connecting {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.busy = false;
                dialog.error = "This tab is already connecting. Wait for it to finish.".into();
            }
            cx.notify();
            return;
        }

        if self.tabs[index].connection.is_none()
            && let Some((connection, catalog)) = self.connection_for_profile(&profile)
        {
            let run_pending = matches!(
                &self.tabs[index].body,
                TabBody::Query(query) if query.pending_run
            );
            self.attach_connection_to_tab(tab_id, connection.clone(), catalog, cx);
            self.ensure_table_page(tab_id, cx);
            if adopt_sidebar {
                self.sidebar_tab = Some(tab_id);
            }
            if let TabBody::Query(query) = &mut self.tabs[index].body {
                query.pending_run = false;
            }
            let attached = self.attach_profile_session(profile.id, tab_id, cx);
            let message = if attached == 0 {
                format!("Connected to {}.", connection.profile.name)
            } else {
                format!(
                    "Connected to {}. Reconnected {} other {}.",
                    connection.profile.name,
                    attached,
                    if attached == 1 { "tab" } else { "tabs" }
                )
            };
            self.status = format!("Connected to {}.", connection.profile.name);
            self.push_toast(
                ToastCategory::Connections,
                profile.id.to_string(),
                Notification::success(message).title("Connected"),
                cx,
            );
            if run_pending {
                self.run_query(tab_id, cx);
            }
            if from_dialog {
                self.dialog = None;
            }
            self.persist_state(cx);
            cx.notify();
            return;
        }

        if self.profile_is_connecting(&profile) {
            self.tabs[index].connecting = false;
            if let TabBody::Query(query) = &mut self.tabs[index].body {
                query.pending_run = false;
            }
            self.tabs[index].notice =
                "Waiting for the connection to finish. Run the query again when connected.".into();
            self.status = "Waiting for the connection to finish.".into();
            if from_dialog && let Some(dialog) = self.dialog.as_mut() {
                dialog.busy = false;
                dialog.error =
                    "A connection for this profile is already starting. This tab will join it."
                        .into();
            }
            cx.notify();
            return;
        }

        let previous_profile = self.tabs[index].profile.clone();
        let attempt_id = Uuid::new_v4();
        self.tabs[index].connecting = true;
        self.tabs[index].connection_attempt = Some(attempt_id);
        self.tabs[index].notice = if self.tabs[index].connection.is_some() {
            "Switching connection...".to_string()
        } else {
            "Connecting...".to_string()
        };
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.busy = true;
            dialog.attempt_tab = Some(tab_id);
            dialog.error.clear();
        }
        self.push_toast(
            ToastCategory::Connections,
            profile.id.to_string(),
            Notification::info(format!("Connecting to {}.", profile.name)).title("Connecting"),
            cx,
        );
        cx.notify();

        let manager = self.manager.clone();
        let store = self.store.clone();
        let profile_for_task = profile.clone();
        let profile_name = profile.name.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let password = if profile_for_task.engine.needs_password() {
                    store.load_password(profile_for_task.id)?
                } else {
                    String::new()
                };
                connect_with_timeout(&manager, &profile_for_task, &password).await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    if let Ok(info) = &result {
                        this.disconnect_session(info.session_id);
                    }
                    return;
                };
                // A cancelled attempt may finish after the user has started a
                // new one. Never let its result or error touch the new state.
                if this.tabs[index].connection_attempt != Some(attempt_id) {
                    if let Ok(info) = &result {
                        this.disconnect_session(info.session_id);
                    }
                    return;
                }
                match result {
                    Ok(info) => {
                        log::info!(
                            "connected {} ({:?}) with session {}",
                            info.profile.name,
                            info.profile.engine,
                            info.session_id
                        );
                        if !this.tabs[index].connecting {
                            this.disconnect_session(info.session_id);
                            return;
                        }
                        let previous_connection = this.tabs[index].connection.replace(info.clone());
                        if let Some(previous) = previous_connection {
                            if previous.session_id != info.session_id {
                                this.disconnect_session_if_unused(
                                    previous.session_id,
                                    Some(tab_id),
                                );
                            }
                        }
                        let preserve_staged = this.tabs[index]
                            .profile
                            .as_ref()
                            .is_some_and(|profile| same_connection_target(profile, &info.profile));
                        let had_staged = this.tabs[index]
                            .body
                            .table()
                            .is_some_and(|table| !table.staged.is_empty());
                        this.tabs[index].profile = Some(info.profile.clone());
                        this.tabs[index].connecting = false;
                        this.tabs[index].connection_attempt = None;
                        this.tabs[index].notice.clear();
                        if matches!(this.tabs[index].body, TabBody::Query(_)) {
                            this.tabs[index].title = query_tab_title(Some(&info.profile));
                        }
                        let engine = info.profile.engine;
                        if this
                            .cell_edit
                            .as_ref()
                            .is_some_and(|edit| edit.tab_id == tab_id)
                        {
                            this.cell_edit = None;
                        }
                        if previous_profile.is_none() {
                            this.tabs[index].pending_starter = Some(starter_sql(engine).into());
                        }
                        this.status = format!("Connected to {}.", info.profile.name);
                        if adopt_sidebar {
                            this.sidebar_tab = Some(tab_id);
                        }
                        this.load_tables(tab_id, cx);
                        let mut run_pending = false;
                        match &mut this.tabs[index].body {
                            TabBody::Query(query) => {
                                query.completion.borrow_mut().clear();
                                query.output = None;
                                query.error.clear();
                                query.duration_ms = None;
                                query.active_result = 0;
                                query.running = false;
                                query.cancelling = false;
                                query.operation_id = None;
                                if query.pending_run {
                                    query.pending_run = false;
                                    run_pending = true;
                                }
                            }
                            TabBody::Table(table) => {
                                // Row identity and staged values belong to the
                                // previous session. Never let Apply use them
                                // against the newly connected server.
                                table.page = None;
                                table.total_rows_cache = None;
                                if !preserve_staged {
                                    table.staged.clear();
                                }
                                table.selected_keys.clear();
                                table.loading = false;
                                table.operation_id = None;
                                table.foreign_keys.clear();
                                table.foreign_keys_loading = false;
                                table.indexes.clear();
                                table.diagram_metadata_loaded = false;
                                // The catalog task owns the session right now;
                                // its callback starts this page.
                            }
                        }
                        if had_staged {
                            this.tabs[index].notice = if preserve_staged {
                                "Reconnected. Staged edits kept; review before applying.".into()
                            } else {
                                "Reconnected. Staged edits discarded because the connection target changed."
                                    .into()
                            };
                        }
                        // A restored tab may be visible while the user
                        // reconnects from another tab. Adopt the new session
                        // immediately in every pane instead of waiting for a
                        // click in each one.
                        let attached = this.attach_profile_session(info.profile.id, tab_id, cx);
                        let message = if attached == 0 {
                            format!("Connected to {}.", info.profile.name)
                        } else {
                            format!(
                                "Connected to {}. Reconnected {} other {}.",
                                info.profile.name,
                                attached,
                                if attached == 1 { "tab" } else { "tabs" }
                            )
                        };
                        this.push_toast(
                            ToastCategory::Connections,
                            info.profile.id.to_string(),
                            Notification::success(message).title("Connected"),
                            cx,
                        );
                        if run_pending {
                            this.run_query(tab_id, cx);
                        }
                        if from_dialog
                            && this
                                .dialog
                                .as_ref()
                                .is_some_and(|dialog| dialog.attempt_tab == Some(tab_id))
                        {
                            this.dialog = None;
                        }
                    }
                    Err(error) => {
                        log::error!("connection to profile {profile_id} failed: {error}");
                        let waiting = this
                            .tabs
                            .iter()
                            .filter(|tab| {
                                tab.connection.is_none()
                                    && tab
                                        .profile
                                        .as_ref()
                                        .is_some_and(|profile| profile.id == profile_id)
                            })
                            .filter(|tab| {
                                tab.connection_attempt
                                    .is_none_or(|attempt| attempt == attempt_id)
                            })
                            .map(|tab| tab.id)
                            .collect::<Vec<_>>();
                        for waiting_id in waiting {
                            let Some(waiting_index) = this.tab_index(waiting_id) else {
                                continue;
                            };
                            this.tabs[waiting_index].connecting = false;
                            this.tabs[waiting_index].connection_attempt = None;
                            this.tabs[waiting_index].notice = error.clone();
                            if let TabBody::Query(query) = &mut this.tabs[waiting_index].body {
                                query.pending_run = false;
                            }
                        }
                        if let Some(index) = this.tab_index(tab_id) {
                            this.tabs[index].connecting = false;
                            this.tabs[index].connection_attempt = None;
                            this.tabs[index].notice = error.clone();
                        }
                        if this
                            .dialog
                            .as_ref()
                            .is_some_and(|dialog| dialog.attempt_tab == Some(tab_id))
                            && let Some(dialog) = this.dialog.as_mut()
                        {
                            dialog.busy = false;
                            dialog.attempt_tab = None;
                            dialog.error = error.clone();
                        }
                        this.status = error.clone();
                        if !from_dialog {
                            this.push_toast(
                                ToastCategory::Connections,
                                profile_id.to_string(),
                                Notification::error(format!(
                                    "Could not connect to {profile_name}. Check the profile and try again."
                                ))
                                .title("Connection failed")
                                .autohide(false),
                                cx,
                            );
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn open_connection_dialog(
        &mut self,
        target_tab: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = target_tab
            .and_then(|id| self.tab_index(id))
            .and_then(|index| self.tabs[index].profile.as_ref())
            .map(|profile| profile.engine)
            .unwrap_or(Engine::Postgres);
        let dialog = ConnectionDialog::new(target_tab, engine, window, cx);
        let focus = dialog.name.read(cx).focus_handle(cx);
        self.dialog = Some(dialog);
        self.profile_filter.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn dismiss_notice(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[index].notice.is_empty() {
            return;
        }
        self.tabs[index].notice.clear();
        cx.notify();
    }

    pub(crate) fn close_connection_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        if dialog.busy {
            // Closing the dialog is also the only reliable way for the user to
            // abandon a handshake that is stuck in the driver. The task is
            // bounded by the connection timeout; its late result is ignored by
            // the attempt check in connect_profile.
            if let Some(tab_id) = dialog.attempt_tab.or(dialog.target_tab)
                && let Some(index) = self.tab_index(tab_id)
            {
                self.tabs[index].connecting = false;
                self.tabs[index].connection_attempt = None;
                self.tabs[index].notice = "Connection cancelled.".into();
                if let TabBody::Query(query) = &mut self.tabs[index].body {
                    query.pending_run = false;
                }
            }
            self.status = "Connection cancelled.".into();
        }
        cx.notify();
    }

    pub(crate) fn choose_engine(
        &mut self,
        engine: Engine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        if dialog.engine == engine {
            return;
        }
        dialog.engine = engine;
        dialog.error.clear();
        // Keep the edit target when the engine changes; saving updates the
        // profile and requires a new password for the new target.
        apply_defaults(dialog, engine, window, cx);
        cx.notify();
    }

    pub(crate) fn edit_profile_in_dialog(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        else {
            return;
        };
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        dialog.engine = profile.engine;
        dialog.editing = Some(profile.id);
        dialog.error.clear();
        set_input(&dialog.name, &profile.name, window, cx);
        set_input(&dialog.host, &profile.host, window, cx);
        set_input(&dialog.port, &profile.port.to_string(), window, cx);
        set_input(&dialog.database, &profile.database, window, cx);
        set_input(&dialog.username, &profile.username, window, cx);
        set_input(&dialog.sqlite_path, &profile.sqlite_path, window, cx);
        set_input(&dialog.password, "", window, cx);
        dialog.ssl_disable = profile.ssl_mode == SslMode::Disable;
        cx.notify();
    }

    pub(crate) fn browse_sqlite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_none() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SQLite database", &["db", "sqlite", "sqlite3"])
            .pick_file()
        else {
            return;
        };
        let path = path.display().to_string();
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        dialog.sqlite_path.update(cx, |input, cx| {
            input.set_value(path, window, cx);
        });
        cx.notify();
    }

    pub(crate) fn set_ssl_disable(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.ssl_disable = disabled;
        }
        cx.notify();
    }

    pub(crate) fn save_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        if dialog.busy {
            return;
        }
        let name = dialog.name.read(cx).value().to_string();
        let host = dialog.host.read(cx).value().to_string();
        let port_text = dialog.port.read(cx).value().to_string();
        let database = dialog.database.read(cx).value().to_string();
        let username = dialog.username.read(cx).value().to_string();
        let password = dialog.password.read(cx).value().to_string();
        let sqlite_path = dialog.sqlite_path.read(cx).value().to_string();
        let engine = dialog.engine;
        let editing = dialog.editing;
        let ssl_disable = dialog.ssl_disable;

        let port = if engine.is_sqlite() {
            0
        } else {
            match port_text.trim().parse::<u16>() {
                Ok(port) if port > 0 => port,
                _ => {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "Port must be between 1 and 65535.".into();
                    }
                    cx.notify();
                    return;
                }
            }
        };

        let input = ProfileInput {
            id: editing,
            name,
            engine,
            host,
            port,
            database,
            username,
            ssl_mode: if ssl_disable {
                SslMode::Disable
            } else {
                SslMode::VerifyFull
            },
            sqlite_path,
        };
        let target_changed = editing
            .and_then(|id| self.profiles.iter().find(|profile| profile.id == id))
            .is_some_and(|previous| {
                previous.engine != input.engine
                    || (!input.engine.is_sqlite()
                        && (previous.host != input.host
                            || previous.port != input.port
                            || previous.database != input.database
                            || previous.username != input.username))
                    || (input.engine.is_sqlite() && previous.sqlite_path != input.sqlite_path)
            });
        if engine.needs_password() && password.is_empty() && target_changed {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.error = "Re-enter the password when changing the connection target.".into();
            }
            cx.notify();
            return;
        }
        let password = if engine.needs_password() && !password.is_empty() {
            Some(password)
        } else {
            None
        };
        let store = self.store.clone();
        match store.save_profile(input, password) {
            Ok(saved) => {
                match store.load_profiles() {
                    Ok(profiles) => self.profiles = profiles,
                    Err(error) => {
                        log::error!("could not reload connection profiles: {error}");
                        self.status = error;
                        self.profiles.retain(|profile| profile.id != saved.id);
                        self.profiles.push(saved.clone());
                    }
                }
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.editing = Some(saved.id);
                    dialog.error.clear();
                }
                self.connect_from_dialog(saved.id, window, cx);
            }
            Err(error) => {
                log::error!("could not save connection profile: {error}");
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.error = error;
                }
                cx.notify();
            }
        }
    }

    pub(crate) fn connect_from_dialog(
        &mut self,
        profile_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        if dialog.busy {
            return;
        }
        let target = dialog.target_tab;
        dialog.busy = true;
        dialog.error.clear();
        self.connect_profile(profile_id, target, true, window, cx);
    }

    pub(crate) fn delete_profile(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            window,
            cx,
            "Remove connection?",
            "Tabs using it will close; running queries are cancelled, SQL editor contents are discarded, and staged edits are discarded.",
            move |this, cx| {
                let store = this.store.clone();
                match store.delete_profile(id) {
                    Ok(()) => {
                        match store.load_profiles() {
                            Ok(profiles) => this.profiles = profiles,
                            Err(error) => {
                                log::error!("could not reload connection profiles: {error}");
                                this.status = error;
                                this.profiles.retain(|profile| profile.id != id);
                            }
                        }
                        this.close_tabs_for_profile(id, cx);
                        if let Some(dialog) = this.dialog.as_mut() {
                            if dialog.editing == Some(id) {
                                dialog.editing = None;
                                dialog.error.clear();
                            }
                        }
                    }
                    Err(error) => {
                        log::error!("could not delete connection profile {id}: {error}");
                        if let Some(dialog) = this.dialog.as_mut() {
                            dialog.error = error;
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn close_tabs_for_profile(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let ids = self
            .tabs
            .iter()
            .filter(|tab| {
                tab.profile.as_ref().is_some_and(|profile| profile.id == id)
                    || tab
                        .connection
                        .as_ref()
                        .is_some_and(|info| info.profile.id == id)
            })
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        for tab_id in ids {
            self.close_tab_now(tab_id, cx);
        }
        let sessions = self
            .parked_connections
            .iter()
            .filter(|parked| parked.profile.id == id)
            .map(|parked| parked.connection.session_id)
            .collect::<Vec<_>>();
        self.parked_connections
            .retain(|parked| parked.profile.id != id);
        for session_id in sessions {
            self.disconnect_session(session_id);
        }
        cx.notify();
    }
}
