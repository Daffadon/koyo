use super::*;

impl KoyoApp {
    pub(crate) fn tab_index(&self, tab_id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == tab_id)
    }

    pub(crate) fn pane_tab_ids(&self, pane: Pane) -> &[u64] {
        match pane {
            Pane::Primary => &self.primary_tabs,
            Pane::Secondary => &self.secondary_tabs,
        }
    }

    pub(crate) fn tab_scroll(&self, pane: Pane) -> &ScrollHandle {
        match pane {
            Pane::Primary => &self.primary_tab_scroll,
            Pane::Secondary => &self.secondary_tab_scroll,
        }
    }

    pub(super) fn reveal_active_tab(&self, pane: Pane) {
        self.tab_scroll(pane)
            .scroll_to_item(self.pane_selected(pane));
    }

    pub(crate) fn pane_selected(&self, pane: Pane) -> usize {
        let active = match pane {
            Pane::Primary => self.primary_active,
            Pane::Secondary => self.secondary_active,
        };
        self.pane_tab_ids(pane)
            .iter()
            .position(|id| *id == active)
            .unwrap_or(0)
    }

    pub(crate) fn pane_active_id(&self, pane: Pane) -> Option<u64> {
        let id = match pane {
            Pane::Primary => self.primary_active,
            Pane::Secondary if self.split.is_some() => self.secondary_active,
            Pane::Secondary => 0,
        };
        (id != 0 && self.tab_index(id).is_some()).then_some(id)
    }

    pub(crate) fn pane_index(&self, pane: Pane) -> Option<usize> {
        self.pane_active_id(pane).and_then(|id| self.tab_index(id))
    }

    pub(crate) fn focused_index(&self) -> Option<usize> {
        self.pane_index(self.focus)
            .or_else(|| self.pane_index(Pane::Primary))
    }

    pub(crate) fn sidebar_index(&self) -> Option<usize> {
        self.sidebar_tab
            .and_then(|id| self.tab_index(id))
            .or_else(|| self.focused_index())
    }

    pub(crate) fn focus_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        if self.pane_index(pane).is_some() && self.focus != pane {
            self.focus = pane;
            self.persist_state(cx);
            cx.notify();
        }
    }

    pub(crate) fn add_tab(&mut self, tab: Tab, pane: Pane) -> u64 {
        let id = tab.id;
        self.tabs.push(tab);
        let pane = if pane == Pane::Secondary && self.split.is_none() {
            Pane::Primary
        } else {
            pane
        };
        match pane {
            Pane::Primary => {
                self.primary_tabs.push(id);
                self.primary_active = id;
            }
            Pane::Secondary => {
                self.secondary_tabs.push(id);
                self.secondary_active = id;
            }
        }
        self.reveal_active_tab(pane);
        let source_live = self
            .sidebar_tab
            .and_then(|tab_id| self.tab_index(tab_id))
            .is_some_and(|index| self.tabs[index].connection.is_some());
        if !source_live {
            self.sidebar_tab = Some(id);
        }
        id
    }

    pub(super) fn duplicate_tab(
        &mut self,
        source_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        enum DuplicateBody {
            Query {
                sql: String,
                starter: SharedString,
                result_view: ResultView,
            },
            Table {
                schema: String,
                table: String,
                mode: TableMode,
                foreign_keys: Vec<koyo_core::models::ForeignKeyInfo>,
                indexes: Vec<koyo_core::models::TableIndexInfo>,
                diagram_metadata_loaded: bool,
                page: Option<koyo_core::models::TablePage>,
                total_rows_cache: Option<u64>,
                filters: Vec<koyo_core::models::TableFilter>,
                filter_text: String,
                sort: Option<koyo_core::models::TableSort>,
                column_widths: Vec<f32>,
            },
        }

        let Some(index) = self.tab_index(source_id) else {
            return self.make_query_tab(None, window, cx);
        };
        let source = &self.tabs[index];
        let profile = source.profile.clone();
        let connection = source.connection.clone();
        let tables = source.tables.clone();
        let schema_objects = source.schema_objects.clone();
        let metadata_groups = source.metadata_groups.clone();
        let columns = source.columns.clone();
        let expanded = source.expanded.clone();
        let title = source.title.clone();
        let body = match &source.body {
            TabBody::Query(query) => DuplicateBody::Query {
                sql: query.editor.read(cx).value().to_string(),
                starter: query.starter.clone(),
                result_view: query.result_view,
            },
            TabBody::Table(table) => DuplicateBody::Table {
                schema: table.schema.clone(),
                table: table.table.clone(),
                mode: table.mode,
                foreign_keys: table.foreign_keys.clone(),
                indexes: table.indexes.clone(),
                diagram_metadata_loaded: table.diagram_metadata_loaded,
                page: table.page.clone(),
                total_rows_cache: table.total_rows_cache,
                filters: table.filters.clone(),
                filter_text: table.filter_input.read(cx).value().to_string(),
                sort: table.sort.clone(),
                column_widths: table.column_widths.clone(),
            },
        };

        let mut tab = match body {
            DuplicateBody::Query {
                sql,
                starter,
                result_view,
            } => {
                let mut tab = self.make_query_tab(profile, window, cx);
                if let TabBody::Query(query) = &mut tab.body {
                    query.editor.update(cx, |editor, cx| {
                        editor.set_value(sql, window, cx);
                    });
                    query.starter = starter;
                    query.result_view = result_view;
                }
                tab
            }
            DuplicateBody::Table {
                schema,
                table,
                mode,
                foreign_keys,
                indexes,
                diagram_metadata_loaded,
                page,
                total_rows_cache,
                filters,
                filter_text,
                sort,
                column_widths,
            } => {
                let Some(profile) = profile else {
                    return self.make_query_tab(None, window, cx);
                };
                let mut tab = self.make_table_tab(profile, schema, table, window, cx);
                if let TabBody::Table(table) = &mut tab.body {
                    table.mode = mode;
                    table.foreign_keys = foreign_keys;
                    table.foreign_keys_loading = false;
                    table.indexes = indexes;
                    table.diagram_metadata_loaded = diagram_metadata_loaded;
                    table.page = page;
                    table.total_rows_cache = total_rows_cache;
                    table.filters = filters;
                    table.filter_input.update(cx, |input, cx| {
                        input.set_value(filter_text, window, cx);
                    });
                    table.sort = sort;
                    table.column_widths = column_widths;
                    table.loading = false;
                }
                tab
            }
        };
        tab.title = title;
        tab.connection = connection;
        tab.tables = tables;
        tab.schema_objects = schema_objects;
        tab.metadata_groups = metadata_groups;
        tab.columns = columns;
        tab.expanded = expanded;
        tab.connecting = false;
        tab.notice.clear();
        tab
    }

    pub(crate) fn toggle_split(
        &mut self,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.split == Some(axis) {
            self.primary_tabs.extend(self.secondary_tabs.drain(..));
            self.secondary_active = 0;
            self.split = None;
            self.focus = Pane::Primary;
            self.persist_state(cx);
            cx.notify();
            return;
        }
        if self.split.is_none() {
            self.split = Some(axis);
            let source_id = self
                .pane_active_id(self.focus)
                .or_else(|| self.pane_active_id(Pane::Primary));
            let tab = match source_id {
                Some(tab_id) => self.duplicate_tab(tab_id, window, cx),
                None => self.make_query_tab(None, window, cx),
            };
            let id = self.add_tab(tab, Pane::Secondary);
            if self.tabs[self.tab_index(id).unwrap()].connection.is_some() {
                self.load_tables(id, cx);
            }
        } else {
            self.split = Some(axis);
        }
        self.focus = Pane::Primary;
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn normalize_panes(&mut self) {
        let valid = self.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
        self.primary_tabs.retain(|id| valid.contains(id));
        self.secondary_tabs.retain(|id| valid.contains(id));
        if !self.primary_tabs.contains(&self.primary_active) {
            self.primary_active = self.primary_tabs.first().copied().unwrap_or(0);
        }
        if self.split.is_none() || self.secondary_tabs.is_empty() {
            self.split = None;
            self.secondary_tabs.clear();
            self.secondary_active = 0;
            self.focus = Pane::Primary;
        } else if !self.secondary_tabs.contains(&self.secondary_active) {
            self.secondary_active = self.secondary_tabs[0];
        }
        if self.focus == Pane::Secondary && self.pane_index(Pane::Secondary).is_none() {
            self.focus = Pane::Primary;
        }
    }

    pub(crate) fn make_query_tab(
        &mut self,
        profile: Option<ConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let engine = profile
            .as_ref()
            .map(|profile| profile.engine)
            .unwrap_or(Engine::Postgres);
        let starter = starter_sql(engine);
        let completion = Rc::new(RefCell::new(CompletionCatalog::default()));
        let editor = cx.new(|cx| {
            let mut editor = gpui_kit::component::input::EditorState::new(window, cx)
                .default_value(starter.clone());
            // The SQL grammar also colors `--` and `/* ... */` comments.
            editor.set_highlighter("sql", cx);
            editor
        });
        let provider: Rc<dyn gpui_kit::base::input::CompletionProvider> =
            Rc::new(SqlCompletionProvider::new(completion.clone()));
        editor.update(cx, |editor, _| {
            let lsp = editor.lsp_mut();
            lsp.completion_provider = Some(provider);
            // Schema-qualified names routinely exceed 240 px; keep the full
            // identifier readable. The popup still clamps to the window edge.
            lsp.completion_menu.max_width = px(640.0);
        });

        let title = query_tab_title(profile.as_ref());
        Tab {
            id,
            title,
            profile,
            connection: None,
            tables: Vec::new(),
            schema_objects: Vec::new(),
            metadata_groups: Vec::new(),
            columns: Vec::new(),
            expanded: BTreeSet::new(),
            notice: String::new(),
            connecting: false,
            catalog_loading: false,
            pending_starter: None,
            body: TabBody::Query(QueryTab {
                editor,
                completion,
                starter: starter.into(),
                output: None,
                active_result: 0,
                result_view: ResultView::Table,
                error: String::new(),
                running: false,
                cancelling: false,
                operation_id: None,
                duration_ms: None,
                results_collapsed: false,
                pending_run: false,
                definition_kind: None,
                result_column_widths: Vec::new(),
                result_resize_pointer_x: None,
                result_resize_column: None,
            }),
        }
    }

    pub(crate) fn make_table_tab(
        &mut self,
        profile: ConnectionProfile,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter: column = value AND column2 contains text")
        });
        Tab {
            id,
            title: format!("{schema}.{table}").into(),
            profile: Some(profile),
            connection: None,
            tables: Vec::new(),
            schema_objects: Vec::new(),
            metadata_groups: Vec::new(),
            columns: Vec::new(),
            expanded: BTreeSet::new(),
            notice: String::new(),
            connecting: true,
            catalog_loading: false,
            pending_starter: None,
            body: TabBody::Table(TableTab {
                schema,
                table,
                mode: TableMode::Data,
                foreign_keys: Vec::new(),
                foreign_keys_loading: false,
                indexes: Vec::new(),
                diagram_metadata_loaded: false,
                diagram_generation: 0,
                page: None,
                total_rows_cache: None,
                filters: Vec::new(),
                sort: None,
                staged: Vec::new(),
                selected_keys: BTreeSet::new(),
                loading: true,
                cancelling: false,
                operation_id: None,
                next_insert_id: 1,
                load_sequence: 0,
                filter_input,
                filter_error: String::new(),
                column_widths: Vec::new(),
                resize_pointer_x: None,
                resize_column: None,
                column_resized: false,
                exporting: false,
            }),
        }
    }

    pub(crate) fn new_sql_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self
            .sidebar_index()
            .and_then(|index| self.tabs.get(index))
            .map(|tab| {
                (
                    tab.profile.clone(),
                    tab.connection.clone(),
                    CatalogSnapshot {
                        tables: tab.tables.clone(),
                        schema_objects: tab.schema_objects.clone(),
                        metadata_groups: tab.metadata_groups.clone(),
                        columns: tab.columns.clone(),
                    },
                )
            });
        let (profile, mut shared, mut catalog) = source.unwrap_or_default();
        if shared.is_none()
            && let Some(profile) = profile.clone()
        {
            if let Some((connection, live_catalog)) = self.connection_for_profile(&profile) {
                shared = Some(connection);
                catalog = live_catalog;
            }
        }
        let waiting = shared.is_none()
            && profile
                .as_ref()
                .is_some_and(|profile| self.profile_is_connecting(profile));
        let mut tab = self.make_query_tab(profile.clone(), window, cx);
        if let Some(connection) = shared.clone() {
            tab.connection = Some(connection);
            tab.connecting = false;
            tab.tables = catalog.tables.clone();
            tab.schema_objects = catalog.schema_objects.clone();
            tab.metadata_groups = catalog.metadata_groups.clone();
            tab.columns = catalog.columns.clone();
            if let TabBody::Query(query) = &mut tab.body {
                query.completion.borrow_mut().replace(
                    &catalog.tables,
                    &catalog.schema_objects,
                    &catalog.columns,
                );
            }
        } else if waiting {
            tab.notice = "Waiting for the connection to finish.".into();
        }
        let tab_id = self.add_tab(tab, self.focus);
        if let Some(connection) = shared {
            let loading = self.tabs.iter().any(|tab| {
                tab.catalog_loading
                    && tab
                        .connection
                        .as_ref()
                        .is_some_and(|current| current.session_id == connection.session_id)
            });
            if !loading && catalog.tables.is_empty() {
                self.load_tables(tab_id, cx);
            }
        } else if let Some(profile) = profile
            && !waiting
        {
            self.connect_profile(profile.id, Some(tab_id), false, window, cx);
        }
        self.focus_query_editor(tab_id, window, cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.pane_active_id(self.focus) else {
            return;
        };
        self.close_tab(tab_id, window, cx);
    }

    pub(crate) fn close_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let session_id = self.tabs[index]
            .connection
            .as_ref()
            .map(|connection| connection.session_id);
        let last_session_tab =
            session_id.is_some_and(|session_id| !self.session_in_use(session_id, Some(tab_id)));
        let open_transaction =
            session_id.is_some_and(|session_id| self.session_transaction_open(session_id));
        let needs_confirm = (open_transaction && last_session_tab)
            || match &self.tabs[index].body {
                TabBody::Query(query) => query.running || query_has_unsaved_sql(query, cx),
                TabBody::Table(table) => !table.staged.is_empty() || table.exporting,
            };
        if needs_confirm {
            self.confirm(
                window,
                cx,
                "Close tab?",
                "The editor has SQL, a query or export is running, a transaction is open, or edits are staged. Closing discards them.",
                move |this, cx| this.close_tab_now(tab_id, cx),
            );
        } else {
            self.close_tab_now(tab_id, cx);
            self.focus_active_query_editor(window, cx);
        }
    }

    pub(crate) fn close_tab_now(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let Some(connection) = self.tabs[index].connection.clone()
            && !self.session_in_use(connection.session_id, Some(tab_id))
        {
            let busy = self.session_transaction_open(connection.session_id)
                || match &self.tabs[index].body {
                    TabBody::Query(query) => query.running,
                    TabBody::Table(table) => table.loading || table.exporting,
                };
            if busy || self.idle_pool_size == 0 {
                self.disconnect_session(connection.session_id);
            } else {
                let profile = self.tabs[index]
                    .profile
                    .clone()
                    .unwrap_or_else(|| connection.profile.clone());
                let pooled_session_id = connection.session_id;
                // Idle connections are pooled for reuse. Keep one per profile
                // so session state cannot leak between two profiles, and cap
                // the pool so closed tabs cannot leave hidden sessions forever.
                let previous = std::mem::take(&mut self.parked_connections);
                for parked in previous {
                    if parked.profile.id == profile.id {
                        self.disconnect_session(parked.connection.session_id);
                    } else {
                        self.parked_connections.push(parked);
                    }
                }
                self.parked_connections.push(LiveConnection {
                    profile,
                    connection,
                    tables: self.tabs[index].tables.clone(),
                    schema_objects: self.tabs[index].schema_objects.clone(),
                    metadata_groups: self.tabs[index].metadata_groups.clone(),
                    columns: self.tabs[index].columns.clone(),
                    parked_at: std::time::Instant::now(),
                });
                let limit = self.idle_pool_size.min(MAX_IDLE_POOL_SIZE);
                while self.parked_connections.len() > limit {
                    let evicted = self.parked_connections.remove(0);
                    self.disconnect_session(evicted.connection.session_id);
                }
                self.spawn_pool_probe(pooled_session_id, cx);
            }
        }
        self.tabs[index].connection = None;
        if self
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.tab_id == tab_id)
        {
            self.cell_edit = None;
        }
        let pane = if self.primary_tabs.contains(&tab_id) {
            Some(Pane::Primary)
        } else if self.secondary_tabs.contains(&tab_id) {
            Some(Pane::Secondary)
        } else {
            None
        };
        if let Some(pane) = pane {
            let ids = match pane {
                Pane::Primary => &mut self.primary_tabs,
                Pane::Secondary => &mut self.secondary_tabs,
            };
            let closed_position = ids.iter().position(|id| *id == tab_id).unwrap_or(0);
            ids.retain(|id| *id != tab_id);
            let active = match pane {
                Pane::Primary => &mut self.primary_active,
                Pane::Secondary => &mut self.secondary_active,
            };
            if *active == tab_id {
                *active = neighbor_after_close(ids, closed_position);
            }
        }
        self.tabs.remove(index);
        self.normalize_panes();
        if self.sidebar_tab == Some(tab_id) {
            self.sidebar_tab = self
                .pane_active_id(self.focus)
                .or_else(|| self.tabs.first().map(|tab| tab.id));
        }
        if let Some(pane) = pane {
            self.reveal_active_tab(pane);
        }
        if let Some(active) = self.pane_active_id(self.focus) {
            self.auto_connect_tab(active, cx);
        }
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn reorder_tab(
        &mut self,
        tab_id: u64,
        target_id: u64,
        pane: Pane,
        cx: &mut Context<Self>,
    ) {
        if tab_id == target_id || (pane == Pane::Secondary && self.split.is_none()) {
            return;
        }
        let source = if self.primary_tabs.contains(&tab_id) {
            Some(Pane::Primary)
        } else if self.secondary_tabs.contains(&tab_id) {
            Some(Pane::Secondary)
        } else {
            None
        };
        let Some(source) = source else {
            return;
        };
        let Some(mut target_index) = self
            .pane_tab_ids(pane)
            .iter()
            .position(|id| *id == target_id)
        else {
            return;
        };
        match source {
            Pane::Primary => {
                self.primary_tabs.retain(|id| *id != tab_id);
                if self.primary_active == tab_id {
                    self.primary_active = self.primary_tabs.first().copied().unwrap_or(0);
                }
            }
            Pane::Secondary => {
                self.secondary_tabs.retain(|id| *id != tab_id);
                if self.secondary_active == tab_id {
                    self.secondary_active = self.secondary_tabs.first().copied().unwrap_or(0);
                }
            }
        }
        if source == pane
            && let Some(index) = self
                .pane_tab_ids(pane)
                .iter()
                .position(|id| *id == target_id)
        {
            target_index = index;
        }
        match pane {
            Pane::Primary => {
                self.primary_tabs.insert(target_index, tab_id);
                self.primary_active = tab_id;
            }
            Pane::Secondary => {
                self.secondary_tabs.insert(target_index, tab_id);
                self.secondary_active = tab_id;
            }
        }
        self.focus = pane;
        self.normalize_panes();
        self.reveal_active_tab(pane);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn move_active_tab(&mut self, direction: isize, cx: &mut Context<Self>) {
        let pane = self.focus;
        let Some(active) = self.pane_active_id(pane) else {
            return;
        };
        let tabs = match pane {
            Pane::Primary => &mut self.primary_tabs,
            Pane::Secondary => &mut self.secondary_tabs,
        };
        if !move_tab_id(tabs, active, direction) {
            return;
        }
        self.reveal_active_tab(pane);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn move_tab_to_pane(&mut self, tab_id: u64, target: Pane, cx: &mut Context<Self>) {
        if target == Pane::Secondary && self.split.is_none() {
            return;
        }
        let source = if self.primary_tabs.contains(&tab_id) {
            Some(Pane::Primary)
        } else if self.secondary_tabs.contains(&tab_id) {
            Some(Pane::Secondary)
        } else {
            None
        };
        let Some(source) = source else {
            return;
        };
        if source == target {
            self.focus = target;
            cx.notify();
            return;
        }
        match source {
            Pane::Primary => {
                self.primary_tabs.retain(|id| *id != tab_id);
                if self.primary_active == tab_id {
                    self.primary_active = self.primary_tabs.first().copied().unwrap_or(0);
                }
            }
            Pane::Secondary => {
                self.secondary_tabs.retain(|id| *id != tab_id);
                if self.secondary_active == tab_id {
                    self.secondary_active = self.secondary_tabs.first().copied().unwrap_or(0);
                }
            }
        }
        match target {
            Pane::Primary => {
                self.primary_tabs.push(tab_id);
                self.primary_active = tab_id;
            }
            Pane::Secondary => {
                self.secondary_tabs.push(tab_id);
                self.secondary_active = tab_id;
            }
        }
        self.focus = target;
        self.normalize_panes();
        self.reveal_active_tab(target);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn switch_tab(
        &mut self,
        pane: Pane,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.pane_tab_ids(pane).get(index).copied() else {
            return;
        };
        match pane {
            Pane::Primary => self.primary_active = id,
            Pane::Secondary => self.secondary_active = id,
        }
        self.focus = pane;
        self.reveal_active_tab(pane);
        self.normalize_panes();
        self.auto_connect_tab(id, cx);
        self.focus_query_editor(id, window, cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn cycle_tab(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.pane_tab_ids(self.focus).len();
        if count < 2 {
            return;
        }
        let current = self.pane_selected(self.focus);
        let next = if direction >= 0 {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.switch_tab(self.focus, next, window, cx);
    }

    pub(crate) fn focus_query_editor(
        &self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &self.tabs[index].body else {
            return;
        };
        query.editor.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub(super) fn focus_active_query_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.pane_active_id(self.focus) {
            self.focus_query_editor(tab_id, window, cx);
        }
    }

    /// A disconnected tab adopts a live session for its profile. One
    /// reconnect serves every pane and the shared explorer.
    pub(crate) fn auto_connect_tab(&mut self, tab_id: u64, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        let tab = &self.tabs[index];
        if tab.connection.is_some() || tab.connecting {
            return false;
        }
        let Some(profile) = tab.profile.clone() else {
            return false;
        };
        let Some((connection, catalog)) = self.connection_for_profile(&profile) else {
            return false;
        };
        let session_id = connection.session_id;
        let empty_catalog = catalog.tables.is_empty();
        let attached = self.attach_connection_to_tab(tab_id, connection, catalog, cx);
        if attached && empty_catalog {
            let loading = self.tabs.iter().any(|tab| {
                tab.catalog_loading
                    && tab
                        .connection
                        .as_ref()
                        .is_some_and(|current| current.session_id == session_id)
            });
            if !loading {
                self.load_tables(tab_id, cx);
            }
        }
        attached
    }

    /// Attach a newly connected session to every disconnected tab that expects
    /// the same profile. The source catalog is copied so reconnecting does not
    /// fetch the schema once per tab.
    pub(crate) fn apply_pending_starters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            if let Some(text) = tab.pending_starter.take() {
                if let TabBody::Query(query) = &mut tab.body {
                    // Replacing a user's SQL on first connect lost work. Only
                    // replace text still equal to the untouched starter.
                    if query.editor.read(cx).value() == query.starter {
                        query.starter = text.clone();
                        query.editor.update(cx, |editor, cx| {
                            editor.set_value(text.clone(), window, cx);
                        });
                    }
                }
            }
        }
    }

    pub(crate) fn attach_profile_session(
        &mut self,
        profile_id: Uuid,
        source_tab_id: u64,
        cx: &mut Context<Self>,
    ) -> usize {
        let Some(source_index) = self.tab_index(source_tab_id) else {
            return 0;
        };
        let Some(connection) = self.tabs[source_index].connection.clone() else {
            return 0;
        };
        let catalog = self.tab_catalog(source_index);
        let tabs = self
            .tabs
            .iter()
            .filter(|tab| {
                tab.id != source_tab_id
                    && tab.connection.is_none()
                    && !tab.connecting
                    && tab.profile.as_ref().is_some_and(|profile| {
                        profile.id == profile_id
                            && same_connection_target(profile, &connection.profile)
                    })
            })
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        tabs.into_iter()
            .filter(|tab_id| {
                self.attach_connection_to_tab(*tab_id, connection.clone(), catalog.clone(), cx)
            })
            .count()
    }
}
