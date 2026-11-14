mod completion;
mod connection_view;
mod query_view;
mod table_view;
mod view;
mod widgets;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::RunQuery;
use completion::{CompletionCatalog, SqlCompletionProvider};
use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::database;
use koyo_core::models::{
    ConnectionInfo, ConnectionProfile, Engine, FilterKind, FilterOperator, ForeignKeyInfo,
    GridChange, MetadataGroup, QueryOutput, SchemaObject, SslMode, TableFilter, TablePage,
    TableSort, TableSummary,
};
use koyo_core::{ProfileInput, ProfileStore, SessionManager};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use uuid::Uuid;

pub(crate) const PAGE_LIMIT: u32 = 100;
pub(crate) const DEFAULT_COLUMN_WIDTH: f32 = 170.0;
const MIN_COLUMN_WIDTH: f32 = 72.0;
const MAX_COLUMN_WIDTH: f32 = 640.0;

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: gpui_kit::Hsla,
    pub foreground: gpui_kit::Hsla,
    pub muted: gpui_kit::Hsla,
    pub border: gpui_kit::Hsla,
    pub sidebar: gpui_kit::Hsla,
    pub sidebar_border: gpui_kit::Hsla,
    pub panel: gpui_kit::Hsla,
    pub panel_alt: gpui_kit::Hsla,
    pub accent: gpui_kit::Hsla,
    pub accent_soft: gpui_kit::Hsla,
    pub success: gpui_kit::Hsla,
    pub warning: gpui_kit::Hsla,
    pub danger: gpui_kit::Hsla,
}

fn query_tab_title(profile: Option<&ConnectionProfile>) -> SharedString {
    let Some(profile) = profile else {
        return "SQL".into();
    };
    let mut database = profile.database.trim().to_string();
    if database.is_empty() && profile.engine.is_sqlite() {
        database = std::path::Path::new(&profile.sqlite_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("SQLite")
            .to_string();
    }
    if database.is_empty() {
        "SQL".into()
    } else {
        format!("SQL-{database}").into()
    }
}

pub(crate) fn palette(cx: &App) -> Palette {
    let theme = cx.theme();
    Palette {
        background: theme.background,
        foreground: theme.foreground,
        muted: theme.muted_foreground,
        border: theme.border,
        sidebar: theme.sidebar,
        sidebar_border: theme.sidebar_border,
        panel: theme.secondary,
        panel_alt: theme.muted,
        accent: theme.primary,
        accent_soft: theme.accent,
        success: theme.success,
        warning: theme.warning,
        danger: theme.danger,
    }
}

/// A warm, low-contrast light palette. The stock light theme is pure white
/// with saturated accents, which is harsh next to the dark theme; this keeps
/// paper-like surfaces, muted sage accents, and dark gray-green type.
fn apply_calm_light_theme(cx: &mut App) {
    fn tone(hex: u32) -> gpui_kit::Hsla {
        gpui_kit::rgb(hex).into()
    }

    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.background = tone(0xF2F0E9);
        colors.foreground = tone(0x30342F);
        colors.muted = tone(0xE7E3D8);
        colors.muted_foreground = tone(0x74796F);
        colors.border = tone(0xD5D0C3);

        colors.sidebar = tone(0xEAE7DD);
        colors.sidebar_border = tone(0xD5D0C3);
        colors.sidebar_foreground = tone(0x3B403A);
        colors.sidebar_accent = tone(0xE0E6DE);
        colors.sidebar_accent_foreground = tone(0x355344);

        colors.secondary = tone(0xF8F6F0);
        colors.secondary_foreground = tone(0x30342F);
        colors.secondary_hover = tone(0xEFECE3);
        colors.secondary_active = tone(0xE6E1D5);

        colors.primary = tone(0x557A68);
        colors.primary_hover = tone(0x648B78);
        colors.primary_active = tone(0x4A6D5C);
        colors.primary_foreground = tone(0xFBF9F4);
        colors.accent = tone(0xDCE5DE);
        colors.accent_foreground = tone(0x3B5A4B);
        colors.input = tone(0xCFC9BC);
        colors.ring = tone(0x8AA69A);
        colors.caret = tone(0x557A68);

        colors.button = tone(0xEAE7DD);
        colors.button_hover = tone(0xE1DDD2);
        colors.button_active = tone(0xD7D2C6);
        colors.button_foreground = tone(0x30342F);
        colors.button_primary = tone(0x557A68);
        colors.button_primary_hover = tone(0x648B78);
        colors.button_primary_active = tone(0x4A6D5C);
        colors.button_primary_foreground = tone(0xFBF9F4);
        colors.button_secondary = tone(0xE7E3D8);
        colors.button_secondary_hover = tone(0xDFDACE);
        colors.button_secondary_active = tone(0xD4CFC2);
        colors.button_secondary_foreground = tone(0x3D423C);

        colors.popover = tone(0xFBF9F4);
        colors.popover_foreground = tone(0x30342F);
        colors.list = tone(0xF8F6F0);
        colors.list_hover = tone(0xEFECE3);
        colors.list_active = tone(0xE4EBE5);
        colors.list_even = tone(0xF4F2EB);
        colors.list_head = tone(0xEEEAE0);

        colors.success = tone(0x4F7F63);
        colors.success_foreground = tone(0xF7FBF8);
        colors.warning = tone(0xA9783F);
        colors.warning_foreground = tone(0xFDF9F1);
        colors.danger = tone(0xA85E57);
        colors.danger_foreground = tone(0xFFF8F7);
        colors.link = tone(0x4C7463);
        colors.link_hover = tone(0x3F6353);
        colors.link_active = tone(0x345547);

        colors.scrollbar = tone(0xE5E1D6);
        colors.scrollbar_thumb = tone(0xC4BEB0);
        colors.scrollbar_thumb_hover = tone(0xAFA898);
    });
}

