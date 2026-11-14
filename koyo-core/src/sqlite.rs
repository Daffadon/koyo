use crate::common::{
    self, QUERY_ROW_LIMIT, TABLE_PAGE_LIMIT, TableMeta, ensure_filter_operator, ensure_writable,
    find_column, quote_identifier,
};
use crate::models::{
    ApplySummary, ColumnCatalogEntry, ColumnInfo, FilterKind, FilterOperator, ForeignKeyInfo,
    GridChange, GridRow, MetadataGroup, MetadataItem, QueryOutput, QueryResultSet, SchemaObject,
    SchemaObjectKind, TableFilter, TablePage, TableSort, TableSummary,
};
use rusqlite::{
    Connection, InterruptHandle, TransactionBehavior, params_from_iter, types::ValueRef,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|error| format!("Could not enable SQLite foreign keys: {error}"))?;
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

pub async fn list_columns(session: &SqliteSession) -> Result<Vec<ColumnCatalogEntry>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let mut tables = connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .map_err(|error| error.to_string())?;
        let names = tables
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(tables);
        let mut columns = Vec::new();
        for table in names {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_xinfo({})", quote_identifier(&table)))
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|error| error.to_string())?;
            for column in rows {
                columns.push(ColumnCatalogEntry {
                    schema: "main".to_string(),
                    table: table.clone(),
                    column: column.map_err(|error| error.to_string())?,
                });
            }
        }
        Ok(columns)
    })
    .await
}

pub async fn list_schema_objects(session: &SqliteSession) -> Result<Vec<SchemaObject>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let mut statement = connection
            .prepare(
                "SELECT m.name, m.type
                 FROM sqlite_schema m
                 WHERE m.type IN ('table', 'view', 'trigger')
                   AND m.name NOT LIKE 'sqlite_%'
                 ORDER BY m.type, m.name",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.into_iter()
            .map(|(name, kind)| {
                let kind = match kind.as_str() {
                    "table" => SchemaObjectKind::Table,
                    "view" => SchemaObjectKind::View,
                    "trigger" => SchemaObjectKind::Trigger,
                    other => return Err(format!("Unexpected SQLite object type: {other}")),
                };
                Ok(SchemaObject {
                    schema: "main".into(),
                    name,
                    kind,
                })
            })
            .collect()
    })
    .await
}

pub async fn list_foreign_keys(
    session: &SqliteSession,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    if schema != "main" {
        return Err("SQLite only supports the main schema.".into());
    }
    let connection = session.connection.clone();
    let table = table.to_string();
    blocking(move || {
        let connection = lock(&connection)?;
        list_foreign_keys_sync(&connection, &table)
    })
    .await
}

pub async fn list_database_metadata(session: &SqliteSession) -> Result<Vec<MetadataGroup>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let encoding: String = connection
            .query_row("PRAGMA encoding", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let file: String = connection
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name = 'main'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let file = if file.is_empty() {
            "in-memory".to_string()
        } else {
            file
        };
        Ok(vec![MetadataGroup {
            name: "System Info".into(),
            items: vec![
                MetadataItem {
                    name: "Version".into(),
                    detail: version,
                },
                MetadataItem {
                    name: "Database File".into(),
                    detail: file,
                },
                MetadataItem {
                    name: "Encoding".into(),
                    detail: encoding,
                },
            ],
        }])
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
            "SELECT m.name, i.name, i.type, i.\"notnull\", i.pk, i.hidden
             FROM sqlite_schema m
             JOIN pragma_table_xinfo(m.name) i
             WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%'
             ORDER BY m.name, i.cid",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, xinfo_column(row, 1, 2, 3, 4, 5)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut tables: Vec<(String, Vec<ColumnInfo>)> = Vec::new();
    for (table, column) in rows {
        match tables.last_mut() {
            Some((name, columns)) if *name == table => columns.push(column),
            _ => tables.push((table, vec![column])),
        }
    }
    Ok(tables
        .into_iter()
        .map(|(name, columns)| TableSummary {
            schema: "main".into(),
            editable: TableMeta::new(columns, None).editable(),
            name,
        })
        .collect())
}

