use std::collections::BTreeSet;

use koyo_core::models::{ConnectionInfo, ConnectionProfile};

use super::*;

impl KoyoApp {
    pub(crate) fn open_table(
        &mut self,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self
            .sidebar_index()
            .and_then(|index| self.tabs.get(index))
            .map(|tab| {
                (
                    tab.profile.clone(),
                    tab.connection.clone(),
                    tab_catalog(tab),
                    tab.expanded.clone(),
                )
            });
        let Some((Some(profile), shared, catalog, expanded)) = source else {
            self.status = "Open a connection before browsing tables.".into();
            cx.notify();
            return;
        };
        self.open_table_source(
            profile, shared, catalog, expanded, schema, table, self.focus, false, window, cx,
        );
    }

    /// Open a table reached from a diagram relation. The source tab supplies
    /// the profile, session, and catalog so the new tab stays on the same
    /// connection even when the explorer points at another pane.
    pub(crate) fn open_related_table(
        &mut self,
        source_tab_id: u64,
        schema: String,
        table: String,
        mode: OpenTableMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(source_tab_id) else {
            return;
        };
        let (profile, shared, catalog, expanded) = {
            let tab = &self.tabs[index];
            (
                tab.profile.clone(),
                tab.connection.clone(),
                tab_catalog(tab),
                tab.expanded.clone(),
            )
        };
        let Some(profile) = profile else {
            self.status = "Open a connection before browsing tables.".into();
            cx.notify();
            return;
        };
        let source_pane = if self.primary_tabs.contains(&source_tab_id) {
            Pane::Primary
        } else if self.secondary_tabs.contains(&source_tab_id) {
            Pane::Secondary
        } else {
            self.focus
        };
        let (target_pane, force_new) = related_table_target(source_pane, mode);
        if target_pane == Pane::Secondary && self.split.is_none() {
            self.split = Some(SplitAxis::Vertical);
        }
        self.open_table_source(
            profile,
            shared,
            catalog,
            expanded,
            schema,
            table,
            target_pane,
            force_new,
            window,
            cx,
        );
        self.focus_pane(target_pane, cx);
        self.reveal_active_tab(target_pane);
        self.persist_state(cx);
        cx.notify();
    }