#[derive(Clone)]
pub(crate) struct StagedInsert {
    pub(crate) local_id: u64,
    pub(crate) values: BTreeMap<String, Option<String>>,
}

#[derive(Clone)]
pub(crate) enum StagedChange {
    Insert(StagedInsert),
    Update {
        key: Vec<Option<String>>,
        version: String,
        values: BTreeMap<String, Option<String>>,
        original: Vec<Option<String>>,
    },
    Delete {
        key: Vec<Option<String>>,
        version: String,
        original: Vec<Option<String>>,
    },
}

impl StagedChange {
    pub(crate) fn row_key(&self) -> Option<&[Option<String>]> {
        match self {
            StagedChange::Insert(_) => None,
            StagedChange::Update { key, .. } | StagedChange::Delete { key, .. } => Some(key),
        }
    }
}

pub(crate) enum TabBody {
    Query(QueryTab),
    Table(TableTab),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResultView {
    Table,
    Json,
}

pub(crate) struct QueryTab {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) completion: Rc<RefCell<CompletionCatalog>>,
    /// Text installed as the starter query. Used to tell an untouched editor
    /// from SQL the user typed before the first connection completed.
    pub(crate) starter: SharedString,
    pub(crate) output: Option<QueryOutput>,
    pub(crate) active_result: usize,
    pub(crate) result_view: ResultView,
    pub(crate) error: String,
    pub(crate) running: bool,
    pub(crate) cancelling: bool,
    pub(crate) duration_ms: Option<u128>,
    pub(crate) results_collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableMode {
    Data,
    Diagram,
}

pub(crate) struct TableTab {
    pub(crate) schema: String,
    pub(crate) table: String,
    pub(crate) mode: TableMode,
    pub(crate) foreign_keys: Vec<ForeignKeyInfo>,
    pub(crate) foreign_keys_loading: bool,
    pub(crate) page: Option<TablePage>,
    pub(crate) filters: Vec<TableFilter>,
    pub(crate) sort: Option<TableSort>,
    pub(crate) staged: Vec<StagedChange>,
    pub(crate) loading: bool,
    pub(crate) cancelling: bool,
    pub(crate) next_insert_id: u64,
    pub(crate) load_sequence: u64,
    pub(crate) filter_column: Option<usize>,
    pub(crate) filter_operator: FilterOperator,
    pub(crate) filter_value: Entity<InputState>,
    pub(crate) column_widths: Vec<f32>,
    pub(crate) resize_pointer_x: Option<f32>,
    pub(crate) column_resized: bool,
}

pub(crate) struct Tab {
    pub(crate) id: u64,
    pub(crate) title: SharedString,
    pub(crate) profile: Option<ConnectionProfile>,
    pub(crate) connection: Option<ConnectionInfo>,
    pub(crate) tables: Vec<TableSummary>,
    pub(crate) schema_objects: Vec<SchemaObject>,
    pub(crate) metadata_groups: Vec<MetadataGroup>,
    pub(crate) expanded: BTreeSet<String>,
    pub(crate) notice: String,
    pub(crate) connecting: bool,
    pub(crate) pending_starter: Option<SharedString>,
    pub(crate) body: TabBody,
}

impl Tab {
    pub(crate) fn engine(&self) -> Option<Engine> {
        self.profile.as_ref().map(|profile| profile.engine)
    }
}

pub(crate) struct ConnectionDialog {
    pub(crate) engine: Engine,
    pub(crate) editing: Option<Uuid>,
    pub(crate) target_tab: Option<u64>,
    pub(crate) name: Entity<InputState>,
    pub(crate) host: Entity<InputState>,
    pub(crate) port: Entity<InputState>,
    pub(crate) database: Entity<InputState>,
    pub(crate) username: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) sqlite_path: Entity<InputState>,
    pub(crate) ssl_disable: bool,
    pub(crate) busy: bool,
    pub(crate) error: String,
}