fn list_foreign_keys_sync(
    connection: &Connection,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let mut statement = connection
        .prepare(
            "SELECT f.id, f.seq, f.\"table\", f.\"from\",
                    COALESCE(
                        f.\"to\",
                        (
                            SELECT i.name
                            FROM pragma_table_info(f.\"table\") i
                            WHERE i.pk = f.seq + 1
                        ),
                        ''
                    )
             FROM pragma_foreign_key_list(?1) f
             ORDER BY f.id, f.seq",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([table], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(
            |(id, referenced_table, column, referenced_column)| ForeignKeyInfo {
                schema: "main".into(),
                table: table.to_string(),
                column,
                referenced_schema: "main".into(),
                referenced_table,
                referenced_column,
                constraint_name: format!("fk_{table}_{id}"),
            },
        )
        .collect())
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
        .map(|column| {
            let identifier = quote_identifier(&column.name);
            if is_binary(column) {
                format!("hex({identifier})")
            } else {
                format!("CAST({identifier} AS TEXT)")
            }
        })
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
            .map(|index| cell_text(row, index))
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
                common::ensure_one_row(count as u64)?;
            }
            GridChange::Update {
                values, original, ..
            } => {
                let (sql, params) = build_update(table, &meta, values, original)?;
                let count = transaction
                    .execute(&sql, params_from_iter(params.iter()))
                    .map_err(|error| error.to_string())?;
                common::ensure_one_row(count as u64)?;
            }
            GridChange::Delete { original, .. } => {
                let (sql, params) = build_delete(table, &meta, original)?;
                let count = transaction
                    .execute(&sql, params_from_iter(params.iter()))
                    .map_err(|error| error.to_string())?;
                common::ensure_one_row(count as u64)?;
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

fn load_meta(connection: &Connection, table: &str) -> Result<TableMeta, String> {
    let sql = format!("PRAGMA table_xinfo({})", quote_identifier(table));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| xinfo_column(row, 1, 2, 3, 5, 6))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if rows.is_empty() {
        return Err("Table has no visible columns.".into());
    }
    Ok(TableMeta::new(rows, None))
}

fn xinfo_column(
    row: &rusqlite::Row<'_>,
    name_index: usize,
    type_index: usize,
    not_null_index: usize,
    pk_index: usize,
    hidden_index: usize,
) -> rusqlite::Result<ColumnInfo> {
    let name: String = row.get(name_index)?;
    let declared: Option<String> = row.get(type_index)?;
    let declared = declared.unwrap_or_else(|| "ANY".into());
    let not_null: bool = row.get(not_null_index)?;
    let pk: i64 = row.get(pk_index)?;
    let hidden: i64 = row.get(hidden_index)?;
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
}

fn binary_literal(value: &Option<String>) -> Result<String, String> {
    let Some(hex) = value else {
        return Ok("NULL".to_string());
    };
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Binary values must be an even number of hexadecimal digits.".into());
    }
    Ok(format!("x'{hex}'"))
}

