use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    #[default]
    VerifyFull,
    Disable,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Engine {
    #[default]
    Postgres,
    Sqlite,
    Mysql,
    Mariadb,
    SqlServer,
    Oracle,
}

impl Engine {
    pub fn is_sqlite(self) -> bool {
        matches!(self, Self::Sqlite)
    }

    pub fn needs_password(self) -> bool {
        !self.is_sqlite()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub sqlite_path: String,
}

#[derive(Debug)]
pub struct ProfileInput {
    pub id: Option<Uuid>,
    pub name: String,
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssl_mode: SslMode,
    pub sqlite_path: String,
}

#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    pub session_id: Uuid,
    pub profile: ConnectionProfile,
    pub server_version: String,
    pub current_user: String,
}

#[derive(Clone, Debug)]
pub struct TableSummary {
    pub schema: String,
    pub name: String,
    pub editable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaObjectKind {
    Table,
    View,
    Function,
    Trigger,
}

#[derive(Clone, Debug)]
pub struct SchemaObject {
    pub schema: String,
    pub name: String,
    pub kind: SchemaObjectKind,
}

#[derive(Clone, Debug)]
pub struct ForeignKeyInfo {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_column: String,
    pub constraint_name: String,
}

#[derive(Clone, Debug)]
pub struct TableIndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Clone, Debug)]
pub struct MetadataItem {
    pub name: String,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct MetadataGroup {
    pub name: String,
    pub items: Vec<MetadataItem>,
}

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub filter_type: String,
    pub filter_kind: FilterKind,
    pub sortable: bool,
    pub nullable: bool,
    pub primary_key: bool,
    pub generated: bool,
    pub identity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Text,
    Number,
    Temporal,
    Boolean,
    Other,
}

#[derive(Clone, Debug)]
pub struct TableFilter {
    pub column: String,
    pub operator: FilterOperator,
    pub value: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOperator {
    Contains,
    Equals,
    NotEquals,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    IsNull,
    IsNotNull,
    IsTrue,
    IsFalse,
}

#[derive(Clone, Debug)]
pub struct TableSort {
    pub column: String,
    pub descending: bool,
}

#[derive(Clone, Debug)]
pub struct GridRow {
    pub version: String,
    pub key: Vec<Option<String>>,
    pub values: Vec<Option<String>>,
    pub truncated: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<GridRow>,
    pub offset: u32,
    pub limit: u32,
    pub total_rows: Option<u64>,
    pub has_more: bool,
    pub editable: bool,
}

#[derive(Clone, Debug)]
pub struct ColumnCatalogEntry {
    pub schema: String,
    pub table: String,
    pub column: String,
}

#[derive(Clone, Debug)]
pub struct QueryResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Debug)]
pub struct QueryOutput {
    pub result_sets: Vec<QueryResultSet>,
    pub affected_rows: Vec<u64>,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum GridChange {
    Insert {
        values: BTreeMap<String, Option<String>>,
    },
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

#[derive(Debug)]
pub struct ApplySummary {
    pub applied: usize,
}
