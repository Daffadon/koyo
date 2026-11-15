use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::{
    ConnectionInfo, ConnectionProfile, Engine, FilterOperator, ForeignKeyInfo, MetadataGroup,
    QueryOutput, SchemaObject, TableFilter, TablePage, TableSort, TableSummary,
};
use koyo_core::{ProfileStore, SessionManager};

use super::completion::CompletionCatalog;
use super::settings::{QueryHistoryEntry, SettingsStore};
use tokio::runtime::Runtime;
use uuid::Uuid;

pub(crate) const PAGE_LIMIT: u32 = 100;
pub(crate) const DEFAULT_COLUMN_WIDTH: f32 = 170.0;
pub(crate) const RESULT_COLUMN_WIDTH: f32 = 180.0;
pub(crate) const RESULT_ROW_RAIL_WIDTH: f32 = 56.0;
pub(crate) const MIN_COLUMN_WIDTH: f32 = 72.0;
pub(crate) const MAX_COLUMN_WIDTH: f32 = 640.0;

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
    pub(crate) pending_run: bool,
    pub(crate) result_column_widths: Vec<Vec<f32>>,
    pub(crate) result_resize_pointer_x: Option<f32>,
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
    pub(crate) close_window: bool,
}

pub(crate) struct KoyoApp {
    pub(crate) store: ProfileStore,
    pub(crate) settings: SettingsStore,
    pub(crate) history: Vec<QueryHistoryEntry>,
    pub(crate) allow_close: bool,
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
