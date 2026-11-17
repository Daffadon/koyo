use std::collections::BTreeSet;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::ProfileInput;
use koyo_core::database;
use koyo_core::models::{
    ColumnCatalogEntry, Engine, MetadataGroup, SchemaObject, SslMode, TableSummary,
};
use uuid::Uuid;

use crate::app::*;

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
        self.tabs[index].catalog_generation = self.tabs[index].catalog_generation.wrapping_add(1);
        let generation = self.tabs[index].catalog_generation;
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
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                if this.tabs[index].catalog_generation != generation
                    || this.tabs[index]
                        .connection
                        .as_ref()
                        .map(|connection| connection.session_id)
                        != Some(session_id)
                {
                    return;
                }
                match result {
                    Ok((tables, objects, metadata, columns)) => {
                        if let TabBody::Query(query) = &mut this.tabs[index].body {
                            query
                                .completion
                                .borrow_mut()
                                .replace(&tables, &objects, &columns);
                        }
                        // The explorer starts closed; expand all is an explicit action.
                        this.tabs[index].expanded.clear();
                        this.tabs[index].tables = tables;
                        this.tabs[index].schema_objects = objects;
                        this.tabs[index].metadata_groups = metadata;
                        this.status = format!("{} objects.", this.tabs[index].schema_objects.len());
                    }
                    Err(error) => {
                        this.tabs[index].notice = error;
                    }
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
            let index = self.tab_index(id).expect("tab exists");
            self.tabs[index].connection.is_some()
                && self.tabs[index]
                    .profile
                    .as_ref()
                    .is_some_and(|current| current.id != profile.id)
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

        let previous_profile = self.tabs[index].profile.clone();
        self.tabs[index].connecting = true;
        self.tabs[index].notice = if self.tabs[index].connection.is_some() {
            "Switching connection...".to_string()
        } else {
            "Connecting...".to_string()
        };
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.busy = true;
            dialog.error.clear();
        }
        cx.notify();

        let manager = self.manager.clone();
        let store = self.store.clone();
        let profile_for_task = profile.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let password = if profile_for_task.engine.needs_password() {
                    store.load_password(profile_for_task.id)?
                } else {
                    String::new()
                };
                database::connect(&manager, &profile_for_task, &password).await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    if let Ok(info) = &result {
                        this.disconnect_session(info.session_id);
                    }
                    return;
                };
                match result {
                    Ok(info) => {
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
                        this.tabs[index].profile = Some(info.profile.clone());
                        this.tabs[index].connecting = false;
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
                                let offset =
                                    table.page.as_ref().map(|page| page.offset).unwrap_or(0);
                                let filters = table.filters.clone();
                                let sort = table.sort.clone();
                                // Row identity and staged values belong to the
                                // previous session. Never let Apply use them
                                // against the newly connected server.
                                table.page = None;
                                table.total_rows_cache = None;
                                table.staged.clear();
                                table.loading = false;
                                table.operation_id = None;
                                table.foreign_keys.clear();
                                table.foreign_keys_loading = false;
                                table.indexes.clear();
                                table.diagram_metadata_loaded = false;
                                this.load_page(tab_id, offset, filters, sort, cx);
                            }
                        }
                        if run_pending {
                            this.run_query(tab_id, cx);
                        }
                        if from_dialog {
                            this.dialog = None;
                        }
                    }
                    Err(error) => {
                        this.tabs[index].connecting = false;
                        if let TabBody::Query(query) = &mut this.tabs[index].body {
                            query.pending_run = false;
                        }
                        this.tabs[index].notice = error.clone();
                        this.status = error.clone();
                        if from_dialog {
                            if let Some(dialog) = this.dialog.as_mut() {
                                dialog.busy = false;
                                dialog.error = error;
                            }
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

    pub(crate) fn close_connection_dialog(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
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
                self.profiles = store.load_profiles().unwrap_or_default();
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.editing = Some(saved.id);
                    dialog.error.clear();
                }
                self.connect_from_dialog(saved.id, window, cx);
            }
            Err(error) => {
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
                        this.profiles = store.load_profiles().unwrap_or_default();
                        this.close_tabs_for_profile(id, cx);
                        if let Some(dialog) = this.dialog.as_mut() {
                            if dialog.editing == Some(id) {
                                dialog.editing = None;
                                dialog.error.clear();
                            }
                        }
                    }
                    Err(error) => {
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