    fn open_table_source(
        &mut self,
        profile: ConnectionProfile,
        shared: Option<ConnectionInfo>,
        catalog: CatalogSnapshot,
        expanded: BTreeSet<String>,
        schema: String,
        table: String,
        target_pane: Pane,
        force_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !force_new
            && self.focus_existing_table(
                target_pane,
                profile.id,
                &schema,
                &table,
                shared.clone(),
                catalog.clone(),
                window,
                cx,
            )
        {
            return;
        }

        // Prefer any live session for this profile, not just the source tab.
        let (shared, catalog) = match shared {
            Some(connection) => (Some(connection), catalog),
            None => match self.connection_for_profile(&profile) {
                Some((connection, catalog)) => (Some(connection), catalog),
                None => (None, catalog),
            },
        };

        let mut tab =
            self.make_table_tab(profile.clone(), schema.clone(), table.clone(), window, cx);
        tab.tables = catalog.tables.clone();
        tab.schema_objects = catalog.schema_objects.clone();
        tab.metadata_groups = catalog.metadata_groups.clone();
        tab.columns = catalog.columns.clone();
        tab.expanded = expanded;
        if let Some(connection) = shared {
            tab.connection = Some(connection);
            tab.connecting = false;
            if let TabBody::Table(table_tab) = &mut tab.body {
                table_tab.loading = false;
            }
            let tab_id = self.add_tab(tab, target_pane);
            self.attach_profile_session(profile.id, tab_id, cx);
            let needs_catalog = self
                .tab_index(tab_id)
                .is_some_and(|index| self.tabs[index].tables.is_empty())
                && !self.tabs.iter().any(|tab| {
                    tab.catalog_loading
                        && tab
                            .connection
                            .as_ref()
                            .is_some_and(|connection| connection.profile.id == profile.id)
                });
            if needs_catalog {
                self.load_tables(tab_id, cx);
            } else {
                self.load_page(tab_id, 0, Vec::new(), None, cx);
            }
            self.persist_state(cx);
            cx.notify();
            return;
        }
        if self.profile_is_connecting(&profile) {
            tab.connecting = false;
            tab.notice = "Waiting for the connection to finish.".into();
            if let TabBody::Table(table_tab) = &mut tab.body {
                table_tab.loading = false;
            }
            self.add_tab(tab, target_pane);
            self.persist_state(cx);
            cx.notify();
            return;
        }

        let tab_id = self.add_tab(tab, target_pane);
        let operation_id = Uuid::new_v4();
        if let Some(index) = self.tab_index(tab_id)
            && let TabBody::Table(table_tab) = &mut self.tabs[index].body
        {
            table_tab.operation_id = Some(operation_id);
        }
        self.persist_state(cx);
        cx.notify();

        let manager = self.manager.clone();
        let store = self.store.clone();
        let profile_for_task = profile.clone();
        let profile_name = profile.name.clone();
        let schema_for_task = schema.clone();
        let table_for_task = table.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let password = if profile_for_task.engine.needs_password() {
                    store.load_password(profile_for_task.id)?
                } else {
                    String::new()
                };
                let info =
                    super::super::connections::connect_with_timeout(
                        &manager,
                        &profile_for_task,
                        &password,
                    )
                    .await?;
                let mut catalog_errors = Vec::new();
                let tables = database::list_tables(&manager, info.session_id)
                    .await
                    .unwrap_or_else(|error| {
                        catalog_errors.push(error);
                        Vec::new()
                    });
                let objects = database::list_schema_objects(&manager, info.session_id)
                    .await
                    .unwrap_or_else(|error| {
                        catalog_errors.push(error);
                        Vec::new()
                    });
                let metadata = database::list_database_metadata(&manager, info.session_id)
                    .await
                    .unwrap_or_else(|error| {
                        catalog_errors.push(error);
                        Vec::new()
                    });
                let columns: Vec<ColumnCatalogEntry> =
                    database::list_column_catalog(&manager, info.session_id)
                        .await
                        .unwrap_or_else(|error| {
                            catalog_errors.push(error);
                            Vec::new()
                        });
                let page = database::load_table_page(
                    &manager,
                    info.session_id,
                    operation_id,
                    schema_for_task,
                    table_for_task,
                    0,
                    PAGE_LIMIT,
                    Vec::new(),
                    None,
                    true,
                )
                .await;
                let catalog_error = (!catalog_errors.is_empty()).then(|| catalog_errors.join("; "));
                Ok::<_, String>((
                    info,
                    tables,
                    objects,
                    metadata,
                    columns,
                    page,
                    catalog_error,
                ))
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    if let Ok((info, _, _, _, _, _, _)) = &result {
                        this.disconnect_session(info.session_id);
                    }
                    return;
                };
                match result {
                    Ok((info, tables, objects, metadata, columns, page, catalog_error)) => {
                        let operation_matches = this.tabs[index]
                            .body
                            .table()
                            .is_some_and(|table| table.operation_id == Some(operation_id));
                        if !operation_matches {
                            this.disconnect_session(info.session_id);
                            return;
                        }
                        this.tabs[index].connection = Some(info.clone());
                        this.tabs[index].profile = Some(profile.clone());
                        this.tabs[index].connecting = false;
                        this.tabs[index].tables = tables;
                        this.tabs[index].schema_objects = objects;
                        this.tabs[index].metadata_groups = metadata;
                        this.tabs[index].columns = columns;
                        if let Some(error) = catalog_error {
                            this.tabs[index].notice = error;
                        }
                        if let TabBody::Table(table_tab) = &mut this.tabs[index].body {
                            if table_tab.operation_id != Some(operation_id) {
                                return;
                            }
                            table_tab.operation_id = None;
                            table_tab.loading = false;
                            match page {
                                Ok(page) => table_tab.page = Some(page),
                                Err(error) => this.tabs[index].notice = error,
                            }
                        }
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
                    }
                    Err(error) => {
                        this.tabs[index].connecting = false;
                        this.tabs[index].notice = error.clone();
                        if let TabBody::Table(table_tab) = &mut this.tabs[index].body {
                            table_tab.loading = false;
                            if table_tab.operation_id == Some(operation_id) {
                                table_tab.operation_id = None;
                            }
                        }
                        this.push_toast(
                            ToastCategory::Connections,
                            profile.id.to_string(),
                            Notification::error(format!(
                                "Could not connect to {profile_name}. Check the profile and try again."
                            ))
                            .title("Connection failed")
                            .autohide(false),
                            cx,
                        );
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn open_schema_definition(
        &mut self,
        schema: String,
        name: String,
        kind: SchemaObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self
            .sidebar_index()
            .and_then(|index| self.tabs.get(index))
            .map(|tab| {
                (
                    tab.profile.clone(),
                    tab.connection.clone(),
                    tab.tables.clone(),
                    tab.schema_objects.clone(),
                    tab.metadata_groups.clone(),
                    tab.columns.clone(),
                    tab.expanded.clone(),
                )
            });
        let Some((Some(profile), shared, tables, objects, metadata, columns, expanded)) = source
        else {
            self.status = "Open a connection before inspecting functions or triggers.".into();
            cx.notify();
            return;
        };
        let target_pane = self.focus;
        let catalog = CatalogSnapshot {
            tables,
            schema_objects: objects,
            metadata_groups: metadata,
            columns,
        };
        let (shared, catalog) = match shared {
            Some(connection) => (Some(connection), catalog),
            None => match self.connection_for_profile(&profile) {
                Some((connection, catalog)) => (Some(connection), catalog),
                None => (None, catalog),
            },
        };
        let sql = object_definition_sql(profile.engine, kind, &schema, &name);
        let mut tab = self.make_query_tab(Some(profile.clone()), window, cx);
        tab.title = format!("{schema}.{name}").into();
        tab.tables = catalog.tables;
        tab.schema_objects = catalog.schema_objects;
        tab.metadata_groups = catalog.metadata_groups;
        tab.columns = catalog.columns;
        tab.expanded = expanded;
        if let TabBody::Query(query) = &mut tab.body {
            query.editor.update(cx, |editor, cx| {
                editor.set_value(sql.clone(), window, cx);
            });
            query.starter = sql.into();
            query.definition_kind = Some(kind);
        }
        let connected = shared.is_some();
        tab.connection = shared;
        tab.connecting = false;
        let tab_id = self.add_tab(tab, target_pane);
        if connected {
            self.attach_profile_session(profile.id, tab_id, cx);
            self.run_query(tab_id, cx);
        } else {
            if let Some(index) = self.tab_index(tab_id)
                && let TabBody::Query(query) = &mut self.tabs[index].body
            {
                query.pending_run = true;
            }
            self.connect_profile(profile.id, Some(tab_id), false, window, cx);
        }
        self.persist_state(cx);
        cx.notify();
    }

    fn focus_existing_table(
        &mut self,
        pane: Pane,
        profile_id: uuid::Uuid,
        schema: &str,
        table: &str,
        connection: Option<koyo_core::models::ConnectionInfo>,
        catalog: CatalogSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let candidates = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let open = tab.body.table()?;
                let pane = if self.primary_tabs.contains(&tab.id) {
                    Pane::Primary
                } else if self.secondary_tabs.contains(&tab.id) {
                    Pane::Secondary
                } else {
                    return None;
                };
                Some((
                    tab.id,
                    pane,
                    tab.profile.as_ref().map(|profile| profile.id),
                    open.schema.as_str(),
                    open.table.as_str(),
                ))
            })
            .collect::<Vec<_>>();
        let Some(tab_id) = table_tab_to_focus(&candidates, pane, profile_id, schema, table) else {
            return false;
        };
        let Some(order) = self.pane_tab_ids(pane).iter().position(|id| *id == tab_id) else {
            return false;
        };
        self.switch_tab(pane, order, window, cx);
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        let needs_connect = self.tabs[index].connection.is_none() && !self.tabs[index].connecting;
        self.status = format!("Focused {schema}.{table}.");
        if needs_connect {
            if let Some(connection) = connection {
                self.tabs[index].connection = Some(connection.clone());
                self.tabs[index].connecting = false;
                self.tabs[index].profile = Some(connection.profile.clone());
                self.tabs[index].notice.clear();
                self.tabs[index].tables = catalog.tables.clone();
                self.tabs[index].schema_objects = catalog.schema_objects.clone();
                self.tabs[index].metadata_groups = catalog.metadata_groups.clone();
                self.tabs[index].columns = catalog.columns.clone();
                if let TabBody::Table(table_tab) = &mut self.tabs[index].body {
                    table_tab.page = None;
                    table_tab.total_rows_cache = None;
                    table_tab.staged.clear();
                    table_tab.selected_keys.clear();
                    table_tab.loading = false;
                    table_tab.operation_id = None;
                    table_tab.foreign_keys.clear();
                    table_tab.foreign_keys_loading = false;
                    table_tab.indexes.clear();
                    table_tab.diagram_metadata_loaded = false;
                }
                self.status = format!("Connected to {}.", connection.profile.name);
                self.load_page(tab_id, 0, Vec::new(), None, cx);
            } else {
                self.connect_profile(profile_id, Some(tab_id), false, window, cx);
            }
        } else {
            cx.notify();
        }
        true
    }
}