fn is_binary(column: &ColumnInfo) -> bool {
    let declared = column.filter_type.to_ascii_uppercase();
    declared.contains("BLOB") || declared.contains("BINARY") || declared.contains("BYTEA")
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
        let column = find_column(&meta.columns, &filter.column)?;
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

fn build_order_by(meta: &TableMeta, sort: Option<&TableSort>) -> Result<String, String> {
    let sort_column = sort
        .map(|sort| find_column(&meta.columns, &sort.column))
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
        let column = find_column(&meta.columns, &name)?;
        ensure_writable(column)?;
        names.push(quote_identifier(&name));
        if is_binary(column) {
            placeholders.push(binary_literal(&value)?);
        } else {
            placeholders.push("?".to_string());
            params.push(value);
        }
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
        let column = find_column(&meta.columns, &name)?;
        ensure_writable(column)?;
        let expression = if is_binary(column) {
            binary_literal(&value)?
        } else {
            params.push(value);
            "?".to_string()
        };
        assignments.push(format!("{} = {expression}", quote_identifier(&name)));
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
        let column = &meta.columns[*index];
        let identifier = quote_identifier(&column.name);
        let expression = if is_binary(column) {
            format!("hex({identifier})")
        } else {
            format!("CAST({identifier} AS TEXT)")
        };
        predicates.push(format!("{expression} IS ?"));
    }
    for (index, column) in meta.columns.iter().enumerate() {
        if meta.primary_indices.contains(&index) {
            continue;
        }
        params.push(original[index].clone());
        let expression = if is_binary(column) {
            format!("hex({})", quote_identifier(&column.name))
        } else {
            format!("CAST({} AS TEXT)", quote_identifier(&column.name))
        };
        predicates.push(format!("{expression} IS ?"));
    }
    predicates.join(" AND ")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FilterOperator, GridChange, SchemaObjectKind, TableFilter, TableSort};
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
    async fn lists_schema_objects_and_foreign_keys() {
        let connection = Connection::open_in_memory().expect("open in-memory sqlite");
        connection
            .execute_batch(
                "CREATE TABLE parents (id INTEGER PRIMARY KEY);
                 CREATE TABLE children (
                     id INTEGER PRIMARY KEY,
                     parent_id INTEGER REFERENCES parents(id),
                     manager_id INTEGER REFERENCES children(id)
                 );
                 CREATE VIEW parent_names AS SELECT id FROM parents;
                 CREATE TRIGGER parents_insert AFTER INSERT ON parents
                 BEGIN SELECT 1; END;",
            )
            .expect("create sqlite fixture");
        let interrupt = Arc::new(connection.get_interrupt_handle());
        let session = SqliteSession {
            connection: Arc::new(Mutex::new(connection)),
            interrupt,
        };

        let objects = list_schema_objects(&session)
            .await
            .expect("list schema objects");
        assert!(objects.iter().any(|object| object.schema == "main"
            && object.name == "parents"
            && object.kind == SchemaObjectKind::Table));
        assert!(
            objects.iter().any(
                |object| object.name == "parent_names" && object.kind == SchemaObjectKind::View
            )
        );
        assert!(
            objects.iter().any(|object| object.name == "parents_insert"
                && object.kind == SchemaObjectKind::Trigger)
        );
        assert!(
            !objects
                .iter()
                .any(|object| object.kind == SchemaObjectKind::Function)
        );

        let keys = list_foreign_keys(&session, "main", "children")
            .await
            .expect("list foreign keys");
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().any(|key| key.column == "parent_id"
            && key.referenced_table == "parents"
            && key.referenced_column == "id"));
        assert!(keys.iter().any(|key| key.column == "manager_id"
            && key.referenced_table == "children"
            && key.referenced_column == "id"));

        let groups = list_database_metadata(&session)
            .await
            .expect("list database metadata");
        assert!(!groups.iter().any(|group| group.name == "Schemas"));
        assert!(groups.iter().all(|group| !group.items.is_empty()));
        let info = groups
            .iter()
            .find(|group| group.name == "System Info")
            .expect("system info group");
        assert!(
            info.items
                .iter()
                .any(|item| item.name == "Version" && !item.detail.is_empty())
        );
        assert!(info.items.iter().any(|item| item.name == "Encoding"));
        assert!(
            info.items
                .iter()
                .any(|item| item.name == "Database File" && item.detail == "in-memory")
        );
    }

    #[tokio::test]
    async fn lists_columns_for_completion() {
        let session = memory_session();
        let columns = list_columns(&session).await.expect("list columns");
        assert!(columns.iter().any(|column| {
            column.schema == "main" && column.table == "items" && column.column == "name"
        }));
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
    async fn updates_rows_with_whole_real_and_blob_values() {
        let connection = Connection::open_in_memory().expect("open in-memory sqlite");
        connection
            .execute_batch(
                "CREATE TABLE measures (id INTEGER PRIMARY KEY, amount REAL, payload BLOB, note TEXT);
                 INSERT INTO measures VALUES (1, 1.0 / 3.0, x'00ff', 'old');",
            )
            .expect("create sqlite fixture");
        let interrupt = Arc::new(connection.get_interrupt_handle());
        let session = SqliteSession {
            connection: Arc::new(Mutex::new(connection)),
            interrupt,
        };
        let page = load_page(&session, "main", "measures", 0, 100, &[], None)
            .await
            .expect("load page");
        assert_eq!(
            page.rows[0].values[1].as_deref(),
            Some("0.33333333333333332")
        );
        assert_eq!(page.rows[0].values[2].as_deref(), Some("00FF"));

        apply_changes(
            &session,
            "main",
            "measures",
            vec![GridChange::Update {
                key: page.rows[0].key.clone(),
                version: String::new(),
                values: BTreeMap::from([
                    ("note".to_string(), Some("new".to_string())),
                    ("payload".to_string(), Some("DEADBEEF".to_string())),
                ]),
                original: page.rows[0].values.clone(),
            }],
        )
        .await
        .expect("apply update");

        let (kind, hex): (String, String) = {
            let connection = lock(&session.connection).expect("lock sqlite");
            connection
                .query_row(
                    "SELECT typeof(payload), hex(payload) FROM measures WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("read updated blob")
        };
        assert_eq!(kind, "blob");
        assert_eq!(hex, "DEADBEEF");

        let invalid = apply_changes(
            &session,
            "main",
            "measures",
            vec![GridChange::Update {
                key: page.rows[0].key.clone(),
                version: String::new(),
                values: BTreeMap::from([("payload".to_string(), Some("xyz".to_string()))]),
                original: page.rows[0].values.clone(),
            }],
        )
        .await;
        assert!(invalid.is_err(), "invalid binary input must be rejected");

        let page = load_page(&session, "main", "measures", 0, 100, &[], None)
            .await
            .expect("reload page");
        assert_eq!(page.rows[0].values[3].as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn opens_file_database() {
        let path = std::env::temp_dir().join(format!("koyo-{}.db", uuid::Uuid::new_v4()));
        let (session, version) = connect(path.to_str().expect("utf-8 path")).expect("open file");
        assert!(!version.is_empty());
        {
            let connection = lock(&session.connection).expect("lock sqlite");
            let foreign_keys: i64 = connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .expect("read foreign_keys pragma");
            assert_eq!(foreign_keys, 1);
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
