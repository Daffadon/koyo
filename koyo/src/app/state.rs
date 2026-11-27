use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{EditorState, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::{
    ColumnCatalogEntry, ConnectionInfo, ConnectionProfile, Engine, ForeignKeyInfo, GridRow,
    MetadataGroup, QueryOutput, SchemaObject, TableFilter, TableIndexInfo, TablePage, TableSort,
    TableSummary,
};
use koyo_core::{ProfileStore, SessionManager};

use super::ai::AiThread;
use super::completion::CompletionCatalog;
use super::settings::{
    NotificationPreferences, QueryHistoryEntry, SettingsSection, StateSaveQueue,
};
use super::theme::ThemeChoice;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub(crate) const PAGE_LIMIT: u32 = 100;
pub(crate) const QUICK_OPEN_RESULT_LIMIT: usize = 100;
pub(crate) const DEFAULT_COLUMN_WIDTH: f32 = 170.0;
pub(crate) const RESULT_COLUMN_WIDTH: f32 = 180.0;
pub(crate) const MIN_COLUMN_WIDTH: f32 = 72.0;
pub(crate) const MAX_COLUMN_WIDTH: f32 = 640.0;
pub(crate) const DEFAULT_SIDEBAR_WIDTH: f32 = 300.0;
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 220.0;
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 520.0;
pub(crate) const DEFAULT_IDLE_POOL_SIZE: usize = 4;
pub(crate) const MAX_IDLE_POOL_SIZE: usize = 8;
pub(crate) const DEFAULT_IDLE_POOL_TTL_SECS: u64 = 300;
pub(crate) const MIN_IDLE_POOL_TTL_SECS: u64 = 30;
pub(crate) const MAX_IDLE_POOL_TTL_SECS: u64 = 3600;
pub(crate) const PARKED_CONNECTION_PROBE: std::time::Duration = std::time::Duration::from_secs(15);
pub(crate) const INACTIVE_CONNECTION_RECHECK: std::time::Duration =
    std::time::Duration::from_secs(180);

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
        truncated: Vec<bool>,
    },
    Delete {
        key: Vec<Option<String>>,
        version: String,
        original: Vec<Option<String>>,
        truncated: Vec<bool>,
    },
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
pub(crate) enum OpenTableMode {
    FocusOrOpen,
    NewTab,
    OtherPane,
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
    pub(crate) operation_id: Option<Uuid>,
    pub(crate) duration_ms: Option<u128>,
    pub(crate) results_collapsed: bool,
    pub(crate) pending_run: bool,
    pub(crate) definition_kind: Option<koyo_core::models::SchemaObjectKind>,
    pub(crate) result_column_widths: Vec<Vec<f32>>,
    pub(crate) result_resize_pointer_x: Option<f32>,
    pub(crate) result_resize_column: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableMode {
    Data,
    Diagram,
}

pub(crate) fn key_is_truncated(page: &TablePage, row: &GridRow) -> bool {
    page.columns.iter().enumerate().any(|(index, column)| {
        column.primary_key && row.truncated.get(index).copied().unwrap_or(false)
    })
}

pub(crate) struct TableTab {
    pub(crate) schema: String,
    pub(crate) table: String,
    pub(crate) mode: TableMode,
    pub(crate) foreign_keys: Vec<ForeignKeyInfo>,
    pub(crate) foreign_keys_loading: bool,
    pub(crate) indexes: Vec<TableIndexInfo>,
    pub(crate) diagram_metadata_loaded: bool,
    pub(crate) diagram_generation: u64,
    pub(crate) page: Option<TablePage>,
    pub(crate) total_rows_cache: Option<u64>,
    pub(crate) filters: Vec<TableFilter>,
    pub(crate) sort: Option<TableSort>,
    pub(crate) staged: Vec<StagedChange>,
    pub(crate) selected_keys: BTreeSet<Vec<Option<String>>>,
    pub(crate) loading: bool,
    pub(crate) cancelling: bool,
    pub(crate) operation_id: Option<Uuid>,
    pub(crate) next_insert_id: u64,
    pub(crate) load_sequence: u64,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter_error: String,
    pub(crate) column_widths: Vec<f32>,
    pub(crate) resize_pointer_x: Option<f32>,
    pub(crate) resize_column: Option<usize>,
    pub(crate) column_resized: bool,
    pub(crate) exporting: bool,
}

/// A connected session kept alive after its last tab closes. Tabs can park
/// here so opening a new tab does not force a reconnect.
pub(crate) struct LiveConnection {
    pub(crate) profile: ConnectionProfile,
    pub(crate) connection: ConnectionInfo,
    pub(crate) tables: Vec<TableSummary>,
    pub(crate) schema_objects: Vec<SchemaObject>,
    pub(crate) metadata_groups: Vec<MetadataGroup>,
    pub(crate) columns: Vec<ColumnCatalogEntry>,
    pub(crate) parked_at: std::time::Instant,
}

#[derive(Clone, Default)]
pub(crate) struct CatalogSnapshot {
    pub(crate) tables: Vec<TableSummary>,
    pub(crate) schema_objects: Vec<SchemaObject>,
    pub(crate) metadata_groups: Vec<MetadataGroup>,
    pub(crate) columns: Vec<ColumnCatalogEntry>,
}

pub(crate) struct Tab {
    pub(crate) id: u64,
    pub(crate) title: SharedString,
    pub(crate) profile: Option<ConnectionProfile>,
    pub(crate) connection: Option<ConnectionInfo>,
    pub(crate) tables: Vec<TableSummary>,
    pub(crate) schema_objects: Vec<SchemaObject>,
    pub(crate) metadata_groups: Vec<MetadataGroup>,
    pub(crate) columns: Vec<ColumnCatalogEntry>,
    pub(crate) expanded: BTreeSet<String>,
    pub(crate) notice: String,
    pub(crate) connecting: bool,
    pub(crate) catalog_loading: bool,
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
        truncated: Vec<bool>,
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
    pub(crate) input: Entity<TextareaState>,
    pub(crate) is_null: bool,
    pub(crate) insert_is_default: bool,
    pub(crate) title: SharedString,
    pub(crate) column_name: String,
    pub(crate) nullable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExportScope {
    CurrentPage,
    Page,
    Range,
    All,
}

pub(crate) struct TableExportDialog {
    pub(crate) tab_id: u64,
    pub(crate) title: SharedString,
    pub(crate) current_page: u32,
    pub(crate) total_pages: u32,
    pub(crate) scope: ExportScope,
    pub(crate) page: Entity<InputState>,
    pub(crate) end_page: Entity<InputState>,
    pub(crate) path: Entity<InputState>,
    pub(crate) file_name: String,
    pub(crate) error: String,
}

pub(crate) struct QuickOpen {
    pub(crate) input: Entity<InputState>,
    pub(crate) selected: usize,
    pub(crate) scroll: ScrollHandle,
    pub(crate) _subscription: gpui_kit::Subscription,
    pub(crate) return_focus: Option<FocusHandle>,
}

pub(crate) struct ConfirmState {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) action: std::rc::Rc<dyn Fn(&mut KoyoApp, &mut Context<KoyoApp>)>,
    pub(crate) close_window: bool,
    pub(crate) return_focus: Option<FocusHandle>,
}

pub(crate) struct KoyoApp {
    pub(crate) store: ProfileStore,
    pub(crate) save_queue: Arc<StateSaveQueue>,
    pub(crate) history: Vec<QueryHistoryEntry>,
    pub(crate) history_filter: Entity<InputState>,
    pub(crate) history_collapsed: bool,
    pub(crate) history_scroll: ScrollHandle,
    pub(crate) catalog_generations: HashMap<Uuid, u64>,
    pub(crate) session_transactions: HashMap<Uuid, bool>,
    pub(crate) allow_close: bool,
    pub(crate) editor_persist_task: Option<Task<()>>,
    pub(crate) inactive_since: Option<std::time::Instant>,
    pub(crate) persistence_disabled: bool,
    pub(crate) manager: Arc<SessionManager>,
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) profiles: Vec<ConnectionProfile>,
    pub(crate) parked_connections: Vec<LiveConnection>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) primary_tabs: Vec<u64>,
    pub(crate) secondary_tabs: Vec<u64>,
    pub(crate) primary_active: u64,
    pub(crate) secondary_active: u64,
    pub(crate) primary_tab_scroll: ScrollHandle,
    pub(crate) secondary_tab_scroll: ScrollHandle,
    pub(crate) explorer_filter: Entity<InputState>,
    pub(crate) profile_filter: Entity<InputState>,
    pub(crate) split: Option<SplitAxis>,
    pub(crate) split_ratio: f32,
    pub(crate) focus: Pane,
    pub(crate) root_focus: FocusHandle,
    pub(crate) window_handle: gpui_kit::AnyWindowHandle,
    pub(crate) sidebar_open: bool,
    pub(crate) sidebar_width: f32,
    pub(crate) sidebar_resizing: bool,
    /// Tab whose connection owns the shared explorer. Switching tabs does not
    /// replace it; only connecting or closing the source tab changes it.
    pub(crate) sidebar_tab: Option<u64>,
    pub(crate) ai_open: bool,
    pub(crate) ai_width: f32,
    pub(crate) ai_resizing: bool,
    pub(crate) ai_settings_open: bool,
    pub(crate) settings_open: bool,
    pub(crate) settings_focus: FocusHandle,
    pub(crate) theme_choice: ThemeChoice,
    pub(crate) settings_section: SettingsSection,
    pub(crate) notifications: NotificationPreferences,
    pub(crate) idle_pool_size: usize,
    pub(crate) idle_pool_ttl_secs: u64,
    pub(crate) ai_config_json: String,
    pub(crate) ai_model: String,
    pub(crate) ai_config_error: String,
    pub(crate) ai_threads: Vec<AiThread>,
    pub(crate) ai_active_thread: u64,
    pub(crate) next_ai_thread_id: u64,
    pub(crate) ai_task: Option<JoinHandle<()>>,
    pub(crate) ai_generation: u64,
    pub(crate) ai_compacting: bool,
    pub(crate) ai_input: Entity<TextareaState>,
    pub(crate) ai_config_input: Entity<TextareaState>,
    pub(crate) ai_pending: bool,
    pub(crate) ai_error: String,
    pub(crate) ai_scroll: ScrollHandle,
    pub(crate) status: String,
    pub(crate) dialog: Option<ConnectionDialog>,
    pub(crate) confirm_focus: FocusHandle,
    pub(crate) cell_edit: Option<CellEdit>,
    pub(crate) confirm: Option<ConfirmState>,
    pub(crate) export: Option<TableExportDialog>,
    pub(crate) quick_open: Option<QuickOpen>,
    pub(crate) next_tab_id: u64,
}

impl TabBody {
    pub(crate) fn table(&self) -> Option<&TableTab> {
        match self {
            TabBody::Table(table) => Some(table),
            _ => None,
        }
    }
}
