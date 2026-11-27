use super::*;

impl KoyoApp {
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
        if self.tabs.iter().any(|tab| {
            tab.catalog_loading
                && tab
                    .connection
                    .as_ref()
                    .is_some_and(|current| current.session_id == connection.session_id)
        }) {
            // Schema discovery holds the shared session. Its callback starts
            // the focused table page when it finishes.
            return;
        }
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
        let page_limit = table.page_limit;
        let filters_for_task = filters.clone();
        let sort_for_task = sort.clone();
        let count_total = offset == 0 || table.total_rows_cache.is_none();
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        let engine = connection.profile.engine;
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let cancel_manager = manager.clone();
                super::super::connections::run_with_cancel(
                    TABLE_IO_TIMEOUT,
                    "Table load timed out. Cancel was requested.",
                    &cancel_manager,
                    session_id,
                    operation_id,
                    engine,
                    async move {
                        database::load_table_page(
                            &manager,
                            session_id,
                            operation_id,
                            schema,
                            name,
                            offset,
                            page_limit,
                            filters_for_task,
                            sort_for_task,
                            count_total,
                        )
                        .await
                    },
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
                let mut retry_offset = None;
                let error = match result {
                    Ok(page) => {
                        if let Some(total) = page.total_rows {
                            table.total_rows_cache = Some(total);
                        }
                        if page.offset > 0 && page.rows.is_empty() {
                            let limit = page.limit.max(1);
                            let last_offset = match page.total_rows.or(table.total_rows_cache) {
                                Some(0) => 0,
                                Some(total) => {
                                    (total.saturating_sub(1).min(u64::from(u32::MAX)) as u32
                                        / limit)
                                        * limit
                                }
                                None => page.offset.saturating_sub(limit),
                            };
                            if last_offset < page.offset {
                                retry_offset = Some(last_offset);
                            }
                        }
                        table.page = Some(page);
                        table.selected_keys.clear();
                        table.filters = filters.clone();
                        table.sort = sort.clone();
                        None
                    }
                    Err(error) => Some(error),
                };
                if let Some(retry_offset) = retry_offset {
                    this.load_page(tab_id, retry_offset, filters, sort, cx);
                    return;
                }
                if let Some(error) = error {
                    log::error!("table page load failed on session {session_id}: {error}");
                    if database::is_connection_lost(&error) {
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

    /// Load a connected table only when it becomes visible. Reconnecting a
    /// profile can restore many table tabs; firing every page request at the
    /// same session makes all but one fail with "session is busy".
    pub(crate) fn ensure_table_page(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(session_id) = self.tabs[index]
            .connection
            .as_ref()
            .map(|connection| connection.session_id)
        else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.page.is_some() || table.loading {
            return;
        }
        // Catalog loading owns the session for several queries. Waiting for
        // its callback avoids a guaranteed "session is busy" failure.
        if self.tabs.iter().any(|tab| {
            tab.catalog_loading
                && tab
                    .connection
                    .as_ref()
                    .is_some_and(|connection| connection.session_id == session_id)
        }) {
            return;
        }
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        self.load_page(tab_id, 0, filters, sort, cx);
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
        let session_id = connection.session_id;
        if self.session_is_busy(session_id) {
            return;
        }
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.foreign_keys_loading {
            return;
        }
        let schema = table.schema.clone();
        let table_name = table.table.clone();
        table.foreign_keys_loading = true;
        table.diagram_metadata_loaded = true;
        table.diagram_generation = table.diagram_generation.wrapping_add(1);
        let generation = table.diagram_generation;
        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let foreign_keys = tokio::time::timeout(
                    TABLE_IO_TIMEOUT,
                    database::list_foreign_keys(&manager, session_id, &schema, &table_name),
                )
                .await
                .map_err(|_| "Loading diagram metadata timed out.".to_string())??;
                let indexes = tokio::time::timeout(
                    TABLE_IO_TIMEOUT,
                    database::list_indexes(&manager, session_id, &schema, &table_name),
                )
                .await
                .map_err(|_| "Loading diagram metadata timed out.".to_string())??;
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
                let error = if let TabBody::Table(table) = &mut this.tabs[index].body {
                    if table.diagram_generation != generation {
                        return;
                    }
                    table.foreign_keys_loading = false;
                    match result {
                        Ok((foreign_keys, indexes)) => {
                            table.foreign_keys = foreign_keys;
                            table.indexes = indexes;
                            None
                        }
                        Err(error) => {
                            table.diagram_metadata_loaded = false;
                            Some(error)
                        }
                    }
                } else {
                    None
                };
                if let Some(error) = error {
                    if database::is_connection_lost(&error) {
                        this.mark_connection_lost(
                            session_id,
                            "Connection lost while loading diagram metadata. Reconnect to continue.",
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
        if !table.staged.is_empty() {
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
                    let window_handle = this.window_handle;
                    if let Some(filter_input) = this
                        .tab_index(tab_id)
                        .and_then(|index| this.tabs[index].body.table())
                        .map(|table| table.filter_input.clone())
                    {
                        cx.defer(move |cx| {
                            let _ = cx.update_window(window_handle, |_, window, cx| {
                                filter_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                            });
                        });
                    }
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
                    if let Some(index) = this.tab_index(tab_id)
                        && let TabBody::Table(table) = &mut this.tabs[index].body
                    {
                        table.total_rows_cache = None;
                    }
                    this.load_page(tab_id, offset, filters.clone(), sort.clone(), cx);
                },
            );
        } else {
            // Refresh is the user's explicit request for current data.
            table.total_rows_cache = None;
            self.load_page(tab_id, offset, filters, sort, cx);
        }
    }
}
