use crate::models::{
    ApplySummary, ColumnInfo, FilterKind, FilterOperator, GridChange, GridRow, QueryOutput,
    QueryResultSet, TableFilter, TablePage, TableSort, TableSummary,
};
use rusqlite::{
    Connection, InterruptHandle, TransactionBehavior, params_from_iter, types::ValueRef,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const QUERY_ROW_LIMIT: usize = 1_000;
const TABLE_PAGE_LIMIT: u32 = 200;

#[derive(Clone)]
pub struct SqliteSession {
    connection: Arc<Mutex<Connection>>,
    pub interrupt: Arc<InterruptHandle>,
}

pub fn connect(path: &str) -> Result<(SqliteSession, String), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("SQLite database file is required.".into());
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("Could not open SQLite database: {error}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("Could not set SQLite busy timeout: {error}"))?;
    let interrupt = Arc::new(connection.get_interrupt_handle());
    Ok((
        SqliteSession {
            connection: Arc::new(Mutex::new(connection)),
            interrupt,
        },
        rusqlite::version().to_string(),
    ))
}

pub async fn list_tables(session: &SqliteSession) -> Result<Vec<TableSummary>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        list_tables_sync(&connection)
    })
    .await
}

pub async fn load_page(
    session: &SqliteSession,
    schema: &str,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    if schema != "main" {
        return Err("SQLite only supports the main schema.".into());
    }
    let connection = session.connection.clone();
    let table = table.to_string();
    let filters = filters.to_vec();
    let sort = sort.cloned();
    blocking(move || {
        let connection = lock(&connection)?;
        load_page_sync(&connection, &table, offset, limit, &filters, sort.as_ref())
    })
    .await
}

pub async fn apply_changes(
    session: &SqliteSession,
    schema: &str,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if schema != "main" {
        return Err("SQLite only supports the main schema.".into());
    }
    let connection = session.connection.clone();
    let table = table.to_string();
    blocking(move || {
        let mut connection = lock(&connection)?;
        apply_changes_sync(&mut connection, &table, changes)
    })
    .await
}

pub async fn run_sql(session: &SqliteSession, sql: &str) -> Result<QueryOutput, String> {
    let connection = session.connection.clone();
    let sql = sql.to_string();
    blocking(move || {
        let connection = lock(&connection)?;
        run_sql_sync(&connection, &sql)
    })
    .await
}

async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("SQLite worker failed: {error}"))?
}

fn lock(
    connection: &Arc<Mutex<Connection>>,
) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
    connection
        .lock()
        .map_err(|_| "SQLite connection lock was poisoned.".to_string())
}

fn list_tables_sync(connection: &Connection) -> Result<Vec<TableSummary>, String> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .map_err(|error| error.to_string())?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    names
        .into_iter()
        .map(|name| {
            let meta = load_meta(connection, &name)?;
            Ok(TableSummary {
                schema: "main".into(),
                name,
                editable: meta.editable(),
            })
        })
        .collect()
}

