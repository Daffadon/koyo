use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::SessionManager;
use koyo_core::database;
use koyo_core::models::{ColumnCatalogEntry, GridChange, TableFilter, TableSort};
use uuid::Uuid;

use super::notifications::ToastCategory;
use crate::app::*;

/// A dead TCP session can otherwise leave a page load spinning forever.
const TABLE_IO_TIMEOUT: Duration = Duration::from_secs(30);

fn table_tab_to_focus(
    candidates: &[(u64, Pane, Option<uuid::Uuid>, &str, &str)],
    pane: Pane,
    profile_id: uuid::Uuid,
    schema: &str,
    table: &str,
) -> Option<u64> {
    candidates
        .iter()
        .find(
            |(_, candidate_pane, profile, candidate_schema, candidate_table)| {
                *candidate_pane == pane
                    && *profile == Some(profile_id)
                    && *candidate_schema == schema
                    && *candidate_table == table
            },
        )
        .map(|(id, _, _, _, _)| *id)
}

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
                    tab.tables.clone(),
                    tab.schema_objects.clone(),
                    tab.metadata_groups.clone(),
                    tab.columns.clone(),
                    tab.expanded.clone(),
                )
            });
        let Some((Some(profile), shared, tables, objects, metadata, columns, expanded)) = source
        else {
            self.status = "Open a connection before browsing tables.".into();
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
        if self.focus_existing_table(
            target_pane,
            profile.id,
            &schema,
            &table,
            shared.clone(),
            catalog.clone(),
            window,
            cx,
        ) {
            return;
        }

        // Prefer any live session for this profile, not just the sidebar tab.
        let (shared, catalog) = match shared {
            Some(connection) => (Some(connection), catalog),
            None => match self.connection_for_profile(profile.id) {
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
            let needs_catalog = self.tabs[self.tab_index(tab_id).expect("tab exists")]
                .tables
                .is_empty()
                && !self.tabs.iter().any(|tab| {
                    tab.catalog_loading
                        && tab
                            .connection
                            .as_ref()
                            .is_some_and(|connection| connection.profile.id == profile.id)
                });
            if needs_catalog {
                self.load_tables(tab_id, cx);
            }
            self.persist_state(cx);
            cx.notify();
            self.load_page(tab_id, 0, Vec::new(), None, cx);
            return;
        }
        if self.profile_is_connecting(profile.id) {
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
                let info = database::connect(&manager, &profile_for_task, &password).await?;
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
        self.switch_tab(pane, order, cx);
        let index = self.tab_index(tab_id).expect("tab exists");
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

    pub(crate) fn load_page(
        &mut self,
        tab_id: u64,
        offset: u32,
        filters: Vec<TableFilter>,
        sort: Option<TableSort>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        table.loading = true;
        table.filter_error.clear();
        table.load_sequence += 1;
        let sequence = table.load_sequence;
        let operation_id = Uuid::new_v4();
        table.operation_id = Some(operation_id);
        let schema = table.schema.clone();
        let name = table.table.clone();
        let filters_for_task = filters.clone();
        let sort_for_task = sort.clone();
        let count_total = offset == 0 || table.total_rows_cache.is_none();
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            tab_id,
            async move {
                tokio::time::timeout(
                    TABLE_IO_TIMEOUT,
                    database::load_table_page(
                        &manager,
                        session_id,
                        operation_id,
                        schema,
                        name,
                        offset,
                        PAGE_LIMIT,
                        filters_for_task,
                        sort_for_task,
                        count_total,
                    ),
                )
                .await
                .unwrap_or_else(|_| {
                    Err("Table load timed out. The database connection may have closed.".into())
                })
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Table(table) = &mut this.tabs[index].body else {
                    return;
                };
                if table.load_sequence != sequence || table.operation_id != Some(operation_id) {
                    return;
                }
                table.operation_id = None;
                table.loading = false;
                let error = match result {
                    Ok(page) => {
                        if let Some(total) = page.total_rows {
                            table.total_rows_cache = Some(total);
                        }
                        table.page = Some(page);
                        table.selected_keys.clear();
                        table.filters = filters;
                        table.sort = sort;
                        None
                    }
                    Err(error) => Some(error),
                };
                if let Some(error) = error {
                    if database::is_connection_lost(&error) || error.contains("timed out") {
                        this.mark_connection_lost(
                            session_id,
                            "Connection lost while loading rows. Reconnect to continue.",
                            cx,
                        );
                    } else {
                        this.tabs[index].notice = error;
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn set_table_mode(&mut self, tab_id: u64, mode: TableMode, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let should_load = {
            let TabBody::Table(table) = &mut self.tabs[index].body else {
                return;
            };
            table.mode = mode;
            mode == TableMode::Diagram
                && !table.diagram_metadata_loaded
                && !table.foreign_keys_loading
        };
        if should_load {
            self.load_diagram_metadata(tab_id, cx);
        }
        cx.notify();
    }

    fn load_diagram_metadata(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.foreign_keys_loading {
            return;
        }
        let schema = table.schema.clone();
        let table_name = table.table.clone();
        let session_id = connection.session_id;
        table.foreign_keys_loading = true;
        table.diagram_metadata_loaded = true;
        table.diagram_generation = table.diagram_generation.wrapping_add(1);
        let generation = table.diagram_generation;
        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let foreign_keys =
                    database::list_foreign_keys(&manager, session_id, &schema, &table_name).await?;
                let indexes =
                    database::list_indexes(&manager, session_id, &schema, &table_name).await?;
                Ok::<_, String>((foreign_keys, indexes))
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                if this.tabs[index]
                    .connection
                    .as_ref()
                    .map(|connection| connection.session_id)
                    != Some(session_id)
                {
                    return;
                }
                if let TabBody::Table(table) = &mut this.tabs[index].body {
                    if table.diagram_generation != generation {
                        return;
                    }
                    table.foreign_keys_loading = false;
                    match result {
                        Ok((foreign_keys, indexes)) => {
                            table.foreign_keys = foreign_keys;
                            table.indexes = indexes;
                        }
                        Err(error) => {
                            table.diagram_metadata_loaded = false;
                            this.tabs[index].notice = error;
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn open_table_export(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.exporting {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let current_page = page.offset / PAGE_LIMIT + 1;
        let total_pages = table
            .total_rows_cache
            .map(|total| {
                total
                    .div_ceil(u64::from(PAGE_LIMIT))
                    .min(u64::from(u32::MAX)) as u32
            })
            .unwrap_or(0);
        let page_input = cx.new(|cx| InputState::new(window, cx).placeholder("Page number"));
        let end_page_input = cx.new(|cx| InputState::new(window, cx).placeholder("End page"));
        let file_name = table_csv_file_name(&table.schema, &table.table);
        let path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Choose where to save the CSV"));
        set_input(&path_input, &default_export_path(&file_name), window, cx);
        self.export = Some(TableExportDialog {
            tab_id,
            title: format!("{}.{}", table.schema, table.table).into(),
            current_page,
            total_pages,
            scope: ExportScope::CurrentPage,
            page: page_input,
            end_page: end_page_input,
            path: path_input,
            file_name,
            error: String::new(),
        });
        cx.notify();
    }

    pub(crate) fn close_table_export(&mut self, cx: &mut Context<Self>) {
        self.export = None;
        cx.notify();
    }

    pub(crate) fn browse_table_export_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.export.as_ref() else {
            return;
        };
        let path_input = dialog.path.clone();
        let file_name = dialog.file_name.clone();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(&file_name)
            .save_file()
        else {
            return;
        };
        set_input(&path_input, &path.display().to_string(), window, cx);
        cx.notify();
    }

    pub(crate) fn set_table_export_scope(&mut self, scope: ExportScope, cx: &mut Context<Self>) {
        if let Some(dialog) = self.export.as_mut() {
            dialog.scope = scope;
            dialog.error.clear();
        }
        cx.notify();
    }

    pub(crate) fn download_table_csv(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.export.as_ref() else {
            return;
        };
        let tab_id = dialog.tab_id;
        let scope = dialog.scope;
        let page_text = dialog.page.read(cx).value().to_string();
        let end_page_text = dialog.end_page.read(cx).value().to_string();
        let path_text = dialog.path.read(cx).value().trim().to_string();
        let Some(index) = self.tab_index(tab_id) else {
            self.export = None;
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "This table is not connected.".into();
            }
            cx.notify();
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            self.export = None;
            return;
        };
        if table.exporting {
            return;
        }
        let current_page = table.page.as_ref().map(|page| page.offset / PAGE_LIMIT + 1);
        let Some(current_page) = current_page else {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Load a table page before downloading.".into();
            }
            cx.notify();
            return;
        };
        let total_pages = table
            .total_rows_cache
            .map(|total| {
                total
                    .div_ceil(u64::from(PAGE_LIMIT))
                    .min(u64::from(u32::MAX)) as u32
            })
            .unwrap_or(0);
        let (first_page, last_page) = match resolve_export_pages(
            scope,
            current_page,
            &page_text,
            &page_text,
            &end_page_text,
            total_pages,
        ) {
            Ok(range) => range,
            Err(error) => {
                if let Some(dialog) = self.export.as_mut() {
                    dialog.error = error;
                }
                cx.notify();
                return;
            }
        };
        let schema = table.schema.clone();
        let table_name = table.table.clone();
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        let export_title = format!("{schema}.{table_name}");
        if path_text.is_empty() {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Choose where to save the CSV.".into();
            }
            cx.notify();
            return;
        }
        let path = PathBuf::from(path_text);
        if path.is_dir() {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Choose a file path, not a folder.".into();
            }
            cx.notify();
            return;
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.is_dir()
        {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "The selected folder does not exist.".into();
            }
            cx.notify();
            return;
        }
        let path = if path.extension().is_none() {
            path.with_extension("csv")
        } else {
            path
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.exporting = true;
            self.tabs[index].notice = format!("Exporting CSV to {}...", path.display());
        }
        self.export = None;
        self.push_toast(
            ToastCategory::Exports,
            tab_id.to_string(),
            Notification::info(format!("Exporting {export_title}.")).title("Downloading CSV"),
            cx,
        );
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        let path_for_task = path.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                export_table_csv(
                    manager,
                    session_id,
                    schema,
                    table_name,
                    first_page,
                    last_page,
                    filters,
                    sort,
                    path_for_task,
                )
                .await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                if let TabBody::Table(table) = &mut this.tabs[index].body {
                    table.exporting = false;
                }
                match result {
                    Ok((rows, truncated)) => {
                        let suffix = if truncated {
                            " Some cell values were truncated by Koyo's limits."
                        } else {
                            ""
                        };
                        let message = format!(
                            "Downloaded {rows} {} to {}.{suffix}",
                            if rows == 1 { "row" } else { "rows" },
                            path.display()
                        );
                        this.tabs[index].notice = message;
                        let file_name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("CSV");
                        this.push_toast(
                            ToastCategory::Exports,
                            tab_id.to_string(),
                            Notification::success(format!("Saved {file_name}.")).title("CSV ready"),
                            cx,
                        );
                    }
                    Err(error) => {
                        if database::is_connection_lost(&error) || error.contains("timed out") {
                            this.mark_connection_lost(
                                session_id,
                                "Connection lost while downloading CSV. Reconnect to continue.",
                                cx,
                            );
                        } else {
                            let message = format!("CSV download failed: {error}");
                            this.tabs[index].notice = message.clone();
                            this.push_toast(
                                ToastCategory::Exports,
                                tab_id.to_string(),
                                Notification::error("Export failed. See the tab for details.")
                                    .title("CSV failed")
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

    pub(crate) fn apply_changes(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        let deletes = table
            .staged
            .iter()
            .filter(|change| matches!(change, StagedChange::Delete { .. }))
            .count();
        if deletes > 0 {
            self.confirm(
                window,
                cx,
                "Delete staged rows?",
                "Apply permanently deletes the marked rows. Other staged edits are applied in the same transaction.",
                move |this, cx| this.commit_staged_changes(tab_id, cx),
            );
        } else {
            self.commit_staged_changes(tab_id, cx);
        }
    }

    fn commit_staged_changes(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.staged.is_empty() || table.loading {
            return;
        }
        let schema = table.schema.clone();
        let name = table.table.clone();
        let offset = table.page.as_ref().map(|page| page.offset).unwrap_or(0);
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        let changes = table
            .staged
            .iter()
            .map(|change| match change.clone() {
                StagedChange::Insert(insert) => GridChange::Insert {
                    values: insert.values,
                },
                StagedChange::Update {
                    key,
                    version,
                    values,
                    original,
                    truncated,
                } => GridChange::Update {
                    key,
                    version,
                    values,
                    original,
                    truncated,
                },
                StagedChange::Delete {
                    key,
                    version,
                    original,
                    truncated,
                } => GridChange::Delete {
                    key,
                    version,
                    original,
                    truncated,
                },
            })
            .collect::<Vec<_>>();
        table.loading = true;
        let operation_id = Uuid::new_v4();
        table.operation_id = Some(operation_id);
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            tab_id,
            async move {
                database::apply_grid_changes(
                    &manager,
                    session_id,
                    operation_id,
                    schema,
                    name,
                    changes,
                )
                .await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Table(table) = &mut this.tabs[index].body else {
                    return;
                };
                if table.operation_id != Some(operation_id) {
                    return;
                }
                table.operation_id = None;
                table.loading = false;
                match result {
                    Ok(summary) => {
                        table.staged.clear();
                        table.selected_keys.clear();
                        table.total_rows_cache = None;
                        let message = format!(
                            "{} staged {} applied.",
                            summary.applied,
                            if summary.applied == 1 { "change" } else { "changes" }
                        );
                        this.tabs[index].notice = message.clone();
                        this.push_toast(
                            ToastCategory::Data,
                            tab_id.to_string(),
                            Notification::success(message).title("Changes applied"),
                            cx,
                        );
                        this.load_page(tab_id, offset, filters, sort, cx);
                    }
                    Err(error) => {
                        if database::is_connection_lost(&error) {
                            this.mark_connection_lost(
                                session_id,
                                "Connection lost while applying changes. Reconnect to retry.",
                                cx,
                            );
                        } else if error.starts_with("COMMIT_OUTCOME_UNKNOWN:") {
                            table.staged.clear();
                            table.selected_keys.clear();
                            let message = "Commit result could not be confirmed. Page reloaded; verify data before retrying.";
                            this.tabs[index].notice = message.into();
                            this.push_toast(
                                ToastCategory::Data,
                                tab_id.to_string(),
                                Notification::warning(
                                    "Verify the data before retrying.",
                                )
                                .title("Commit unconfirmed")
                                .autohide(false),
                                cx,
                            );
                            this.load_page(tab_id, offset, filters, sort, cx);
                        } else {
                            this.tabs[index].notice = error.clone();
                            this.push_toast(
                                ToastCategory::Data,
                                tab_id.to_string(),
                                Notification::error(
                                    "No changes were applied. See the tab for details.",
                                )
                                .title("Save failed")
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

    pub(crate) fn discard_changes(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.staged.clear();
        }
        cx.notify();
    }

    pub(crate) fn set_row_selected(
        &mut self,
        tab_id: u64,
        row_index: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        let Some((row, key_truncated)) =
            table
                .page
                .as_ref()
                .filter(|page| page.editable)
                .and_then(|page| {
                    let row = page.rows.get(row_index)?;
                    Some((row.clone(), key_is_truncated(page, row)))
                })
        else {
            return;
        };
        let already_deleted = table
            .staged
            .iter()
            .any(|change| matches!(change, StagedChange::Delete { key, .. } if key == &row.key));
        if key_truncated || already_deleted {
            return;
        }
        if selected {
            table.selected_keys.insert(row.key);
        } else {
            table.selected_keys.remove(&row.key);
        }
        cx.notify();
    }

    pub(crate) fn set_all_rows_selected(
        &mut self,
        tab_id: u64,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        table.selected_keys.clear();
        if !selected {
            cx.notify();
            return;
        }
        let Some(page) = table.page.as_ref().filter(|page| page.editable) else {
            return;
        };
        for row in &page.rows {
            let already_deleted = table.staged.iter().any(
                |change| matches!(change, StagedChange::Delete { key, .. } if key == &row.key),
            );
            if !already_deleted && !key_is_truncated(page, row) {
                table.selected_keys.insert(row.key.clone());
            }
        }
        cx.notify();
    }

    pub(crate) fn delete_selected_rows(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading || table.selected_keys.is_empty() {
            return;
        }
        let Some(page) = table.page.as_ref().filter(|page| page.editable) else {
            return;
        };
        let rows = page.rows.clone();
        let mut staged = 0usize;
        let mut skipped = 0usize;
        for row in rows {
            if !table.selected_keys.contains(&row.key) {
                continue;
            }
            if key_is_truncated(page, &row) {
                skipped += 1;
                continue;
            }
            if stage_row_delete_change(
                &mut table.staged,
                row.key,
                row.version,
                row.values,
                row.truncated,
            ) {
                staged += 1;
            }
        }
        table.selected_keys.clear();
        this_notice(
            &mut self.tabs[index],
            &if skipped > 0 {
                format!(
                    "{staged} row{} marked for deletion; {skipped} skipped because the primary key was truncated.",
                    if staged == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "{staged} row{} marked for deletion. Apply changes to delete {}.",
                    if staged == 1 { "" } else { "s" },
                    if staged == 1 { "it" } else { "them" }
                )
            },
        );
        cx.notify();
    }

    pub(crate) fn clear_row_selection(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.selected_keys.clear();
        }
        cx.notify();
    }

    pub(crate) fn toggle_row_delete(
        &mut self,
        tab_id: u64,
        row_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        let Some((row, key_truncated)) =
            table
                .page
                .as_ref()
                .filter(|page| page.editable)
                .and_then(|page| {
                    let row = page.rows.get(row_index)?;
                    let key_truncated = page.columns.iter().enumerate().any(|(index, column)| {
                        column.primary_key && row.truncated.get(index).copied().unwrap_or(false)
                    });
                    Some((row.clone(), key_truncated))
                })
        else {
            return;
        };
        if key_truncated {
            this_notice(
                &mut self.tabs[index],
                "The primary key was truncated for display. Filter the row before deleting it.",
            );
            cx.notify();
            return;
        }
        let deleted = stage_row_delete(
            &mut table.staged,
            row.key.clone(),
            row.version,
            row.values,
            row.truncated,
        );
        if deleted {
            table.selected_keys.remove(&row.key);
        }
        this_notice(
            &mut self.tabs[index],
            if deleted {
                "Row marked for deletion. Apply changes to delete it."
            } else {
                "Row deletion undone."
            },
        );
        cx.notify();
    }

    pub(crate) fn add_row(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let editable = self.tabs[index]
            .body
            .table()
            .and_then(|table| table.page.as_ref())
            .is_some_and(|page| page.editable);
        if !editable || self.tabs[index].connection.is_none() {
            return;
        }
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let local_id = table.next_insert_id;
        table.next_insert_id += 1;
        table.staged.push(StagedChange::Insert(StagedInsert {
            local_id,
            values: BTreeMap::new(),
        }));
        cx.notify();
    }

    pub(crate) fn request_page(
        &mut self,
        tab_id: u64,
        offset: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        if !table.staged.is_empty() {
            self.confirm(
                window,
                cx,
                "Discard staged changes?",
                "Changing the page discards staged edits.",
                move |this, cx| {
                    this.discard_changes(tab_id, cx);
                    this.load_page(tab_id, offset, filters.clone(), sort.clone(), cx);
                },
            );
        } else {
            self.load_page(tab_id, offset, filters, sort, cx);
        }
    }

    pub(crate) fn request_sort(&mut self, tab_id: u64, column: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.staged.is_empty() == false {
            this_notice(
                &mut self.tabs[index],
                "Apply or discard staged edits to sort.",
            );
            cx.notify();
            return;
        }
        if table.loading {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        if !info.sortable {
            return;
        }
        let name = info.name.clone();
        let next = match &table.sort {
            Some(sort) if sort.column == name && !sort.descending => Some(TableSort {
                column: name,
                descending: true,
            }),
            Some(sort) if sort.column == name && sort.descending => None,
            _ => Some(TableSort {
                column: name,
                descending: false,
            }),
        };
        let filters = table.filters.clone();
        self.load_page(tab_id, 0, filters, next, cx);
    }

    pub(crate) fn complete_active_filter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((tab_id, columns, value, cursor)) = self
            .focused_index()
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| {
                let TabBody::Table(table) = &tab.body else {
                    return None;
                };
                let page = table.page.as_ref()?;
                let input = table.filter_input.read(cx);
                Some((
                    tab.id,
                    page.columns.clone(),
                    input.value().to_string(),
                    input.cursor(),
                ))
            })
        else {
            return false;
        };
        let Some(suggestion) = filter_suggestions(&columns, &value, cursor)
            .into_iter()
            .next()
        else {
            return false;
        };
        self.complete_filter_input(tab_id, suggestion.insert, window, cx);
        true
    }

    pub(crate) fn complete_filter_input(
        &mut self,
        tab_id: u64,
        insert: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.loading || !table.staged.is_empty() {
            return;
        }
        let (value, cursor) = {
            let input = table.filter_input.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let (value, cursor) = insert_filter_suggestion(&value, cursor, &insert);
        table.filter_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.set_selected_range(cursor..cursor, cx);
        });
        table.filter_error.clear();
        cx.notify();
    }

    pub(crate) fn apply_filter(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        table.filter_error.clear();
        if table.loading {
            table.filter_error = "Wait for the current page load to finish.".into();
            cx.notify();
            return;
        }
        if !table.staged.is_empty() {
            table.filter_error = "Apply or discard staged edits to change filters.".into();
            cx.notify();
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let input = table.filter_input.read(cx).value().to_string();
        let filters = match parse_filter_input(&page.columns, &input) {
            Ok(filters) => filters,
            Err(error) => {
                table.filter_error = error;
                cx.notify();
                return;
            }
        };
        table.filters = filters.clone();
        let sort = table.sort.clone();
        self.load_page(tab_id, 0, filters, sort, cx);
    }

    pub(crate) fn request_clear_filters(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        table.filter_error.clear();
        if table.loading {
            table.filter_error = "Wait for the current page load to finish.".into();
            cx.notify();
            return;
        }
        if !table.filters.is_empty() && !table.staged.is_empty() {
            self.confirm(
                window,
                cx,
                "Discard staged changes?",
                "Clearing filters reloads this page and discards staged edits.",
                move |this, cx| {
                    this.discard_changes(tab_id, cx);
                    this.clear_filters_now(tab_id, cx);
                },
            );
            return;
        }
        self.clear_filters_now(tab_id, cx);
        self.clear_filter_input(tab_id, window, cx);
        cx.notify();
    }

    fn clear_filter_input(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        table.filter_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
    }

    fn clear_filters_now(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let had_filters = !table.filters.is_empty();
        table.filters.clear();
        table.filter_error.clear();
        let sort = table.sort.clone();
        if had_filters {
            self.load_page(tab_id, 0, Vec::new(), sort, cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn refresh_active_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.focused_index().and_then(|index| self.tabs.get(index)) else {
            return;
        };
        if !matches!(tab.body, TabBody::Table(_)) {
            return;
        }
        let tab_id = tab.id;
        self.refresh_current_table(tab_id, window, cx);
    }

    pub(crate) fn refresh_current_table(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        // Refresh is the user's explicit request for current data.
        table.total_rows_cache = None;
        let offset = table.page.as_ref().map(|page| page.offset).unwrap_or(0);
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        let has_staged = !table.staged.is_empty();
        if has_staged {
            self.confirm(
                window,
                cx,
                "Discard staged changes?",
                "Reloading this page discards staged edits.",
                move |this, cx| {
                    this.discard_changes(tab_id, cx);
                    this.load_page(tab_id, offset, filters.clone(), sort.clone(), cx);
                },
            );
        } else {
            self.load_page(tab_id, offset, filters, sort, cx);
        }
    }
}

async fn export_table_csv(
    manager: Arc<SessionManager>,
    session_id: Uuid,
    schema: String,
    table: String,
    first_page: u32,
    last_page: Option<u32>,
    filters: Vec<TableFilter>,
    sort: Option<TableSort>,
    path: PathBuf,
) -> Result<(usize, bool), String> {
    let mut file =
        File::create(&path).map_err(|error| format!("Could not create file: {error}"))?;
    let mut page_number = first_page;
    let mut rows = 0usize;
    let mut truncated = false;
    let mut first_chunk = true;
    loop {
        if last_page.is_some_and(|last| page_number > last) {
            break;
        }
        let offset = page_number
            .saturating_sub(1)
            .checked_mul(PAGE_LIMIT)
            .ok_or_else(|| "Page range is too large.".to_string())?;
        let page = tokio::time::timeout(
            TABLE_IO_TIMEOUT,
            database::load_table_page(
                &manager,
                session_id,
                Uuid::new_v4(),
                schema.clone(),
                table.clone(),
                offset,
                PAGE_LIMIT,
                filters.clone(),
                sort.clone(),
                false,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            Err("CSV export timed out. The database connection may have closed.".into())
        })?;
        let chunk = table_page_as_csv(&page, first_chunk);
        if !chunk.is_empty() {
            file.write_all(chunk.as_bytes())
                .map_err(|error| format!("Could not write file: {error}"))?;
        }
        first_chunk = false;
        rows += page.rows.len();
        truncated |= page
            .rows
            .iter()
            .any(|row| row.truncated.iter().any(|value| *value));
        if !page.has_more || page.rows.is_empty() {
            break;
        }
        page_number = page_number
            .checked_add(1)
            .ok_or_else(|| "Page range is too large.".to_string())?;
    }
    file.flush()
        .map_err(|error| format!("Could not finish file: {error}"))?;
    Ok((rows, truncated))
}

fn stage_row_delete_change(
    staged: &mut Vec<StagedChange>,
    key: Vec<Option<String>>,
    version: String,
    original: Vec<Option<String>>,
    truncated: Vec<bool>,
) -> bool {
    if staged
        .iter()
        .any(|change| matches!(change, StagedChange::Delete { key: staged_key, .. } if staged_key == &key))
    {
        return false;
    }
    staged.retain(|change| !matches!(change, StagedChange::Update { key: staged_key, .. } if staged_key == &key));
    staged.push(StagedChange::Delete {
        key,
        version,
        original,
        truncated,
    });
    true
}

fn stage_row_delete(
    staged: &mut Vec<StagedChange>,
    key: Vec<Option<String>>,
    version: String,
    original: Vec<Option<String>>,
    truncated: Vec<bool>,
) -> bool {
    if let Some(index) = staged
        .iter()
        .position(|change| matches!(change, StagedChange::Delete { key: staged_key, .. } if staged_key == &key))
    {
        staged.remove(index);
        return false;
    }
    stage_row_delete_change(staged, key, version, original, truncated);
    true
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use koyo_core::SessionManager;
    use koyo_core::database;
    use koyo_core::models::{ConnectionProfile, Engine, SslMode};
    use uuid::Uuid;

    use super::{export_table_csv, stage_row_delete, stage_row_delete_change, table_tab_to_focus};
    use crate::app::{Pane, StagedChange};
    use std::collections::BTreeMap;

    #[test]
    fn table_focus_is_scoped_to_one_pane() {
        let profile = Uuid::new_v4();
        let other = Uuid::new_v4();
        let candidates = [
            (1, Pane::Primary, Some(profile), "main", "users"),
            (2, Pane::Secondary, Some(profile), "main", "users"),
            (3, Pane::Primary, Some(profile), "main", "orders"),
            (4, Pane::Primary, Some(other), "main", "users"),
        ];

        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, profile, "main", "users"),
            Some(1)
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Secondary, profile, "main", "users"),
            Some(2)
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, profile, "main", "missing"),
            None
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, other, "main", "missing"),
            None
        );
    }

    #[tokio::test]
    async fn exports_every_page_with_one_header() {
        let database_path = std::env::temp_dir().join(format!("koyo-export-{}.db", Uuid::new_v4()));
        let csv_path = std::env::temp_dir().join(format!("koyo-export-{}.csv", Uuid::new_v4()));
        let manager = Arc::new(SessionManager::default());
        let profile = ConnectionProfile {
            id: Uuid::new_v4(),
            name: "export test".into(),
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            database: String::new(),
            username: String::new(),
            ssl_mode: SslMode::Disable,
            sqlite_path: database_path.display().to_string(),
        };
        let connection = database::connect(&manager, &profile, "")
            .await
            .expect("connect sqlite");
        database::run_sql(
            &manager,
            connection.session_id,
            Uuid::new_v4(),
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)".into(),
        )
        .await
        .expect("create sqlite table");
        database::run_sql(
            &manager,
            connection.session_id,
            Uuid::new_v4(),
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 250) \
             INSERT INTO people(name) SELECT 'Person ' || n FROM seq"
                .into(),
        )
        .await
        .expect("seed sqlite");

        let (rows, truncated) = export_table_csv(
            manager,
            connection.session_id,
            "main".into(),
            "people".into(),
            1,
            None,
            Vec::new(),
            None,
            csv_path.clone(),
        )
        .await
        .expect("export csv");

        assert_eq!(rows, 250);
        assert!(!truncated);
        let csv = fs::read_to_string(&csv_path).expect("read csv");
        assert_eq!(csv.lines().count(), 251);
        assert_eq!(csv.lines().next(), Some("id,name"));
        assert!(csv.contains("Person 250"));

        let _ = fs::remove_file(database_path);
        let _ = fs::remove_file(csv_path);
    }

    #[test]
    fn delete_supersedes_update_and_toggles() {
        let key = vec![Some("7".to_string())];
        let mut staged = vec![StagedChange::Update {
            key: key.clone(),
            version: "v1".into(),
            values: BTreeMap::from([("name".to_string(), Some("changed".to_string()))]),
            original: vec![Some("7".to_string()), Some("old".to_string())],
            truncated: vec![false, false],
        }];

        assert!(stage_row_delete(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(matches!(staged.as_slice(), [StagedChange::Delete { .. }]));

        assert!(!stage_row_delete(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(staged.is_empty());

        assert!(stage_row_delete_change(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(!stage_row_delete_change(
            &mut staged,
            key,
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(matches!(staged.as_slice(), [StagedChange::Delete { .. }]));
    }
}