fn tab_catalog(tab: &Tab) -> CatalogSnapshot {
    CatalogSnapshot {
        tables: tab.tables.clone(),
        schema_objects: tab.schema_objects.clone(),
        metadata_groups: tab.metadata_groups.clone(),
        columns: tab.columns.clone(),
    }
}

fn related_table_target(source: Pane, mode: OpenTableMode) -> (Pane, bool) {
    match mode {
        OpenTableMode::FocusOrOpen => (source, false),
        OpenTableMode::NewTab => (source, true),
        OpenTableMode::OtherPane => match source {
            Pane::Primary => (Pane::Secondary, true),
            Pane::Secondary => (Pane::Primary, true),
        },
    }
}

#[cfg(test)]
mod target_tests {
    use super::{OpenTableMode, Pane, related_table_target};

    #[test]
    fn related_table_actions_pick_the_right_pane() {
        assert_eq!(
            related_table_target(Pane::Primary, OpenTableMode::FocusOrOpen),
            (Pane::Primary, false)
        );
        assert_eq!(
            related_table_target(Pane::Primary, OpenTableMode::NewTab),
            (Pane::Primary, true)
        );
        assert_eq!(
            related_table_target(Pane::Primary, OpenTableMode::OtherPane),
            (Pane::Secondary, true)
        );
        assert_eq!(
            related_table_target(Pane::Secondary, OpenTableMode::OtherPane),
            (Pane::Primary, true)
        );
    }
}