fn load_page_sync(
    connection: &Connection,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    let meta = load_meta(connection, table)?;
    if meta.columns.is_empty() {
        return Err("Table has no visible columns.".into());
    }

    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let fetch_limit = limit + 1;
    let selected = meta
        .columns
        .iter()
        .map(|column| format!("CAST({} AS TEXT)", quote_identifier(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::new();
    let predicate = build_filter_predicate(&meta, filters, &mut params)?;
    let order = build_order_by(&meta, sort)?;
    let mut sql = format!("SELECT {selected} FROM {}", quote_identifier(table));
    if let Some(predicate) = predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate);
    }
    sql.push_str(&format!(
        " ORDER BY {order} LIMIT {fetch_limit} OFFSET {offset}"
    ));

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params_from_iter(params.iter()))
        .map_err(|error| error.to_string())?;
    let mut page_rows = Vec::with_capacity(fetch_limit as usize);
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let values = (0..meta.columns.len())
            .map(|index| {
                row.get::<_, Option<String>>(index)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let key = meta
            .primary_indices
            .iter()
            .map(|index| values[*index].clone())
            .collect();
        page_rows.push(GridRow {
            version: String::new(),
            key,
            values,
        });
    }

    let has_more = page_rows.len() > limit as usize;
    page_rows.truncate(limit as usize);
    let editable = meta.editable();
    Ok(TablePage {
        columns: meta.columns,
        rows: page_rows,
        offset,
        limit,
        has_more,
        editable,
    })
}

fn apply_changes_sync(
    connection: &mut Connection,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    let meta = load_meta(connection, table)?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }
    let mut applied = 0;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    for change in changes {
        match change {
            GridChange::Insert { values } => {
                let (sql, params) = build_insert(table, &meta, values)?;
                let count = transaction
                    .execute(&sql, params_from_iter(params.iter()))
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
            GridChange::Update {
                values, original, ..
            } => {
                let (sql, params) = build_update(table, &meta, values, original)?;
                let count = transaction
                    .execute(&sql, params_from_iter(params.iter()))
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
            GridChange::Delete { original, .. } => {
                let (sql, params) = build_delete(table, &meta, original)?;
                let count = transaction
                    .execute(&sql, params_from_iter(params.iter()))
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
        }
        applied += 1;
    }
    transaction
        .commit()
        .map_err(|error| format!("COMMIT_OUTCOME_UNKNOWN: {error}"))?;
    Ok(ApplySummary { applied })
}

fn run_sql_sync(connection: &Connection, sql: &str) -> Result<QueryOutput, String> {
    let mut statement = connection.prepare(sql).map_err(|error| match error {
        rusqlite::Error::MultipleStatement => {
            "SQLite runs one statement at a time; remove extra statements.".to_string()
        }
        error => error.to_string(),
    })?;
    let column_count = statement.column_count();
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let readonly = statement.readonly();
    let mut result_sets = Vec::new();
    let mut affected_rows = Vec::new();
    let mut truncated = false;

    if column_count > 0 {
        let mut rows = statement.query([]).map_err(|error| error.to_string())?;
        let mut result_rows = Vec::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            if result_rows.len() == QUERY_ROW_LIMIT {
                truncated = true;
                continue;
            }
            result_rows.push(
                (0..column_count)
                    .map(|index| cell_text(row, index))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        result_sets.push(QueryResultSet {
            columns,
            rows: result_rows,
        });
    } else {
        let count = statement.execute([]).map_err(|error| error.to_string())?;
        affected_rows.push(count as u64);
    }
    drop(statement);
    if column_count > 0 && !readonly {
        let count = connection.changes();
        if count > 0 {
            affected_rows.push(count);
        }
    }
    Ok(QueryOutput {
        result_sets,
        affected_rows,
        truncated,
    })
}

struct TableMeta {
    columns: Vec<ColumnInfo>,
    primary_indices: Vec<usize>,
}

impl TableMeta {
    fn editable(&self) -> bool {
        !self.primary_indices.is_empty()
    }
}

fn load_meta(connection: &Connection, table: &str) -> Result<TableMeta, String> {
    let sql = format!("PRAGMA table_xinfo({})", quote_identifier(table));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let declared: Option<String> = row.get(2)?;
            let declared = declared.unwrap_or_else(|| "ANY".into());
            let not_null: bool = row.get(3)?;
            let pk: i64 = row.get(5)?;
            let hidden: i64 = row.get(6)?;
            Ok(ColumnInfo {
                name,
                filter_type: declared.clone(),
                data_type: declared.clone(),
                filter_kind: filter_kind(&declared),
                sortable: true,
                nullable: !not_null && pk == 0,
                primary_key: pk > 0,
                generated: hidden == 2 || hidden == 3,
                identity: false,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if rows.is_empty() {
        return Err("Table has no visible columns.".into());
    }
    let primary_indices = rows
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| index)
        .collect();
    Ok(TableMeta {
        columns: rows,
        primary_indices,
    })
}

fn filter_kind(declared: &str) -> FilterKind {
    let declared = declared.to_ascii_uppercase();
    if declared.contains("BOOL") {
        FilterKind::Boolean
    } else if declared.contains("INT")
        || declared.contains("REAL")
        || declared.contains("FLOA")
        || declared.contains("DOUB")
        || declared.contains("NUM")
        || declared.contains("DEC")
    {
        FilterKind::Number
    } else if declared.contains("DATE") || declared.contains("TIME") {
        FilterKind::Temporal
    } else if declared.contains("CHAR") || declared.contains("CLOB") || declared.contains("TEXT") {
        FilterKind::Text
    } else {
        FilterKind::Other
    }
}

fn build_filter_predicate(
    meta: &TableMeta,
    filters: &[TableFilter],
    params: &mut Vec<String>,
) -> Result<Option<String>, String> {
    let mut predicates = Vec::with_capacity(filters.len());
    for filter in filters {
        let column = find_column(meta, &filter.column)?;
        ensure_filter_operator(column, filter.operator)?;
        let identifier = quote_identifier(&column.name);
        let value = filter.value.as_deref().filter(|value| !value.is_empty());
        let predicate = match filter.operator {
            FilterOperator::Contains => {
                let value = value
                    .ok_or_else(|| format!("Filter on column {} needs a value.", column.name))?;
                params.push(value.to_string());
                format!("instr(lower(CAST({identifier} AS TEXT)), lower(?)) > 0")
            }
            FilterOperator::Equals
            | FilterOperator::NotEquals
            | FilterOperator::LessThan
            | FilterOperator::LessThanOrEqual
            | FilterOperator::GreaterThan
            | FilterOperator::GreaterThanOrEqual => {
                let value = value
                    .ok_or_else(|| format!("Filter on column {} needs a value.", column.name))?;
                let operator = match filter.operator {
                    FilterOperator::Equals => "IS",
                    FilterOperator::NotEquals => "IS NOT",
                    FilterOperator::LessThan => "<",
                    FilterOperator::LessThanOrEqual => "<=",
                    FilterOperator::GreaterThan => ">",
                    FilterOperator::GreaterThanOrEqual => ">=",
                    _ => unreachable!(),
                };
                params.push(value.to_string());
                format!("{identifier} {operator} ?")
            }
            FilterOperator::IsNull => format!("{identifier} IS NULL"),
            FilterOperator::IsNotNull => format!("{identifier} IS NOT NULL"),
            FilterOperator::IsTrue => {
                format!("({identifier} IS 1 OR lower(CAST({identifier} AS TEXT)) = 'true')")
            }
            FilterOperator::IsFalse => {
                format!("({identifier} IS 0 OR lower(CAST({identifier} AS TEXT)) = 'false')")
            }
        };
        predicates.push(predicate);
    }
    Ok((!predicates.is_empty()).then(|| predicates.join(" AND ")))
}

fn ensure_filter_operator(column: &ColumnInfo, operator: FilterOperator) -> Result<(), String> {
    let allowed = match column.filter_kind {
        FilterKind::Text => matches!(
            operator,
            FilterOperator::Contains
                | FilterOperator::Equals
                | FilterOperator::NotEquals
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Number | FilterKind::Temporal => matches!(
            operator,
            FilterOperator::Equals
                | FilterOperator::NotEquals
                | FilterOperator::LessThan
                | FilterOperator::LessThanOrEqual
                | FilterOperator::GreaterThan
                | FilterOperator::GreaterThanOrEqual
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Boolean => matches!(
            operator,
            FilterOperator::IsTrue
                | FilterOperator::IsFalse
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Other => {
            matches!(operator, FilterOperator::IsNull | FilterOperator::IsNotNull)
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "Filter operator is not valid for column {}.",
            column.name
        ))
    }
}

fn build_order_by(meta: &TableMeta, sort: Option<&TableSort>) -> Result<String, String> {
    let sort_column = sort
        .map(|sort| find_column(meta, &sort.column))
        .transpose()?;
    let mut terms = Vec::with_capacity(meta.columns.len() + 1);
    if let (Some(sort), Some(column)) = (sort, sort_column) {
        if !column.sortable {
            return Err(format!("Column {} cannot be sorted.", column.name));
        }
        terms.push(format!(
            "{} {}",
            quote_identifier(&column.name),
            if sort.descending { "DESC" } else { "ASC" }
        ));
    }
    for (index, column) in meta.columns.iter().enumerate() {
        if meta.primary_indices.contains(&index)
            && sort_column.is_none_or(|sort| sort.name != column.name)
        {
            terms.push(format!("{} ASC", quote_identifier(&column.name)));
        }
    }
    if meta.primary_indices.is_empty() {
        terms.push("rowid".into());
    }
    if terms.is_empty() {
        terms.push("rowid".into());
    }
    Ok(terms.join(", "))
}

fn build_insert(
    table: &str,
    meta: &TableMeta,
    values: BTreeMap<String, Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    let mut names = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());
    let mut params = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = find_column(meta, &name)?;
        ensure_writable(column)?;
        names.push(quote_identifier(&name));
        placeholders.push("?");
        params.push(value);
    }
    let sql = if names.is_empty() {
        format!("INSERT INTO {} DEFAULT VALUES", quote_identifier(table))
    } else {
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(table),
            names.join(", "),
            placeholders.join(", ")
        )
    };
    Ok((sql, params))
}

fn build_update(
    table: &str,
    meta: &TableMeta,
    values: BTreeMap<String, Option<String>>,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    if values.is_empty() {
        return Err("Update contains no changed values.".into());
    }
    if original.len() != meta.columns.len() {
        return Err("Update is missing the original row values.".into());
    }
    let mut params = Vec::with_capacity(values.len() + meta.columns.len());
    let mut assignments = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = find_column(meta, &name)?;
        ensure_writable(column)?;
        assignments.push(format!("{} = ?", quote_identifier(&name)));
        params.push(value);
    }
    let predicate = original_predicate(meta, original, &mut params);
    Ok((
        format!(
            "UPDATE {} SET {} WHERE {predicate}",
            quote_identifier(table),
            assignments.join(", ")
        ),
        params,
    ))
}

fn build_delete(
    table: &str,
    meta: &TableMeta,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    if original.len() != meta.columns.len() {
        return Err("Delete is missing the original row values.".into());
    }
    let mut params = Vec::with_capacity(meta.columns.len());
    let predicate = original_predicate(meta, original, &mut params);
    Ok((
        format!("DELETE FROM {} WHERE {predicate}", quote_identifier(table)),
        params,
    ))
}

fn original_predicate(
    meta: &TableMeta,
    original: Vec<Option<String>>,
    params: &mut Vec<Option<String>>,
) -> String {
    let mut predicates = Vec::with_capacity(meta.columns.len());
    for index in &meta.primary_indices {
        params.push(original[*index].clone());
        predicates.push(format!(
            "{} IS ?",
            quote_identifier(&meta.columns[*index].name)
        ));
    }
    for (index, column) in meta.columns.iter().enumerate() {
        if meta.primary_indices.contains(&index) {
            continue;
        }
        params.push(original[index].clone());
        predicates.push(format!(
            "CAST({} AS TEXT) IS ?",
            quote_identifier(&column.name)
        ));
    }
    predicates.join(" AND ")
}

fn find_column<'a>(meta: &'a TableMeta, name: &str) -> Result<&'a ColumnInfo, String> {
    meta.columns
        .iter()
        .find(|column| column.name == name)
        .ok_or_else(|| format!("Unknown table column: {name}"))
}

fn ensure_writable(column: &ColumnInfo) -> Result<(), String> {
    if column.generated {
        return Err(format!(
            "Column {} is generated and cannot be edited.",
            column.name
        ));
    }
    Ok(())
}

fn ensure_one_row(count: usize) -> Result<(), String> {
    if count == 1 {
        Ok(())
    } else {
        Err(
            "A row changed or disappeared since it was loaded. The staged batch was rolled back."
                .into(),
        )
    }
}

fn cell_text(row: &rusqlite::Row<'_>, index: usize) -> Result<Option<String>, String> {
    match row.get_ref(index).map_err(|error| error.to_string())? {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(value) => Ok(Some(value.to_string())),
        ValueRef::Real(value) => Ok(Some(value.to_string())),
        ValueRef::Text(value) => Ok(Some(String::from_utf8_lossy(value).into_owned())),
        ValueRef::Blob(value) => Ok(Some(format!(
            "\\x{}",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))),
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FilterOperator, GridChange, TableFilter, TableSort};
    use std::collections::BTreeMap;

    fn memory_session() -> SqliteSession {
        let connection = Connection::open_in_memory().expect("open in-memory sqlite");
        connection
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT);
                 INSERT INTO items (name, note) VALUES ('Alice', 'one'), ('Bob', NULL), ('Charlie', 'three');",
            )
            .expect("create sqlite fixture");
        let interrupt = Arc::new(connection.get_interrupt_handle());
        SqliteSession {
            connection: Arc::new(Mutex::new(connection)),
            interrupt,
        }
    }

    #[tokio::test]
    async fn filters_sorts_and_rejects_stale_edits() {
        let session = memory_session();
        let tables = list_tables(&session).await.expect("list tables");
        assert_eq!(tables.len(), 1);
        assert!(tables[0].editable);

        let page = load_page(
            &session,
            "main",
            "items",
            0,
            100,
            &[TableFilter {
                column: "name".into(),
                operator: FilterOperator::Contains,
                value: Some("li".into()),
            }],
            Some(&TableSort {
                column: "name".into(),
                descending: false,
            }),
        )
        .await
        .expect("load filtered page");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[1].as_deref(), Some("Alice"));
        assert_eq!(page.rows[1].values[1].as_deref(), Some("Charlie"));

        let original = page.rows[0].values.clone();
        let key = page.rows[0].key.clone();
        let mut values = BTreeMap::new();
        values.insert("name".to_string(), Some("Alicia".to_string()));
        apply_changes(
            &session,
            "main",
            "items",
            vec![GridChange::Update {
                key: key.clone(),
                version: String::new(),
                values,
                original: original.clone(),
            }],
        )
        .await
        .expect("apply update");

        let stale = apply_changes(
            &session,
            "main",
            "items",
            vec![GridChange::Update {
                key,
                version: String::new(),
                values: BTreeMap::from([("name".to_string(), Some("Stale".to_string()))]),
                original,
            }],
        )
        .await;
        assert!(stale.is_err(), "stale update must roll back");
    }

    #[tokio::test]
    async fn opens_file_database() {
        let path = std::env::temp_dir().join(format!("koyo-{}.db", uuid::Uuid::new_v4()));
        let (session, version) = connect(path.to_str().expect("utf-8 path")).expect("open file");
        assert!(!version.is_empty());
        {
            let connection = lock(&session.connection).expect("lock sqlite");
            connection
                .execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)")
                .expect("create file table");
        }
        let tables = list_tables(&session).await.expect("list file tables");
        assert_eq!(tables[0].name, "notes");
        drop(session);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn runs_one_sqlite_statement() {
        let session = memory_session();
        let output = run_sql(&session, "SELECT id, name FROM items ORDER BY id")
            .await
            .expect("run select");
        assert_eq!(output.result_sets.len(), 1);
        assert_eq!(output.result_sets[0].columns, vec!["id", "name"]);
        assert_eq!(output.result_sets[0].rows.len(), 3);
        assert_eq!(output.result_sets[0].rows[0][1].as_deref(), Some("Alice"));

        let output = run_sql(&session, "INSERT INTO items (name) VALUES ('Zoe')")
            .await
            .expect("run insert");
        assert_eq!(output.affected_rows, vec![1]);
        assert!(run_sql(&session, "SELECT 1; SELECT 2").await.is_err());
    }
}
