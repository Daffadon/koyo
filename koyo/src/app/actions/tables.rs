use std::collections::BTreeMap;

use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::database;
use koyo_core::models::{GridChange, TableFilter, TableSort};
use uuid::Uuid;

use crate::app::*;

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
                    tab.expanded.clone(),
                )
            });
        let Some((Some(profile), shared, tables, objects, metadata, expanded)) = source else {
            self.status = "Open a connection before browsing tables.".into();
            cx.notify();
            return;
        };
        if self.focus_existing_table(profile.id, &schema, &table, cx) {
            return;
        }
        let mut tab =
            self.make_table_tab(profile.clone(), schema.clone(), table.clone(), window, cx);
        tab.tables = tables;
        tab.schema_objects = objects;
        tab.metadata_groups = metadata;
        tab.expanded = expanded;
        if let Some(connection) = shared {
            tab.connection = Some(connection);
            tab.connecting = false;
            if let TabBody::Table(table_tab) = &mut tab.body {
                table_tab.loading = false;
            }
            let tab_id = self.add_tab(tab, self.focus);
            self.persist_state(cx);
            cx.notify();
            self.load_page(tab_id, 0, Vec::new(), None, cx);
            return;
        }
        let tab_id = self.add_tab(tab, self.focus);
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
                let tables = database::list_tables(&manager, info.session_id)
                    .await
                    .unwrap_or_default();
                let objects = database::list_schema_objects(&manager, info.session_id)
                    .await
                    .unwrap_or_default();
                let metadata = database::list_database_metadata(&manager, info.session_id)
                    .await
                    .unwrap_or_default();
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
                )
                .await;
                Ok::<_, String>((info, tables, objects, metadata, page))
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    if let Ok((info, _, _, _, _)) = &result {
                        this.disconnect_session(info.session_id);
                    }
                    return;
                };
                match result {
                    Ok((info, tables, objects, metadata, page)) => {
                        let operation_matches = this.tabs[index]
                            .body
                            .table()
                            .is_some_and(|table| table.operation_id == Some(operation_id));
                        if !operation_matches {
                            this.disconnect_session(info.session_id);
                            return;
                        }
                        this.tabs[index].connection = Some(info);
                        this.tabs[index].profile = Some(profile.clone());
                        this.tabs[index].connecting = false;
                        this.tabs[index].tables = tables;
                        this.tabs[index].schema_objects = objects;
                        this.tabs[index].metadata_groups = metadata;
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
                    }
                }
                cx.notify();
            },
        );
    }

    fn focus_existing_table(
        &mut self,
        profile_id: uuid::Uuid,
        schema: &str,
        table: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((pane, tab_id)) = self.tabs.iter().find_map(|tab| {
            if !tab
                .profile
                .as_ref()
                .is_some_and(|profile| profile.id == profile_id)
            {
                return None;
            }
            let TabBody::Table(open) = &tab.body else {
                return None;
            };
            if open.schema != schema || open.table != table {
                return None;
            }
            if self.primary_tabs.contains(&tab.id) {
                Some((Pane::Primary, tab.id))
            } else if self.secondary_tabs.contains(&tab.id) {
                Some((Pane::Secondary, tab.id))
            } else {
                None
            }
        }) else {
            return false;
        };
        let Some(order) = self.pane_tab_ids(pane).iter().position(|id| *id == tab_id) else {
            return false;
        };
        self.switch_tab(pane, order, cx);
        self.status = format!("Focused {schema}.{table}.");
        cx.notify();
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
        cx.notify();

        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                database::load_table_page(
                    &manager,
                    connection.session_id,
                    operation_id,
                    schema,
                    name,
                    offset,
                    PAGE_LIMIT,
                    filters_for_task,
                    sort_for_task,
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
                if table.load_sequence != sequence || table.operation_id != Some(operation_id) {
                    return;
                }
                table.operation_id = None;
                table.loading = false;
                match result {
                    Ok(page) => {
                        table.page = Some(page);
                        table.filters = filters;
                        table.sort = sort;
                    }
                    Err(error) => {
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

    pub(crate) fn apply_changes(&mut self, tab_id: u64, cx: &mut Context<Self>) {
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
                } => GridChange::Update {
                    key,
                    version,
                    values,
                    original,
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
                        this.tabs[index].notice = format!(
                            "{} staged {} applied.",
                            summary.applied,
                            if summary.applied == 1 { "change" } else { "changes" }
                        );
                        this.load_page(tab_id, offset, filters, sort, cx);
                    }
                    Err(error) => {
                        if error.starts_with("COMMIT_OUTCOME_UNKNOWN:") {
                            table.staged.clear();
                            this.tabs[index].notice = "Commit result could not be confirmed. Page reloaded; verify data before retrying."
                                .into();
                            this.load_page(tab_id, offset, filters, sort, cx);
                        } else {
                            this.tabs[index].notice = error;
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

    pub(crate) fn refresh_current_table(
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
        if table.loading {
            return;
        }
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
