use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{Enter, IndentInline, InputState, TextareaState};
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::database;
use koyo_core::models::{ConnectionProfile, Engine};
use koyo_core::{ProfileStore, SessionManager};
use tokio::runtime::Runtime;
use uuid::Uuid;

use super::ai::restore_ai_threads;
use crate::app::completion::{CompletionCatalog, SqlCompletionProvider};
use crate::app::*;

impl KoyoApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, runtime: Arc<Runtime>) -> Self {
        let store = ProfileStore::from_default_dir();
        let settings = SettingsStore::from_default_dir();
        let (saved, state_warning, persistence_error) = match settings.load() {
            Ok(loaded) => (loaded.state, loaded.warning, None),
            Err(error) => (PersistedState::default(), Some(error.clone()), Some(error)),
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
            Err(error) => (Vec::new(), error),
        };
        if let Some(warning) = store.take_migration_warning() {
            status = warning;
        }
        if let Some(warning) = state_warning {
            status = warning;
        }
        let ai_config_json = saved.ai_config_json.clone();
        let ai_config_error = AiConfig::parse(&ai_config_json).err().unwrap_or_default();
        let ai_model = AiConfig::parse(&ai_config_json)
            .map(|config| config.model)
            .unwrap_or_else(|_| "Not configured".into());
        let (ai_threads, ai_active_thread, next_ai_thread_id) = restore_ai_threads(&saved);
        let save_queue = Arc::new(StateSaveQueue::default());
        let (save_error_tx, mut save_error_rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_state_save_worker(save_queue.clone(), settings.clone(), save_error_tx);
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
        let mut app = Self {
            store,
            save_queue,
            history: saved.history.clone(),
            allow_close: false,
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
            cell_edit: None,
            confirm: None,
            export: None,
            next_tab_id: 1,
        };
        cx.spawn(async move |this, cx| {
            while let Some(error) = save_error_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.status = error;
                    cx.notify();
                });
            }
        })
        .detach();
        if saved.theme == "light" {
            Theme::change(ThemeMode::Light, None, cx);
            apply_calm_light_theme(cx);
        }
        app.restore_workspace(&saved.workspace, window, cx);
        app.ensure_workspace_tab(window, cx);

        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let mut allow = true;
            let _ = weak.update(cx, |app, cx| {
                if app.allow_close {
                    return;
                }
                app.persist_state_sync(cx);
                if app.has_unsaved_work(cx) {
                    allow = false;
                    app.confirm_close(cx);
                }
            });
            allow
        });
        cx.observe(&app.explorer_filter, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&app.profile_filter, |_, _, cx| cx.notify())
            .detach();
        app
    }

    pub(crate) fn ensure_workspace_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.primary_tabs.is_empty() {
            return;
        }
        let parked = self.parked_connections.pop();
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

    fn restore_workspace(
        &mut self,
        snapshot: &WorkspaceSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split = match snapshot.split.as_deref() {
            Some("vertical") => Some(SplitAxis::Vertical),
            Some("horizontal") => Some(SplitAxis::Horizontal),
            _ => None,
        };
        if !snapshot.secondary.is_empty() && self.split.is_none() {
            self.split = Some(SplitAxis::Vertical);
        }
        if snapshot.secondary.is_empty() {
            self.split = None;
        }

        let mut primary_ids = Vec::new();
        let mut secondary_ids = Vec::new();
        for (pane, tabs, ids) in [
            (Pane::Primary, &snapshot.primary, &mut primary_ids),
            (Pane::Secondary, &snapshot.secondary, &mut secondary_ids),
        ] {
            if pane == Pane::Secondary && self.split.is_none() {
                continue;
            }
            for saved_tab in tabs {
                let profile = saved_tab.profile_id().and_then(|id| {
                    self.profiles
                        .iter()
                        .find(|profile| profile.id.to_string() == id)
                        .cloned()
                });
                match saved_tab {
                    TabSnapshot::Query { sql, .. } => {
                        let mut tab = self.make_query_tab(profile, window, cx);
                        tab.auto_connect = true;
                        if let TabBody::Query(query) = &mut tab.body {
                            query.editor.update(cx, |editor, cx| {
                                editor.set_value(sql.clone(), window, cx);
                            });
                            // Restored SQL is not a starter; connection setup
                            // must not overwrite it.
                            query.starter = String::new().into();
                        }
                        let id = self.add_tab(tab, pane);
                        ids.push(id);
                    }
                    TabSnapshot::Table { schema, table, .. } => {
                        let Some(profile) = profile else {
                            continue;
                        };
                        let mut tab =
                            self.make_table_tab(profile, schema.clone(), table.clone(), window, cx);
                        tab.auto_connect = true;
                        tab.connecting = false;
                        tab.notice = "Restored tab. Reconnect to load rows.".into();
                        if let TabBody::Table(table) = &mut tab.body {
                            table.loading = false;
                        }
                        let id = self.add_tab(tab, pane);
                        ids.push(id);
                    }
                }
            }
        }

        if primary_ids.is_empty() && secondary_ids.is_empty() {
            return;
        }
        self.primary_active = primary_ids
            .get(snapshot.primary_active)
            .copied()
            .or_else(|| primary_ids.first().copied())
            .unwrap_or(0);
        self.secondary_active = secondary_ids
            .get(snapshot.secondary_active)
            .copied()
            .or_else(|| secondary_ids.first().copied())
            .unwrap_or(0);
        self.focus = if snapshot.focus_secondary && self.split.is_some() {
            Pane::Secondary
        } else {
            Pane::Primary
        };
        self.normalize_panes();
        self.sidebar_tab = self
            .pane_active_id(self.focus)
            .or_else(|| self.primary_tabs.first().copied())
            .or_else(|| self.secondary_tabs.first().copied());
    }

    pub(crate) fn persist_state(&mut self, cx: &App) {
        if self.persistence_disabled {
            return;
        }
        let state = self.persisted_state(cx);
        self.save_queue.submit(state);
    }

    pub(crate) fn persist_state_sync(&mut self, cx: &App) {
        if self.persistence_disabled {
            return;
        }
        let state = self.persisted_state(cx);
        if let Err(error) = self.save_queue.flush(state) {
            self.status = error;
        }
    }

    fn persisted_state(&mut self, cx: &App) -> PersistedState {
        PersistedState {
            theme: if cx.theme().is_dark() {
                "dark"
            } else {
                "light"
            }
            .into(),
            sidebar_open: self.sidebar_open,
            sidebar_width: self.sidebar_width,
            ai_open: self.ai_open,
            ai_width: self.ai_width,
            ai_config_json: self.ai_config_json.clone(),
            ai_messages: Vec::new(),
            ai_threads: self.ai_threads_snapshot(),
            ai_active_thread: self.ai_active_thread,
            next_ai_thread_id: self.next_ai_thread_id,
            split: self.split.map(|axis| {
                match axis {
                    SplitAxis::Vertical => "vertical",
                    SplitAxis::Horizontal => "horizontal",
                }
                .to_string()
            }),
            split_ratio: self.split_ratio,
            history: self.history.clone(),
            workspace: self.workspace_snapshot(cx),
        }
    }

    fn workspace_snapshot(&self, cx: &App) -> WorkspaceSnapshot {
        let snapshot = |tab: &Tab| -> Option<TabSnapshot> {
            let profile_id = tab.profile.as_ref().map(|profile| profile.id.to_string());
            match &tab.body {
                TabBody::Query(query) => Some(TabSnapshot::Query {
                    profile_id,
                    sql: query.editor.read(cx).value().to_string(),
                }),
                TabBody::Table(table) => Some(TabSnapshot::Table {
                    profile_id,
                    schema: table.schema.clone(),
                    table: table.table.clone(),
                }),
            }
        };
        let primary = self
            .primary_tabs
            .iter()
            .filter_map(|id| self.tab_index(*id))
            .filter_map(|index| snapshot(&self.tabs[index]))
            .collect::<Vec<_>>();
        let secondary = self
            .secondary_tabs
            .iter()
            .filter_map(|id| self.tab_index(*id))
            .filter_map(|index| snapshot(&self.tabs[index]))
            .collect::<Vec<_>>();
        WorkspaceSnapshot {
            split: self.split.map(|axis| {
                match axis {
                    SplitAxis::Vertical => "vertical",
                    SplitAxis::Horizontal => "horizontal",
                }
                .to_string()
            }),
            primary_active: self
                .primary_tabs
                .iter()
                .position(|id| *id == self.primary_active)
                .unwrap_or(0),
            secondary_active: self
                .secondary_tabs
                .iter()
                .position(|id| *id == self.secondary_active)
                .unwrap_or(0),
            primary,
            secondary,
            focus_secondary: self.focus == Pane::Secondary,
        }
    }

    pub(crate) fn has_unsaved_work(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| match &tab.body {
            TabBody::Query(query) => query.running || query_has_unsaved_sql(query, cx),
            TabBody::Table(table) => !table.staged.is_empty(),
        })
    }

    pub(crate) fn confirm_close(&mut self, cx: &mut Context<Self>) {
        self.confirm = Some(ConfirmState {
            title: "Close Koyo?".into(),
            description: "A query is running, a SQL editor has text, or edits are staged. Running queries stop and staged edits are discarded; saved workspace SQL is restored when possible.".into(),
            action: std::rc::Rc::new(|_, _| {}),
            close_window: true,
        });
        cx.notify();
    }

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

    fn reveal_active_tab(&self, pane: Pane) {
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

    pub(crate) fn set_split_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        if self.split.is_some() {
            let ratio = ratio.clamp(0.15, 0.85);
            if (self.split_ratio - ratio).abs() > 0.001 {
                self.split_ratio = ratio;
                cx.notify();
            }
        }
    }

    pub(crate) fn accept_completion_or_indent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.focused_index().and_then(|index| {
            self.tabs.get(index).and_then(|tab| match &tab.body {
                TabBody::Query(query) => Some(query.editor.clone()),
                TabBody::Table(_) => None,
            })
        }) else {
            return;
        };
        let (completion_open, handle) = {
            let editor = editor.read(cx);
            (
                editor.completion_menu_state().open,
                editor.focus_handle(cx).clone(),
            )
        };
        if completion_open {
            handle.dispatch_action(
                &Enter {
                    secondary: false,
                    shift: false,
                },
                window,
                cx,
            );
        } else {
            handle.dispatch_action(&IndentInline, window, cx);
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

    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(next, None, cx);
        if !next.is_dark() {
            apply_calm_light_theme(cx);
        }
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.persist_state(cx);
        cx.notify();
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

    fn duplicate_tab(
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
            // Keep the popup from covering the neighboring split pane.
            lsp.completion_menu.max_width = px(240.0);
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
            expanded: BTreeSet::new(),
            notice: String::new(),
            connecting: false,
            catalog_loading: false,
            auto_connect: false,
            catalog_generation: 0,
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
                result_column_widths: Vec::new(),
                result_resize_pointer_x: None,
                result_resize_column: None,
            }),
        }
    }

    pub(crate) fn begin_column_resize(
        &mut self,
        tab_id: u64,
        column: usize,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = table
            .page
            .as_ref()
            .map(|page| page.columns.len())
            .unwrap_or(0);
        if table.column_widths.len() < column_count {
            table
                .column_widths
                .resize(column_count, DEFAULT_COLUMN_WIDTH);
        }
        if column < column_count {
            table.resize_pointer_x = Some(pointer_x);
            table.resize_column = Some(column);
        }
        cx.notify();
    }

    pub(crate) fn end_column_resize(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.resize_pointer_x = None;
            table.resize_column = None;
        }
        cx.notify();
    }

    pub(crate) fn take_column_resized(&mut self, tab_id: u64) -> bool {
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return false;
        };
        let resized = table.column_resized;
        table.column_resized = false;
        resized
    }

    pub(crate) fn resize_column(
        &mut self,
        tab_id: u64,
        column: usize,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.resize_column != Some(column) {
            return;
        }
        let Some(previous_x) = table.resize_pointer_x else {
            return;
        };
        let Some(width) = table.column_widths.get_mut(column) else {
            return;
        };
        let delta = pointer_x - previous_x;
        if delta.abs() > 0.5 {
            table.column_resized = true;
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        table.resize_pointer_x = Some(pointer_x);
        cx.notify();
    }

    pub(crate) fn nudge_column_width(
        &mut self,
        tab_id: u64,
        column: usize,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = table
            .page
            .as_ref()
            .map(|page| page.columns.len())
            .unwrap_or(0);
        if table.column_widths.len() < column_count {
            table
                .column_widths
                .resize(column_count, DEFAULT_COLUMN_WIDTH);
        }
        if let Some(width) = table.column_widths.get_mut(column) {
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        cx.notify();
    }

    pub(crate) fn nudge_result_column_width(
        &mut self,
        tab_id: u64,
        result_index: usize,
        column: usize,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = query
            .output
            .as_ref()
            .and_then(|output| output.result_sets.get(result_index))
            .map(|result| result.columns.len())
            .unwrap_or(0);
        if query.result_column_widths.len() <= result_index {
            query
                .result_column_widths
                .resize(result_index + 1, Vec::new());
        }
        let widths = &mut query.result_column_widths[result_index];
        if widths.len() < column_count {
            widths.resize(column_count, RESULT_COLUMN_WIDTH);
        }
        if let Some(width) = widths.get_mut(column) {
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        cx.notify();
    }

    pub(crate) fn autosize_column(&mut self, tab_id: u64, column: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        let mut longest = info
            .name
            .chars()
            .count()
            .max(info.data_type.chars().count());
        for row in &page.rows {
            if let Some(Some(value)) = row.values.get(column) {
                longest = longest.max(value.chars().count());
            }
        }
        // Monospace text at text_sm is about 7.2 px per glyph; include the
        // cell's horizontal padding and the sort affordance.
        let width = if page.rows.is_empty() {
            DEFAULT_COLUMN_WIDTH
        } else {
            (longest as f32 * 7.2 + 44.0).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
        };
        if table.column_widths.len() < page.columns.len() {
            table
                .column_widths
                .resize(page.columns.len(), DEFAULT_COLUMN_WIDTH);
        }
        if let Some(current) = table.column_widths.get_mut(column) {
            *current = width;
        }
        table.resize_pointer_x = None;
        table.resize_column = None;
        cx.notify();
    }

    pub(crate) fn resize_active_column(
        &mut self,
        tab_id: u64,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(column) = self.tabs[index]
            .body
            .table()
            .and_then(|table| table.resize_column)
        else {
            return;
        };
        self.resize_column(tab_id, column, pointer_x, cx);
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
            expanded: BTreeSet::new(),
            notice: String::new(),
            connecting: true,
            catalog_loading: false,
            auto_connect: false,
            catalog_generation: 0,
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

    pub(crate) fn spawn_task<T, F>(
        &self,
        cx: &mut Context<Self>,
        tab_id: u64,
        task: impl Future<Output = T> + Send + 'static,
        done: F,
    ) where
        T: Send + 'static,
        F: FnOnce(&mut Self, &mut Context<Self>, T) + 'static,
    {
        let handle = self.runtime.spawn(task);
        cx.spawn(async move |this, cx| match handle.await {
            Ok(value) => {
                let _ = this.update(cx, |this, cx| done(this, cx, value));
            }
            Err(error) => {
                let message = format!("Background task failed unexpectedly: {error}");
                let _ = this.update(cx, |this, cx| this.fail_task(tab_id, message, cx));
            }
        })
        .detach();
    }

    pub(crate) fn fail_task(&mut self, tab_id: u64, error: String, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.busy = false;
                dialog.error = error.clone();
            }
            self.status = error;
            cx.notify();
            return;
        };
        self.tabs[index].connecting = false;
        self.tabs[index].notice = error.clone();
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.running = false;
                query.cancelling = false;
                query.operation_id = None;
                query.pending_run = false;
                query.error = error.clone();
            }
            TabBody::Table(table) => {
                table.loading = false;
                table.cancelling = false;
                table.operation_id = None;
                table.foreign_keys_loading = false;
                table.exporting = false;
            }
        }
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.busy = false;
            dialog.error = error.clone();
        }
        self.status = error;
        cx.notify();
    }

    pub(crate) fn disconnect_session(&self, session_id: Uuid) {
        let manager = self.manager.clone();
        self.runtime.spawn(async move {
            let _ = database::disconnect(&manager, session_id).await;
        });
    }

    /// Split panes share one live session. Only the last tab holding a
    /// session may tear it down.
    fn session_is_used(
        connections: impl IntoIterator<Item = (u64, Option<Uuid>)>,
        session_id: Uuid,
        except_tab: Option<u64>,
    ) -> bool {
        connections.into_iter().any(|(tab_id, connection)| {
            Some(tab_id) != except_tab && connection == Some(session_id)
        })
    }

    fn session_in_use(&self, session_id: Uuid, except_tab: Option<u64>) -> bool {
        Self::session_is_used(
            self.tabs.iter().map(|tab| {
                (
                    tab.id,
                    tab.connection
                        .as_ref()
                        .map(|connection| connection.session_id),
                )
            }),
            session_id,
            except_tab,
        )
    }

    pub(crate) fn disconnect_session_if_unused(&self, session_id: Uuid, except_tab: Option<u64>) {
        if !self.session_in_use(session_id, except_tab) {
            self.disconnect_session(session_id);
        }
    }

    /// An engine-reported dead session. Stop pending work, mark every tab on
    /// that session disconnected, and keep local staged edits for a retry.
    pub(crate) fn mark_connection_lost(
        &mut self,
        session_id: Uuid,
        notice: &str,
        cx: &mut Context<Self>,
    ) {
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
        self.parked_connections
            .retain(|parked| parked.connection.session_id != session_id);
        for tab_id in ids {
            let Some(index) = self.tab_index(tab_id) else {
                continue;
            };
            self.tabs[index].connection = None;
            self.tabs[index].connecting = false;
            self.tabs[index].catalog_loading = false;
            self.tabs[index].auto_connect = false;
            self.tabs[index].notice = notice.to_string();
            match &mut self.tabs[index].body {
                TabBody::Query(query) => {
                    query.running = false;
                    query.cancelling = false;
                    query.operation_id = None;
                    query.pending_run = false;
                }
                TabBody::Table(table) => {
                    table.loading = false;
                    table.cancelling = false;
                    table.operation_id = None;
                    table.foreign_keys_loading = false;
                    table.exporting = false;
                }
            }
        }
        self.disconnect_session(session_id);
        self.status = notice.to_string();
        cx.notify();
    }

    pub(crate) fn disconnect_tabs_for_session(
        &mut self,
        session_id: Uuid,
        notice: &str,
        cx: &mut Context<Self>,
    ) {
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
        self.parked_connections
            .retain(|parked| parked.connection.session_id != session_id);
        for tab_id in &ids {
            self.disconnect_now(*tab_id, cx);
        }
        for tab_id in ids {
            if let Some(index) = self.tab_index(tab_id) {
                self.tabs[index].notice = notice.to_string();
            }
        }
        self.status = notice.to_string();
        cx.notify();
    }

    pub(crate) fn disconnect_now(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let Some(connection) = self.tabs[index].connection.take() {
            if !self.session_in_use(connection.session_id, Some(tab_id)) {
                self.parked_connections
                    .retain(|parked| parked.connection.session_id != connection.session_id);
                self.disconnect_session(connection.session_id);
            }
        }
        self.tabs[index].connecting = false;
        self.tabs[index].auto_connect = false;
        self.tabs[index].notice = "Disconnected. Reconnect when needed.".into();
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.running = false;
                query.cancelling = false;
                query.operation_id = None;
            }
            TabBody::Table(table) => {
                table.loading = false;
                table.cancelling = false;
                table.operation_id = None;
                table.staged.clear();
                table.selected_keys.clear();
            }
        }
        self.status = "Disconnected.".into();
        cx.notify();
    }

    pub(crate) fn request_disconnect(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let needs_confirm = match &self.tabs[index].body {
            TabBody::Query(query) => query.running,
            TabBody::Table(table) => !table.staged.is_empty(),
        };
        if needs_confirm {
            self.confirm(
                window,
                cx,
                "Disconnect?",
                "A query is running or edits are staged. Disconnecting discards them.",
                move |this, cx| this.disconnect_now(tab_id, cx),
            );
        } else {
            self.disconnect_now(tab_id, cx);
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
                    tab.tables.clone(),
                    tab.schema_objects.clone(),
                    tab.metadata_groups.clone(),
                )
            });
        let (profile, shared, tables, objects, metadata) = source.unwrap_or_default();
        let shared_connection = shared.is_some();
        let mut tab = self.make_query_tab(profile.clone(), window, cx);
        if let Some(connection) = shared {
            tab.connection = Some(connection);
            tab.connecting = false;
            tab.tables = tables;
            tab.schema_objects = objects;
            tab.metadata_groups = metadata;
        }
        let tab_id = self.add_tab(tab, self.focus);
        if shared_connection {
            self.load_tables(tab_id, cx);
        }
        if profile.is_some()
            && self.tabs[self.tab_index(tab_id).unwrap()]
                .connection
                .is_none()
        {
            self.connect_profile(profile.unwrap().id, Some(tab_id), false, window, cx);
        }
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn close_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let needs_confirm = match &self.tabs[index].body {
            TabBody::Query(query) => query.running || query_has_unsaved_sql(query, cx),
            TabBody::Table(table) => !table.staged.is_empty(),
        };
        if needs_confirm {
            self.confirm(
                window,
                cx,
                "Close tab?",
                "The editor has SQL or a query is running or edits are staged. Closing discards them.",
                move |this, cx| this.close_tab_now(tab_id, cx),
            );
        } else {
            self.close_tab_now(tab_id, cx);
        }
    }

    pub(crate) fn close_tab_now(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let Some(connection) = self.tabs[index].connection.clone()
            && !self.session_in_use(connection.session_id, Some(tab_id))
        {
            let busy = match &self.tabs[index].body {
                TabBody::Query(query) => query.running,
                TabBody::Table(table) => table.loading,
            };
            if busy {
                self.disconnect_session(connection.session_id);
            } else {
                let profile = self.tabs[index]
                    .profile
                    .clone()
                    .unwrap_or_else(|| connection.profile.clone());
                // Only one idle session is reusable. Disconnect any older
                // parked connection instead of keeping invisible sessions.
                let previous = std::mem::take(&mut self.parked_connections);
                for parked in previous {
                    self.disconnect_session(parked.connection.session_id);
                }
                self.parked_connections.push(LiveConnection {
                    profile,
                    connection,
                    tables: self.tabs[index].tables.clone(),
                    schema_objects: self.tabs[index].schema_objects.clone(),
                    metadata_groups: self.tabs[index].metadata_groups.clone(),
                });
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
            ids.retain(|id| *id != tab_id);
            let active = match pane {
                Pane::Primary => &mut self.primary_active,
                Pane::Secondary => &mut self.secondary_active,
            };
            if *active == tab_id {
                *active = ids.first().copied().unwrap_or(0);
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
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn confirm(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        title: &'static str,
        description: &'static str,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) {
        self.confirm = Some(ConfirmState {
            title: title.to_string(),
            description: description.to_string(),
            action: std::rc::Rc::new(action),
            close_window: false,
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

    pub(crate) fn switch_tab(&mut self, pane: Pane, index: usize, cx: &mut Context<Self>) {
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
        self.persist_state(cx);
        cx.notify();
    }

    /// Restored tabs have no live session. When one is selected after the user
    /// reconnects a profile, adopt that live session instead of showing the
    /// reconnect notice again.
    pub(crate) fn auto_connect_tab(&mut self, tab_id: u64, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        let tab = &self.tabs[index];
        if !tab.auto_connect || tab.connection.is_some() || tab.connecting {
            return false;
        }
        let Some(profile_id) = tab.profile.as_ref().map(|profile| profile.id) else {
            return false;
        };
        let Some(connection) = self
            .tabs
            .iter()
            .filter(|candidate| candidate.id != tab_id)
            .filter_map(|candidate| candidate.connection.as_ref())
            .find(|connection| connection.profile.id == profile_id)
            .cloned()
        else {
            return false;
        };

        self.tabs[index].connection = Some(connection.clone());
        self.tabs[index].connecting = false;
        self.tabs[index].auto_connect = false;
        self.tabs[index].profile = Some(connection.profile.clone());
        self.tabs[index].notice.clear();
        let is_table = matches!(self.tabs[index].body, TabBody::Table(_));
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.error.clear();
                query.output = None;
                query.running = false;
                query.cancelling = false;
                query.operation_id = None;
                query.active_result = 0;
            }
            TabBody::Table(table) => {
                table.page = None;
                table.total_rows_cache = None;
                table.staged.clear();
                table.selected_keys.clear();
                table.loading = false;
                table.operation_id = None;
                table.foreign_keys.clear();
                table.foreign_keys_loading = false;
                table.indexes.clear();
                table.diagram_metadata_loaded = false;
            }
        }
        if is_table {
            self.load_page(tab_id, 0, Vec::new(), None, cx);
        } else {
            self.load_tables(tab_id, cx);
        }
        true
    }

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
}

#[cfg(test)]
mod tests {
    use super::KoyoApp;
    use uuid::Uuid;

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
}
