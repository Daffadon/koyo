use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::database;
use koyo_core::models::{ConnectionProfile, Engine, FilterOperator};
use koyo_core::{ProfileStore, SessionManager};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::app::completion::{CompletionCatalog, SqlCompletionProvider};
use crate::app::*;

impl KoyoApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, runtime: Arc<Runtime>) -> Self {
        let store = ProfileStore::from_default_dir();
        let (profiles, status) = match store.load_profiles() {
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

        let mut app = Self {
            store,
            manager: Arc::new(SessionManager::default()),
            runtime,
            profiles,
            tabs: Vec::new(),
            primary_tabs: Vec::new(),
            secondary_tabs: Vec::new(),
            primary_active: 0,
            secondary_active: 0,
            split: None,
            split_ratio: 0.5,
            focus: Pane::Primary,
            sidebar_open: true,
            status,
            dialog: None,
            cell_edit: None,
            confirm: None,
            next_tab_id: 1,
        };
        let tab = app.make_query_tab(None, window, cx);
        let id = tab.id;
        app.tabs.push(tab);
        app.primary_tabs.push(id);
        app.primary_active = id;
        app
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

    fn pane_active_id(&self, pane: Pane) -> Option<u64> {
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

    pub(crate) fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.focused_index()?)
    }

    pub(crate) fn focus_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        if self.pane_index(pane).is_some() && self.focus != pane {
            self.focus = pane;
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
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
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
        id
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
            cx.notify();
            return;
        }
        if self.split.is_none() {
            self.split = Some(axis);
            let tab = self.make_query_tab(None, window, cx);
            self.add_tab(tab, Pane::Secondary);
        } else {
            self.split = Some(axis);
        }
        self.focus = Pane::Primary;
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
            gpui_kit::component::input::EditorState::new(window, cx).default_value(starter.clone())
        });
        let provider: Rc<dyn gpui_kit::base::input::CompletionProvider> =
            Rc::new(SqlCompletionProvider::new(completion.clone()));
        editor.update(cx, |editor, _| {
            editor.lsp_mut().completion_provider = Some(provider);
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
                duration_ms: None,
                results_collapsed: false,
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
        }
        cx.notify();
    }

    pub(crate) fn end_column_resize(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.resize_pointer_x = None;
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
        fallback_origin_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let previous_x = table.resize_pointer_x.unwrap_or(fallback_origin_x);
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
        let filter_value = cx.new(|cx| InputState::new(window, cx).placeholder("Filter value"));
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
            pending_starter: None,
            body: TabBody::Table(TableTab {
                schema,
                table,
                mode: TableMode::Data,
                foreign_keys: Vec::new(),
                foreign_keys_loading: false,
                page: None,
                filters: Vec::new(),
                sort: None,
                staged: Vec::new(),
                loading: true,
                cancelling: false,
                next_insert_id: 1,
                load_sequence: 0,
                filter_column: None,
                filter_operator: FilterOperator::Contains,
                filter_value,
                column_widths: Vec::new(),
                resize_pointer_x: None,
                column_resized: false,
            }),
        }
    }

    pub(crate) fn spawn_task<T, F>(
        &self,
        cx: &mut Context<Self>,
        task: impl Future<Output = T> + Send + 'static,
        done: F,
    ) where
        T: Send + 'static,
        F: FnOnce(&mut Self, &mut Context<Self>, T) + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.runtime.spawn(async move {
            let _ = tx.send(task.await);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(value) = rx.await {
                let _ = this.update(cx, |this, cx| done(this, cx, value));
            }
        })
        .detach();
    }

    pub(crate) fn disconnect_session(&self, session_id: Uuid) {
        let manager = self.manager.clone();
        self.runtime.spawn(async move {
            let _ = database::disconnect(&manager, session_id).await;
        });
    }

    pub(crate) fn disconnect_now(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let Some(connection) = self.tabs[index].connection.take() {
            self.disconnect_session(connection.session_id);
        }
        self.tabs[index].connecting = false;
        self.tabs[index].notice = "Disconnected. Reconnect when needed.".into();
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.running = false;
                query.cancelling = false;
            }
            TabBody::Table(table) => {
                table.loading = false;
                table.cancelling = false;
                table.staged.clear();
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
        let profile = self.active_tab().and_then(|tab| tab.profile.clone());
        let tab = self.make_query_tab(profile.clone(), window, cx);
        let tab_id = self.add_tab(tab, self.focus);
        if let Some(profile) = profile {
            self.connect_profile(profile.id, Some(tab_id), false, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_active_tab(
        &mut self,
        pane: Pane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.pane_active_id(pane) else {
            return;
        };
        self.close_tab(tab_id, window, cx);
    }

    pub(crate) fn close_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
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
                "Close tab?",
                "A query is running or edits are staged. Closing discards them.",
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
        if let Some(connection) = self.tabs[index].connection.take() {
            self.disconnect_session(connection.session_id);
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
        });
        cx.notify();
    }

    pub(crate) fn resolve_confirm(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let Some(pending) = self.confirm.take() else {
            return;
        };
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
        self.normalize_panes();
        cx.notify();
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
