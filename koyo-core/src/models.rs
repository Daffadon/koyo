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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub id: Option<Uuid>,
    pub name: String,
    #[serde(default)]
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub sqlite_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub session_id: Uuid,
    pub profile: ConnectionProfile,
    pub server_version: String,
    pub current_user: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSummary {
    pub schema: String,
    pub name: String,
    pub editable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemaObjectKind {
    Table,
    View,
    Function,
    Trigger,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaObject {
    pub schema: String,
    pub name: String,
    pub kind: SchemaObjectKind,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_column: String,
    pub constraint_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataItem {
    pub name: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataGroup {
    pub name: String,
    pub items: Vec<MetadataItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    #[serde(skip)]
    pub filter_type: String,
    pub filter_kind: FilterKind,
    pub sortable: bool,
    pub nullable: bool,
    pub primary_key: bool,
    pub generated: bool,
    pub identity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterKind {
    Text,
    Number,
    Temporal,
    Boolean,
    Other,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableFilter {
    pub column: String,
    pub operator: FilterOperator,
    pub value: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSort {
    pub column: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GridRow {
    pub version: String,
    pub key: Vec<Option<String>>,
    pub values: Vec<Option<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<GridRow>,
    pub offset: u32,
    pub limit: u32,
    pub has_more: bool,
    pub editable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCatalogEntry {
    pub schema: String,
    pub table: String,
    pub column: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutput {
    pub result_sets: Vec<QueryResultSet>,
    pub affected_rows: Vec<u64>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GridChange {
    Insert {
        values: BTreeMap<String, Option<String>>,
    },
    Update {
        key: Vec<Option<String>>,
        version: String,
        values: BTreeMap<String, Option<String>>,
        #[serde(default)]
        original: Vec<Option<String>>,
    },
    Delete {
        key: Vec<Option<String>>,
        version: String,
        #[serde(default)]
        original: Vec<Option<String>>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplySummary {
    pub applied: usize,
}