pub(crate) enum CellTarget {
    Update {
        key: Vec<Option<String>>,
        version: String,
        original: Vec<Option<String>>,
        column: String,
        original_value: Option<String>,
    },
    Insert {
        local_id: u64,
        column: usize,
    },
}

pub(crate) struct CellEdit {
    pub(crate) tab_id: u64,
    pub(crate) target: CellTarget,
    pub(crate) input: Entity<InputState>,
    pub(crate) is_null: bool,
    pub(crate) insert_is_default: bool,
    pub(crate) title: SharedString,
    pub(crate) column_name: String,
    pub(crate) nullable: bool,
}

pub(crate) struct ConfirmState {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) action: std::rc::Rc<dyn Fn(&mut KoyoApp, &mut Context<KoyoApp>)>,
}

pub(crate) struct KoyoApp {
    pub(crate) store: ProfileStore,
    pub(crate) manager: Arc<SessionManager>,
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) profiles: Vec<ConnectionProfile>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) primary_tabs: Vec<u64>,
    pub(crate) secondary_tabs: Vec<u64>,
    pub(crate) primary_active: u64,
    pub(crate) secondary_active: u64,
    pub(crate) split: Option<SplitAxis>,
    pub(crate) split_ratio: f32,
    pub(crate) focus: Pane,
    pub(crate) sidebar_open: bool,
    pub(crate) status: String,
    pub(crate) dialog: Option<ConnectionDialog>,
    pub(crate) cell_edit: Option<CellEdit>,
    pub(crate) confirm: Option<ConfirmState>,
    next_tab_id: u64,
}

impl ConnectionDialog {
    fn new(
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

    fn add_tab(&mut self, tab: Tab, pane: Pane) -> u64 {
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

    fn normalize_panes(&mut self) {
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

    fn make_query_tab(
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

    fn make_table_tab(
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

    fn spawn_task<T, F>(
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

    fn disconnect_session(&self, session_id: Uuid) {
        let manager = self.manager.clone();
        self.runtime.spawn(async move {
            let _ = database::disconnect(&manager, session_id).await;
        });
    }

    fn load_tables(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            async move {
                let tables = database::list_tables(&manager, connection.session_id).await?;
                let objects =
                    database::list_schema_objects(&manager, connection.session_id).await?;
                let metadata = database::list_database_metadata(&manager, connection.session_id)
                    .await
                    .unwrap_or_default();
                let columns = database::list_column_catalog(&manager, connection.session_id)
                    .await
                    .unwrap_or_default();
                Ok::<_, String>((tables, objects, metadata, columns))
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                match result {
                    Ok((tables, objects, metadata, columns)) => {
                        if let TabBody::Query(query) = &mut this.tabs[index].body {
                            query
                                .completion
                                .borrow_mut()
                                .replace(&tables, &objects, &columns);
                        }
                        let mut expanded = objects
                            .iter()
                            .map(|object| object.schema.clone())
                            .collect::<BTreeSet<_>>();
                        expanded.insert("Schemas".to_string());
                        let schemas = expanded.iter().cloned().collect::<Vec<_>>();
                        for schema in schemas {
                            expanded.insert(format!("{schema}/Table"));
                        }
                        this.tabs[index].expanded = expanded;
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
                        let previous_connection = this.tabs[index].connection.replace(info.clone());
                        if let Some(previous) = previous_connection {
                            if previous.session_id != info.session_id {
                                this.disconnect_session(previous.session_id);
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
                        this.load_tables(tab_id, cx);
                        match &mut this.tabs[index].body {
                            TabBody::Query(query) => {
                                query.completion.borrow_mut().clear();
                                query.output = None;
                                query.error.clear();
                                query.duration_ms = None;
                                query.active_result = 0;
                                query.running = false;
                                query.cancelling = false;
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
                                table.staged.clear();
                                table.loading = false;
                                this.load_page(tab_id, offset, filters, sort, cx);
                            }
                        }
                        if from_dialog {
                            this.dialog = None;
                        }
                    }
                    Err(error) => {
                        this.tabs[index].connecting = false;
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

    pub(crate) fn open_table(
        &mut self,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.active_tab().and_then(|tab| tab.profile.clone()) else {
            self.status = "Open a connection before browsing tables.".into();
            cx.notify();
            return;
        };
        let tab = self.make_table_tab(profile.clone(), schema.clone(), table.clone(), window, cx);
        let tab_id = self.add_tab(tab, self.focus);
        cx.notify();

        let manager = self.manager.clone();
        let store = self.store.clone();
        let profile_for_task = profile.clone();
        let schema_for_task = schema.clone();
        let table_for_task = table.clone();
        self.spawn_task(
            cx,
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
                        this.tabs[index].connection = Some(info);
                        this.tabs[index].profile = Some(profile.clone());
                        this.tabs[index].connecting = false;
                        let mut expanded = objects
                            .iter()
                            .map(|object| object.schema.clone())
                            .collect::<BTreeSet<_>>();
                        expanded.insert("Schemas".to_string());
                        for schema in expanded.clone() {
                            expanded.insert(format!("{schema}/Table"));
                        }
                        this.tabs[index].expanded = expanded;
                        this.tabs[index].tables = tables;
                        this.tabs[index].schema_objects = objects;
                        this.tabs[index].metadata_groups = metadata;
                        if let TabBody::Table(table_tab) = &mut this.tabs[index].body {
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
                        }
                    }
                }
                cx.notify();
            },
        );
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
        table.load_sequence += 1;
        let sequence = table.load_sequence;
        let schema = table.schema.clone();
        let name = table.table.clone();
        let filters_for_task = filters.clone();
        let sort_for_task = sort.clone();
        cx.notify();

        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            async move {
                database::load_table_page(
                    &manager,
                    connection.session_id,
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
                if table.load_sequence != sequence {
                    return;
                }
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

    pub(crate) fn run_active_query(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.focused_index().and_then(|index| self.tabs.get(index)) else {
            return;
        };
        if !matches!(tab.body, TabBody::Query(_)) {
            return;
        }
        let tab_id = tab.id;
        self.run_query(tab_id, cx);
    }

    pub(crate) fn run_query(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            self.tabs[index].notice = "Connect a database to run SQL.".into();
            cx.notify();
            return;
        };
        let TabBody::Query(query) = &self.tabs[index].body else {
            return;
        };
        if query.running {
            return;
        }
        let sql = query.editor.read(cx).value().to_string();
        if sql.trim().is_empty() {
            self.tabs[index].notice = "Enter SQL to run.".into();
            cx.notify();
            return;
        }
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        query.running = true;
        query.cancelling = false;
        query.error.clear();
        query.output = None;
        query.duration_ms = None;
        self.tabs[index].notice.clear();
        cx.notify();

        let started = Instant::now();
        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            async move { database::run_sql(&manager, session_id, sql).await },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Query(query) = &mut this.tabs[index].body else {
                    return;
                };
                query.running = false;
                query.cancelling = false;
                query.duration_ms = Some(started.elapsed().as_millis());
                match result {
                    Ok(output) => {
                        query.output = Some(output);
                        query.active_result = 0;
                        query.error.clear();
                    }
                    Err(error) => query.error = error,
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn cancel_task(&mut self, tab_id: u64, table_only: bool, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let running = match &self.tabs[index].body {
            TabBody::Query(query) => !table_only && query.running,
            TabBody::Table(table) => table.loading,
        };
        if !running {
            return;
        }
        match &mut self.tabs[index].body {
            TabBody::Query(query) => query.cancelling = true,
            TabBody::Table(table) => table.cancelling = true,
        }
        cx.notify();

        let manager = self.manager.clone();
        let engine = self.tabs[index]
            .profile
            .as_ref()
            .map(|profile| profile.engine);
        self.spawn_task(
            cx,
            async move { database::cancel_query(&manager, connection.session_id).await },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                match &mut this.tabs[index].body {
                    TabBody::Query(query) => query.cancelling = false,
                    TabBody::Table(table) => table.cancelling = false,
                }
                match result {
                    Ok(()) => {
                        if engine == Some(Engine::SqlServer) {
                            this.disconnect_now(tab_id, cx);
                            this.tabs[index].notice =
                                "SQL Server cancelled the statement by closing its session. Reconnect to continue."
                                    .into();
                        } else {
                            this.tabs[index].notice = "Cancellation requested.".into();
                        }
                    }
                    Err(error) => this.tabs[index].notice = error,
                }
                cx.notify();
            },
        );
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
                && table.foreign_keys.is_empty()
                && !table.foreign_keys_loading
        };
        if should_load {
            self.load_foreign_keys(tab_id, cx);
        }
        cx.notify();
    }

    fn load_foreign_keys(&mut self, tab_id: u64, cx: &mut Context<Self>) {
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
        table.foreign_keys_loading = true;
        let manager = self.manager.clone();
        self.spawn_task(
            cx,
            async move {
                database::list_foreign_keys(&manager, connection.session_id, &schema, &table_name)
                    .await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                if let TabBody::Table(table) = &mut this.tabs[index].body {
                    table.foreign_keys_loading = false;
                    match result {
                        Ok(foreign_keys) => table.foreign_keys = foreign_keys,
                        Err(error) => this.tabs[index].notice = error,
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn toggle_results(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[index].body {
            query.results_collapsed = !query.results_collapsed;
            cx.notify();
        }
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
                StagedChange::Delete {
                    key,
                    version,
                    original,
                } => GridChange::Delete {
                    key,
                    version,
                    original,
                },
            })
            .collect::<Vec<_>>();
        table.loading = true;
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            async move {
                database::apply_grid_changes(&manager, session_id, schema, name, changes).await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Table(table) = &mut this.tabs[index].body else {
                    return;
                };
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

    pub(crate) fn discard_insert(&mut self, tab_id: u64, local_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        table.staged.retain(
            |change| !matches!(change, StagedChange::Insert(insert) if insert.local_id == local_id),
        );
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

    pub(crate) fn set_filter_column(
        &mut self,
        tab_id: u64,
        column: usize,
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
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        table.filter_column = Some(column);
        table.filter_operator = default_operator(info.filter_kind);
        table.filter_value.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    pub(crate) fn set_filter_operator(
        &mut self,
        tab_id: u64,
        operator: FilterOperator,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        table.filter_operator = operator;
        cx.notify();
    }

    pub(crate) fn apply_filter(
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
        if table.loading || !table.staged.is_empty() {
            this_notice(
                &mut self.tabs[index],
                "Apply or discard staged edits to filter.",
            );
            cx.notify();
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(column) = table
            .filter_column
            .and_then(|index| page.columns.get(index))
        else {
            this_notice(&mut self.tabs[index], "Choose a column to filter.");
            cx.notify();
            return;
        };
        let value = table.filter_value.read(cx).value().to_string();
        if filter_needs_value(table.filter_operator) && value.is_empty() {
            this_notice(&mut self.tabs[index], "Enter a filter value.");
            cx.notify();
            return;
        }
        let filter = TableFilter {
            column: column.name.clone(),
            operator: table.filter_operator,
            value: filter_needs_value(table.filter_operator).then_some(value),
        };
        table.filters.retain(|item| item.column != filter.column);
        table.filters.push(filter);
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        table.filter_value.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.load_page(tab_id, 0, filters, sort, cx);
    }

    pub(crate) fn clear_filters(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.loading || !table.staged.is_empty() {
            return;
        }
        table.filters.clear();
        let sort = table.sort.clone();
        self.load_page(tab_id, 0, Vec::new(), sort, cx);
    }

    pub(crate) fn remove_filter(&mut self, tab_id: u64, column: String, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.loading || !table.staged.is_empty() {
            return;
        }
        table.filters.retain(|filter| filter.column != column);
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        self.load_page(tab_id, 0, filters, sort, cx);
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

    pub(crate) fn open_cell_editor(
        &mut self,
        tab_id: u64,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[index].connection.is_none() {
            return;
        }
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        if page.rows.get(row).is_none() || info.generated || info.identity {
            return;
        }
        if table
            .staged
            .iter()
            .any(|change| matches!(change, StagedChange::Delete { key, .. } if key == &page.rows[row].key))
        {
            return;
        }
        let current = table
            .staged
            .iter()
            .find_map(|change| match change {
                StagedChange::Update { key, values, .. } if key == &page.rows[row].key => Some(
                    values
                        .get(&info.name)
                        .cloned()
                        .unwrap_or_else(|| page.rows[row].values[column].clone()),
                ),
                _ => None,
            })
            .unwrap_or_else(|| page.rows[row].values[column].clone());
        let is_null = current.is_none();
        let text = current.unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .default_value(text)
        });
        let handle = input.read(cx).focus_handle(cx);
        let row_data = &page.rows[row];
        self.cell_edit = Some(CellEdit {
            tab_id,
            target: CellTarget::Update {
                key: row_data.key.clone(),
                version: row_data.version.clone(),
                original: row_data.values.clone(),
                column: info.name.clone(),
                original_value: row_data.values[column].clone(),
            },
            input,
            is_null,
            insert_is_default: false,
            title: format!("Edit {}", info.name).into(),
            column_name: info.name.clone(),
            nullable: info.nullable,
        });
        handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn open_insert_cell_editor(
        &mut self,
        tab_id: u64,
        local_id: u64,
        column: usize,
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
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        if info.generated || info.identity {
            return;
        }
        let Some(insert) = table.staged.iter().find_map(|change| match change {
            StagedChange::Insert(insert) if insert.local_id == local_id => Some(insert),
            _ => None,
        }) else {
            return;
        };
        let existing = insert.values.get(&info.name).cloned();
        let (is_default, is_null, text) = match existing {
            None => (true, false, String::new()),
            Some(None) => (false, true, String::new()),
            Some(Some(value)) => (false, false, value),
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .default_value(text)
        });
        let handle = input.read(cx).focus_handle(cx);
        self.cell_edit = Some(CellEdit {
            tab_id,
            target: CellTarget::Insert { local_id, column },
            input,
            is_null,
            insert_is_default: is_default,
            title: format!("New value for {}", info.name).into(),
            column_name: info.name.clone(),
            nullable: info.nullable,
        });
        handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn set_cell_null(&mut self, is_null: bool, cx: &mut Context<Self>) {
        if let Some(edit) = self.cell_edit.as_mut() {
            edit.is_null = is_null;
            if is_null {
                edit.insert_is_default = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn set_cell_default(&mut self, is_default: bool, cx: &mut Context<Self>) {
        if let Some(edit) = self.cell_edit.as_mut() {
            edit.insert_is_default = is_default;
            if is_default {
                edit.is_null = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn set_result_view(
        &mut self,
        tab_id: u64,
        view: ResultView,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[index].body {
            query.result_view = view;
            cx.notify();
        }
    }

    pub(crate) fn set_active_result(&mut self, tab_id: u64, index: usize, cx: &mut Context<Self>) {
        let Some(tab_index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[tab_index].body {
            query.active_result = index;
        }
        cx.notify();
    }

    pub(crate) fn close_cell_editor(&mut self, cx: &mut Context<Self>) {
        self.cell_edit = None;
        cx.notify();
    }

    pub(crate) fn apply_cell_editor(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.cell_edit.as_ref() else {
            return;
        };
        if !edit.nullable && edit.is_null && !edit.insert_is_default {
            return;
        }
        let value = edit.input.read(cx).value().to_string();
        let tab_id = edit.tab_id;
        let target = match &edit.target {
            CellTarget::Update {
                key,
                version,
                original,
                column,
                original_value,
            } => CellTarget::Update {
                key: key.clone(),
                version: version.clone(),
                original: original.clone(),
                column: column.clone(),
                original_value: original_value.clone(),
            },
            CellTarget::Insert { local_id, column } => CellTarget::Insert {
                local_id: *local_id,
                column: *column,
            },
        };
        let is_null = edit.is_null;
        let is_default = edit.insert_is_default;
        self.cell_edit = None;
        self.update_cell_value(tab_id, target, is_null, is_default, value, cx);
        cx.notify();
    }

    fn update_cell_value(
        &mut self,
        tab_id: u64,
        target: CellTarget,
        is_null: bool,
        is_default: bool,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let page = table.page.clone();
        match target {
            CellTarget::Update {
                key,
                version,
                original,
                column,
                original_value,
            } => {
                let existing = table.staged.iter().position(|change| match change {
                    StagedChange::Update {
                        key: staged_key, ..
                    } => staged_key == &key,
                    _ => false,
                });
                let mut values = existing
                    .and_then(|index| match &table.staged[index] {
                        StagedChange::Update { values, .. } => Some(values.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let new_value = if is_null { None } else { Some(value) };
                if original_value == new_value {
                    values.remove(&column);
                } else {
                    values.insert(column, new_value);
                }
                if let Some(index) = existing {
                    table.staged.remove(index);
                }
                if !values.is_empty() {
                    table.staged.push(StagedChange::Update {
                        key,
                        version,
                        values,
                        original,
                    });
                }
            }
            CellTarget::Insert { local_id, column } => {
                let Some(page) = page else {
                    return;
                };
                let Some(info) = page.columns.get(column) else {
                    return;
                };
                let Some(change) = table.staged.iter_mut().find_map(|change| match change {
                    StagedChange::Insert(insert) if insert.local_id == local_id => Some(insert),
                    _ => None,
                }) else {
                    return;
                };
                if is_default {
                    change.values.remove(&info.name);
                } else if is_null {
                    change.values.insert(info.name.clone(), None);
                } else {
                    change.values.insert(info.name.clone(), Some(value));
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_row_delete(
        &mut self,
        tab_id: u64,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(row_data) = page.rows.get(row) else {
            return;
        };
        let key = row_data.key.clone();
        let existing = table.staged.iter().position(|change| match change {
            StagedChange::Delete {
                key: staged_key, ..
            } => staged_key == &key,
            StagedChange::Update {
                key: staged_key, ..
            } => staged_key == &key,
            _ => false,
        });
        match existing {
            Some(position) if matches!(table.staged[position], StagedChange::Delete { .. }) => {
                self.stage_row_delete(tab_id, row, false, cx);
            }
            Some(_) => {
                self.confirm(
                    window,
                    cx,
                    "Stage deletion?",
                    "Staged edits to this row will be discarded.",
                    move |this, cx| this.stage_row_delete(tab_id, row, true, cx),
                );
            }
            None => {
                self.stage_row_delete(tab_id, row, true, cx);
            }
        }
    }

    fn stage_row_delete(&mut self, tab_id: u64, row: usize, delete: bool, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.clone() else {
            return;
        };
        let Some(row_data) = page.rows.get(row).cloned() else {
            return;
        };
        let key = row_data.key.clone();
        table.staged.retain(|change| match change {
            StagedChange::Insert(_) => true,
            _ => change.row_key() != Some(key.as_slice()),
        });
        if delete {
            table.staged.push(StagedChange::Delete {
                key,
                version: row_data.version,
                original: row_data.values,
            });
        }
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
        dialog.engine = engine;
        dialog.editing = None;
        dialog.error.clear();
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
            "Tabs using it will close; running queries are cancelled and staged edits are discarded.",
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

    fn apply_pending_starters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

impl TabBody {
    pub(crate) fn table(&self) -> Option<&TableTab> {
        match self {
            TabBody::Table(table) => Some(table),
            _ => None,
        }
    }
}

impl Render for KoyoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.primary_tabs.is_empty() {
            let tab = self.make_query_tab(None, window, cx);
            self.add_tab(tab, Pane::Primary);
        }
        self.normalize_panes();
        self.apply_pending_starters(window, cx);

        let palette = palette(cx);
        let dialog = if let Some(dialog) = self.dialog.as_ref() {
            Some(connection_view::render_dialog(self, dialog, &palette, cx))
        } else {
            None
        };
        let cell_edit = if let Some(edit) = self.cell_edit.as_ref() {
            Some(widgets::render_cell_editor(edit, &palette, cx))
        } else {
            None
        };
        let confirm = if let Some(pending) = self.confirm.as_ref() {
            Some(widgets::render_confirm(pending, &palette, cx))
        } else {
            None
        };

        div()
            .flex()
            .size_full()
            .bg(palette.background)
            .text_color(palette.foreground)
            .when(self.sidebar_open, |this| {
                this.child(self.render_sidebar(&palette, cx))
            })
            .child(self.render_workspace(window, &palette, cx))
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.run_active_query(cx);
            }))
            .when_some(dialog, |this, dialog| this.child(dialog))
            .when_some(cell_edit, |this, editor| this.child(editor))
            .when_some(confirm, |this, confirm| this.child(confirm))
    }
}

fn starter_sql(engine: Engine) -> String {
    match engine {
        Engine::Postgres => "SELECT current_database(), current_user;".into(),
        Engine::Sqlite => "SELECT sqlite_version();".into(),
        Engine::Mysql | Engine::Mariadb => "SELECT DATABASE(), CURRENT_USER();".into(),
        Engine::SqlServer => "SELECT DB_NAME(), SUSER_SNAME();".into(),
        Engine::Oracle => "SELECT SYS_CONTEXT('USERENV', 'DB_NAME'), USER FROM dual;".into(),
    }
}

pub(crate) fn engine_label(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "PostgreSQL",
        Engine::Sqlite => "SQLite",
        Engine::Mysql => "MySQL",
        Engine::Mariadb => "MariaDB",
        Engine::SqlServer => "SQL Server",
        Engine::Oracle => "Oracle (beta)",
    }
}

pub(crate) fn filter_needs_value(operator: FilterOperator) -> bool {
    matches!(
        operator,
        FilterOperator::Contains
            | FilterOperator::Equals
            | FilterOperator::NotEquals
            | FilterOperator::LessThan
            | FilterOperator::LessThanOrEqual
            | FilterOperator::GreaterThan
            | FilterOperator::GreaterThanOrEqual
    )
}

pub(crate) fn default_operator(kind: FilterKind) -> FilterOperator {
    match kind {
        FilterKind::Text => FilterOperator::Contains,
        FilterKind::Number | FilterKind::Temporal => FilterOperator::Equals,
        FilterKind::Boolean => FilterOperator::IsTrue,
        FilterKind::Other => FilterOperator::IsNull,
    }
}

pub(crate) fn operator_options(kind: FilterKind) -> Vec<(FilterOperator, &'static str)> {
    match kind {
        FilterKind::Text => vec![
            (FilterOperator::Contains, "contains"),
            (FilterOperator::Equals, "equals"),
            (FilterOperator::NotEquals, "not equals"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Number | FilterKind::Temporal => vec![
            (FilterOperator::Equals, "equals"),
            (FilterOperator::NotEquals, "not equals"),
            (FilterOperator::LessThan, "less than"),
            (FilterOperator::LessThanOrEqual, "at most"),
            (FilterOperator::GreaterThan, "greater than"),
            (FilterOperator::GreaterThanOrEqual, "at least"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Boolean => vec![
            (FilterOperator::IsTrue, "is true"),
            (FilterOperator::IsFalse, "is false"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Other => vec![
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
    }
}

fn set_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| {
        input.set_value(value.to_string(), window, cx);
    });
}

fn set_placeholder(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| {
        input.set_placeholder(value, window, cx);
    });
}

fn apply_defaults(
    dialog: &mut ConnectionDialog,
    engine: Engine,
    window: &mut Window,
    cx: &mut App,
) {
    let (host, port, database, database_placeholder, username, sqlite_disabled, sqlite_path) =
        match engine {
            Engine::Postgres => (
                "localhost",
                5432,
                "postgres",
                "postgres",
                "postgres",
                false,
                "",
            ),
            Engine::Mysql | Engine::Mariadb => {
                ("localhost", 3306, "", "database name", "root", false, "")
            }
            Engine::SqlServer => (
                "localhost",
                1433,
                "master",
                "database name",
                "sa",
                false,
                "",
            ),
            Engine::Oracle => (
                "localhost",
                1521,
                "FREEPDB1",
                "service name",
                "system",
                false,
                "",
            ),
            Engine::Sqlite => ("", 0, "", "database name", "", true, ""),
        };
    set_input(&dialog.host, host, window, cx);
    set_input(&dialog.port, &port.to_string(), window, cx);
    set_input(&dialog.database, database, window, cx);
    set_placeholder(&dialog.database, database_placeholder, window, cx);
    set_input(&dialog.username, username, window, cx);
    set_input(&dialog.sqlite_path, sqlite_path, window, cx);
    set_input(&dialog.password, "", window, cx);
    dialog.ssl_disable = sqlite_disabled;
}

fn this_notice(tab: &mut Tab, message: impl Into<String>) {
    tab.notice = message.into();
}
