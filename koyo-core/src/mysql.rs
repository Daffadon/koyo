use crate::common::{self, QUERY_ROW_LIMIT, TABLE_PAGE_LIMIT};
use crate::models::{
    ApplySummary, ColumnCatalogEntry, ColumnInfo, ConnectionProfile, FilterOperator,
    ForeignKeyInfo, GridChange, GridRow, MetadataGroup, MetadataItem, QueryOutput, QueryResultSet,
    SchemaObject, SchemaObjectKind, SslMode, TableFilter, TablePage, TableSort, TableSummary,
};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, Params, Row as MysqlRow, SslOpts, TxOpts, Value};
use std::collections::BTreeMap;

const SYSTEM_SCHEMAS: &str = "('information_schema', 'mysql', 'performance_schema', 'sys')";

pub struct MysqlSession {
    pub conn: Conn,
    pub connection_id: u32,
    pub transaction_open: bool,
}

pub async fn connect(
    profile: &ConnectionProfile,
    password: &str,
) -> Result<(MysqlSession, String, String, Opts), String> {
    let mut builder = OptsBuilder::default()
        .ip_or_hostname(profile.host.trim())
        .tcp_port(profile.port)
        .user(Some(profile.username.trim()))
        .pass(Some(password))
        .db_name(Some(profile.database.trim()))
        .prefer_socket(false)
        .stmt_cache_size(0usize)
        .client_found_rows(true);
    if profile.ssl_mode == SslMode::VerifyFull {
        builder = builder.ssl_opts(Some(SslOpts::default()));
    } else {
        builder = builder.ssl_opts(None);
    }
    let opts = Opts::from(builder);
    let mut conn = Conn::new(opts.clone())
        .await
        .map_err(|error| format!("MySQL connection failed: {error}"))?;
    let (major, minor, patch) = conn.server_version();
    let server_version = format!("{major}.{minor}.{patch}");
    let current_user = conn
        .query_first::<String, _>("SELECT CURRENT_USER()")
        .await
        .map_err(|error| format!("Could not read MySQL current user: {error}"))?
        .unwrap_or_else(|| profile.username.clone());
    let connection_id = conn.id();
    Ok((
        MysqlSession {
            conn,
            connection_id,
            transaction_open: false,
        },
        server_version,
        current_user,
        opts,
    ))
}

pub async fn list_tables(session: &mut MysqlSession) -> Result<Vec<TableSummary>, String> {
    let sql = format!(
        "SELECT t.TABLE_SCHEMA, t.TABLE_NAME,
                EXISTS (
                    SELECT 1 FROM information_schema.STATISTICS s
                    WHERE s.TABLE_SCHEMA = t.TABLE_SCHEMA
                      AND s.TABLE_NAME = t.TABLE_NAME
                      AND s.INDEX_NAME = 'PRIMARY'
                )
         FROM information_schema.TABLES t
         WHERE t.TABLE_TYPE = 'BASE TABLE'
           AND t.TABLE_SCHEMA NOT IN {SYSTEM_SCHEMAS}
         ORDER BY t.TABLE_SCHEMA, t.TABLE_NAME"
    );
    let rows: Vec<MysqlRow> = session
        .conn
        .query(sql)
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            Ok(TableSummary {
                schema: row_string(&row, 0)?,
                name: row_string(&row, 1)?,
                editable: row.get::<u8, _>(2).unwrap_or(0) != 0,
            })
        })
        .collect()
}

pub async fn list_columns(session: &mut MysqlSession) -> Result<Vec<ColumnCatalogEntry>, String> {
    let rows: Vec<MysqlRow> = session
        .conn
        .query(
            "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME
             FROM information_schema.COLUMNS
             WHERE TABLE_SCHEMA = DATABASE()
             ORDER BY TABLE_NAME, ORDINAL_POSITION",
        )
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            Ok(ColumnCatalogEntry {
                schema: row_string(&row, 0)?,
                table: row_string(&row, 1)?,
                column: row_string(&row, 2)?,
            })
        })
        .collect()
}

pub async fn list_schema_objects(session: &mut MysqlSession) -> Result<Vec<SchemaObject>, String> {
    let sql = format!(
        "SELECT t.TABLE_SCHEMA, t.TABLE_NAME, 'table'
         FROM information_schema.TABLES t
         WHERE t.TABLE_TYPE = 'BASE TABLE'
           AND t.TABLE_SCHEMA NOT IN {SYSTEM_SCHEMAS}
         UNION ALL
         SELECT v.TABLE_SCHEMA, v.TABLE_NAME, 'view'
         FROM information_schema.VIEWS v
         WHERE v.TABLE_SCHEMA NOT IN {SYSTEM_SCHEMAS}
         UNION ALL
         SELECT r.ROUTINE_SCHEMA, r.ROUTINE_NAME, 'function'
         FROM information_schema.ROUTINES r
         WHERE r.ROUTINE_TYPE = 'FUNCTION'
           AND r.ROUTINE_SCHEMA NOT IN {SYSTEM_SCHEMAS}
         UNION ALL
         SELECT g.TRIGGER_SCHEMA, g.TRIGGER_NAME, 'trigger'
         FROM information_schema.TRIGGERS g
         WHERE g.TRIGGER_SCHEMA NOT IN {SYSTEM_SCHEMAS}
         ORDER BY 1, 2, 3"
    );
    let rows: Vec<MysqlRow> = session
        .conn
        .query(sql)
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            let kind = row_string(&row, 2)?;
            Ok(SchemaObject {
                schema: row_string(&row, 0)?,
                name: row_string(&row, 1)?,
                kind: match kind.as_str() {
                    "table" => SchemaObjectKind::Table,
                    "view" => SchemaObjectKind::View,
                    "function" => SchemaObjectKind::Function,
                    "trigger" => SchemaObjectKind::Trigger,
                    other => return Err(format!("Unexpected MySQL object type: {other}")),
                },
            })
        })
        .collect()
}

pub async fn list_foreign_keys(
    session: &mut MysqlSession,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let sql = format!(
        "SELECT k.CONSTRAINT_SCHEMA, k.TABLE_NAME, k.COLUMN_NAME,
                k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME,
                k.REFERENCED_COLUMN_NAME, k.CONSTRAINT_NAME
         FROM information_schema.KEY_COLUMN_USAGE k
         JOIN information_schema.REFERENTIAL_CONSTRAINTS r
           ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA
          AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME
         WHERE k.REFERENCED_TABLE_NAME IS NOT NULL
           AND k.TABLE_SCHEMA NOT IN {SYSTEM_SCHEMAS}
           AND ((k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ?)
                OR (k.REFERENCED_TABLE_SCHEMA = ? AND k.REFERENCED_TABLE_NAME = ?))
         ORDER BY k.TABLE_SCHEMA, k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION"
    );
    let params = Params::Positional(vec![
        Value::from(schema),
        Value::from(table),
        Value::from(schema),
        Value::from(table),
    ]);
    let rows: Vec<MysqlRow> = session
        .conn
        .exec(sql, params)
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            Ok(ForeignKeyInfo {
                schema: row_string(&row, 0)?,
                table: row_string(&row, 1)?,
                column: row_string(&row, 2)?,
                referenced_schema: row_string(&row, 3)?,
                referenced_table: row_string(&row, 4)?,
                referenced_column: row_string(&row, 5)?,
                constraint_name: row_string(&row, 6)?,
            })
        })
        .collect()
}

pub async fn list_database_metadata(
    session: &mut MysqlSession,
) -> Result<Vec<MetadataGroup>, String> {
    let mut groups = Vec::new();

    let rows: Vec<MysqlRow> = session
        .conn
        .query("SELECT User, Host FROM mysql.user ORDER BY User, Host LIMIT 200")
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            Ok(MetadataItem {
                name: row_string(&row, 0)?,
                detail: format!("@{}", row_string(&row, 1)?),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !items.is_empty() {
        groups.push(MetadataGroup {
            name: "Users".into(),
            items,
        });
    }

    let rows: Vec<MysqlRow> = session
        .conn
        .query(
            "SELECT ENGINE, COALESCE(SUPPORT, 'unknown'),
                    COALESCE(TRANSACTIONS, 'NO')
             FROM information_schema.ENGINES
             ORDER BY ENGINE",
        )
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            Ok(MetadataItem {
                name: row_string(&row, 0)?,
                detail: format!(
                    "{}, transactions {}",
                    row_string(&row, 1)?,
                    row_string(&row, 2)?
                ),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !items.is_empty() {
        groups.push(MetadataGroup {
            name: "Storage Engines".into(),
            items,
        });
    }

    let row: MysqlRow = session
        .conn
        .query_first(
            "SELECT VERSION(), DATABASE(), CURRENT_USER(),
                    @@character_set_database, @@collation_database",
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "MySQL returned no server details.".to_string())?;
    groups.push(MetadataGroup {
        name: "System Info".into(),
        items: vec![
            MetadataItem {
                name: "Version".into(),
                detail: row_string(&row, 0)?,
            },
            MetadataItem {
                name: "Database".into(),
                detail: row_string(&row, 1)?,
            },
            MetadataItem {
                name: "Current User".into(),
                detail: row_string(&row, 2)?,
            },
            MetadataItem {
                name: "Character Set".into(),
                detail: format!(
                    "{}, collation {}",
                    row_string(&row, 3)?,
                    row_string(&row, 4)?
                ),
            },
        ],
    });

    Ok(groups)
}

pub async fn load_page(
    session: &mut MysqlSession,
    schema: &str,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    let meta = load_meta(session, schema, table).await?;
    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let selected = meta
        .columns
        .iter()
        .map(|column| {
            let identifier = quote_identifier(&column.name);
            if is_binary(column) {
                format!("HEX({identifier})")
            } else {
                format!("CAST({identifier} AS CHAR)")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::new();
    let predicate = build_filter_predicate(&meta.columns, filters, &mut params)?;
    let order = build_order_by(&meta.columns, meta.primary_indices.as_slice(), sort)?;
    let mut sql = format!("SELECT {selected} FROM {}", quote_relation(schema, table));
    if let Some(predicate) = predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate);
    }
    sql.push_str(&format!(
        " ORDER BY {order} LIMIT {} OFFSET {offset}",
        limit + 1
    ));

    let rows: Vec<MysqlRow> = session
        .conn
        .exec(&sql, Params::Positional(params))
        .await
        .map_err(|error| error.to_string())?;
    let mut page_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let values = (0..meta.columns.len())
            .map(|index| row.as_ref(index).and_then(value_text))
            .collect::<Vec<_>>();
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
    Ok(TablePage {
        editable: meta.editable(),
        columns: meta.columns,
        rows: page_rows,
        offset,
        limit,
        has_more,
    })
}

pub async fn apply_changes(
    session: &mut MysqlSession,
    schema: &str,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if has_open_transaction(session).await? {
        return Err(
            "Commit or roll back the SQL tab's transaction before applying grid edits.".into(),
        );
    }
    let meta = load_meta(session, schema, table).await?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }
    let mut transaction = session
        .conn
        .start_transaction(TxOpts::default())
        .await
        .map_err(|error| error.to_string())?;
    let mut applied = 0;
    for change in changes {
        let result = match change {
            GridChange::Insert { values } => {
                let (sql, params) = build_insert(schema, table, &meta, values)?;
                transaction
                    .exec_drop(&sql, Params::Positional(params))
                    .await
                    .map(|_| transaction.affected_rows())
                    .map_err(|error| error.to_string())
            }
            GridChange::Update {
                values, original, ..
            } => {
                let (sql, params) = build_update(schema, table, &meta, values, original)?;
                transaction
                    .exec_drop(&sql, Params::Positional(params))
                    .await
                    .map(|_| transaction.affected_rows())
                    .map_err(|error| error.to_string())
            }
            GridChange::Delete { original, .. } => {
                let (sql, params) = build_delete(schema, table, &meta, original)?;
                transaction
                    .exec_drop(&sql, Params::Positional(params))
                    .await
                    .map(|_| transaction.affected_rows())
                    .map_err(|error| error.to_string())
            }
        };
        match result {
            Ok(count) => {
                if let Err(error) = common::ensure_one_row(count) {
                    let _ = transaction.rollback().await;
                    session.transaction_open = false;
                    return Err(error);
                }
                applied += 1;
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                session.transaction_open = false;
                return Err(error);
            }
        }
    }
    session.transaction_open = false;
    transaction
        .commit()
        .await
        .map_err(|error| format!("COMMIT_OUTCOME_UNKNOWN: {error}"))?;
    Ok(ApplySummary { applied })
}

pub async fn run_sql(session: &mut MysqlSession, sql: &str) -> Result<QueryOutput, String> {
    let mut result = session
        .conn
        .query_iter(sql)
        .await
        .map_err(|error| error.to_string())?;
    update_transaction_flag(sql, &mut session.transaction_open);
    let mut result_sets = Vec::new();
    let mut affected_rows = Vec::new();
    let mut truncated = false;
    loop {
        let columns = result.columns_ref().to_vec();
        if columns.is_empty() {
            let affected = result.affected_rows();
            if affected > 0 {
                affected_rows.push(affected);
            }
            result.next().await.map_err(|error| error.to_string())?;
        } else {
            let column_names = columns
                .iter()
                .map(|column| column.name_str().into_owned())
                .collect::<Vec<_>>();
            let mut rows = Vec::new();
            while let Some(row) = result.next().await.map_err(|error| error.to_string())? {
                if rows.len() == QUERY_ROW_LIMIT {
                    truncated = true;
                    continue;
                }
                rows.push(
                    (0..columns.len())
                        .map(|index| row.as_ref(index).and_then(value_text))
                        .collect(),
                );
            }
            result_sets.push(QueryResultSet {
                columns: column_names,
                rows,
            });
        }
        if result.is_empty() {
            break;
        }
    }
    result
        .drop_result()
        .await
        .map_err(|error| error.to_string())?;
    Ok(QueryOutput {
        result_sets,
        affected_rows,
        truncated,
    })
}

pub async fn has_open_transaction(session: &mut MysqlSession) -> Result<bool, String> {
    if session.transaction_open {
        return Ok(true);
    }
    if let Ok(Some(value)) = session
        .conn
        .query_first::<u8, _>("SELECT @@in_transaction")
        .await
    {
        return Ok(value != 0);
    }
    if let Ok(Some(value)) = session
        .conn
        .query_first::<u8, _>("SELECT @@autocommit")
        .await
    {
        if value == 0 {
            return Ok(true);
        }
    }
    let value = session
        .conn
        .query_first::<u64, _>(
            "SELECT COUNT(*) FROM information_schema.innodb_trx
             WHERE trx_mysql_thread_id = CONNECTION_ID()",
        )
        .await
        .map_err(|error| format!("Could not read MySQL transaction state: {error}"))?
        .unwrap_or(0);
    Ok(value > 0)
}

fn update_transaction_flag(sql: &str, open: &mut bool) {
    let statement = sql
        .trim_start()
        .trim_start_matches('(')
        .trim_start()
        .to_ascii_lowercase();
    if statement.starts_with("commit") || statement.starts_with("rollback") {
        *open = false;
    } else if statement.starts_with("begin") || statement.starts_with("start transaction") {
        *open = true;
    } else if statement.starts_with("set") && statement.contains("autocommit") {
        *open = statement.contains("autocommit=0") || statement.contains("autocommit = 0");
    }
}

pub async fn cancel_query(opts: &Opts, connection_id: u32) -> Result<(), String> {
    let mut cancel = Conn::new(opts.clone())
        .await
        .map_err(|error| format!("Could not open MySQL cancellation connection: {error}"))?;
    cancel
        .query_drop(format!("KILL QUERY {connection_id}"))
        .await
        .map_err(|error| format!("Could not cancel MySQL query: {error}"))
}

struct MysqlMeta {
    columns: Vec<ColumnInfo>,
    primary_indices: Vec<usize>,
}

impl MysqlMeta {
    fn editable(&self) -> bool {
        !self.primary_indices.is_empty()
    }
}

async fn load_meta(
    session: &mut MysqlSession,
    schema: &str,
    table: &str,
) -> Result<MysqlMeta, String> {
    let rows: Vec<MysqlRow> = session
        .conn
        .exec(
            "SELECT COLUMN_NAME, COLUMN_TYPE, DATA_TYPE, IS_NULLABLE, COLUMN_KEY,
                    EXTRA, COALESCE(GENERATION_EXPRESSION, '')
             FROM information_schema.COLUMNS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION",
            Params::Positional(vec![
                Value::Bytes(schema.as_bytes().to_vec()),
                Value::Bytes(table.as_bytes().to_vec()),
            ]),
        )
        .await
        .map_err(|error| error.to_string())?;
    if rows.is_empty() {
        return Err("Table has no visible columns.".into());
    }
    let columns = rows
        .into_iter()
        .map(|row| {
            let data_type = row_string(&row, 1)?;
            let base_type = row_string(&row, 2)?;
            let extra = row_string(&row, 5)?;
            let generated_expression = row_string(&row, 6)?;
            let nullable = row_string(&row, 3)?.eq_ignore_ascii_case("YES");
            let primary_key = row_string(&row, 4)?.eq_ignore_ascii_case("PRI");
            let extra_upper = extra.to_ascii_uppercase();
            let generated = !generated_expression.is_empty()
                || extra_upper.contains("VIRTUAL GENERATED")
                || extra_upper.contains("STORED GENERATED");
            Ok(ColumnInfo {
                name: row_string(&row, 0)?,
                data_type: data_type.clone(),
                filter_type: base_type.clone(),
                filter_kind: common::filter_kind(&data_type),
                sortable: !base_type.eq_ignore_ascii_case("json")
                    && !base_type.eq_ignore_ascii_case("blob"),
                nullable,
                primary_key,
                generated,
                identity: extra_upper.contains("AUTO_INCREMENT"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let primary_indices = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| index)
        .collect();
    Ok(MysqlMeta {
        columns,
        primary_indices,
    })
}

fn build_filter_predicate(
    columns: &[ColumnInfo],
    filters: &[TableFilter],
    params: &mut Vec<Value>,
) -> Result<Option<String>, String> {
    let mut predicates = Vec::with_capacity(filters.len());
    for filter in filters {
        let column = common::find_column(columns, &filter.column)?;
        common::ensure_filter_operator(column, filter.operator)?;
        let identifier = quote_identifier(&column.name);
        let value = filter.value.as_deref().filter(|value| !value.is_empty());
        let predicate = match filter.operator {
            FilterOperator::Contains => {
                let value = value
                    .ok_or_else(|| format!("Filter on column {} needs a value.", column.name))?;
                params.push(Value::Bytes(value.as_bytes().to_vec()));
                format!("LOCATE(LOWER(?), LOWER(CAST({identifier} AS CHAR))) > 0")
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
                    FilterOperator::Equals => "=",
                    FilterOperator::NotEquals => "<>",
                    FilterOperator::LessThan => "<",
                    FilterOperator::LessThanOrEqual => "<=",
                    FilterOperator::GreaterThan => ">",
                    FilterOperator::GreaterThanOrEqual => ">=",
                    _ => unreachable!(),
                };
                params.push(Value::Bytes(value.as_bytes().to_vec()));
                format!("{identifier} {operator} ?")
            }
            FilterOperator::IsNull => format!("{identifier} IS NULL"),
            FilterOperator::IsNotNull => format!("{identifier} IS NOT NULL"),
            FilterOperator::IsTrue => format!("{identifier} = 1"),
            FilterOperator::IsFalse => format!("{identifier} = 0"),
        };
        predicates.push(predicate);
    }
    Ok((!predicates.is_empty()).then(|| predicates.join(" AND ")))
}

fn build_order_by(
    columns: &[ColumnInfo],
    primary_indices: &[usize],
    sort: Option<&TableSort>,
) -> Result<String, String> {
    let sort_column = sort
        .map(|sort| common::find_column(columns, &sort.column))
        .transpose()?;
    let mut terms = Vec::with_capacity(primary_indices.len() + 1);
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
    for index in primary_indices {
        let column = &columns[*index];
        if sort_column.is_none_or(|sort| sort.name != column.name) {
            terms.push(format!("{} ASC", quote_identifier(&column.name)));
        }
    }
    if terms.is_empty() {
        if let Some(column) = columns.first() {
            terms.push(format!("{} ASC", quote_identifier(&column.name)));
        }
    }
    if terms.is_empty() {
        return Err("Table has no sortable columns.".into());
    }
    Ok(terms.join(", "))
}

fn build_insert(
    schema: &str,
    table: &str,
    meta: &MysqlMeta,
    values: BTreeMap<String, Option<String>>,
) -> Result<(String, Vec<Value>), String> {
    let mut names = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());
    let mut params = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = common::find_column(&meta.columns, &name)?;
        common::ensure_writable(column)?;
        names.push(quote_identifier(&name));
        if is_binary(column) {
            ensure_binary_hex(&value)?;
            placeholders.push("UNHEX(?)".to_string());
        } else {
            placeholders.push("?".to_string());
        }
        params.push(string_value(value));
    }
    if names.is_empty() {
        return Ok((
            format!("INSERT INTO {} () VALUES ()", quote_relation(schema, table)),
            params,
        ));
    }
    Ok((
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_relation(schema, table),
            names.join(", "),
            placeholders.join(", ")
        ),
        params,
    ))
}

fn build_update(
    schema: &str,
    table: &str,
    meta: &MysqlMeta,
    values: BTreeMap<String, Option<String>>,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Value>), String> {
    if values.is_empty() {
        return Err("No changes were staged for this row.".into());
    }
    if original.len() != meta.columns.len() {
        return Err("The table shape changed. Reload before retrying.".into());
    }
    let mut assignments = Vec::with_capacity(values.len());
    let mut params = Vec::with_capacity(values.len() + original.len());
    for (name, value) in values {
        let column = common::find_column(&meta.columns, &name)?;
        common::ensure_writable(column)?;
        assignments.push(format!(
            "{} = {}",
            quote_identifier(&name),
            if is_binary(column) {
                ensure_binary_hex(&value)?;
                "UNHEX(?)"
            } else {
                "?"
            }
        ));
        params.push(string_value(value));
    }
    let predicate = original_predicate(&meta.columns, &original, &mut params, "<=>");
    Ok((
        format!(
            "UPDATE {} SET {} WHERE {predicate}",
            quote_relation(schema, table),
            assignments.join(", ")
        ),
        params,
    ))
}

fn build_delete(
    schema: &str,
    table: &str,
    meta: &MysqlMeta,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Value>), String> {
    if original.len() != meta.columns.len() {
        return Err("The table shape changed. Reload before retrying.".into());
    }
    let mut params = Vec::with_capacity(original.len());
    let predicate = original_predicate(&meta.columns, &original, &mut params, "<=>");
    Ok((
        format!(
            "DELETE FROM {} WHERE {predicate}",
            quote_relation(schema, table)
        ),
        params,
    ))
}

fn original_predicate(
    columns: &[ColumnInfo],
    original: &[Option<String>],
    params: &mut Vec<Value>,
    null_safe_equals: &str,
) -> String {
    columns
        .iter()
        .zip(original)
        .map(|(column, value)| {
            params.push(string_value(value.clone()));
            let identifier = quote_identifier(&column.name);
            let expression = if is_binary(column) {
                format!("HEX({identifier})")
            } else {
                format!("CAST({identifier} AS CHAR)")
            };
            format!("{expression} {null_safe_equals} ?")
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn is_binary(column: &ColumnInfo) -> bool {
    let declared = column.filter_type.to_ascii_lowercase();
    declared.contains("blob") || declared.contains("binary")
}

fn ensure_binary_hex(value: &Option<String>) -> Result<(), String> {
    let Some(hex) = value else {
        return Ok(());
    };
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Binary values must be an even number of hexadecimal digits.".into());
    }
    Ok(())
}

fn string_value(value: Option<String>) -> Value {
    value
        .map(|value| Value::Bytes(value.into_bytes()))
        .unwrap_or(Value::NULL)
}

fn row_string(row: &MysqlRow, index: usize) -> Result<String, String> {
    value_text(
        row.as_ref(index)
            .ok_or_else(|| "Column value was missing from the MySQL result.".to_string())?,
    )
    .ok_or_else(|| "Unexpected NULL in MySQL metadata.".to_string())
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::NULL => None,
        Value::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Value::Int(value) => Some(value.to_string()),
        Value::UInt(value) => Some(value.to_string()),
        Value::Float(value) => Some(value.to_string()),
        Value::Double(value) => Some(value.to_string()),
        Value::Date(year, month, day, hour, minute, second, micros) => {
            if *hour == 0 && *minute == 0 && *second == 0 && *micros == 0 {
                Some(format!("{year:04}-{month:02}-{day:02}"))
            } else if *micros == 0 {
                Some(format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
                ))
            } else {
                Some(format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{micros:06}"
                ))
            }
        }
        Value::Time(negative, days, hours, minutes, seconds, micros) => {
            let sign = if *negative { "-" } else { "" };
            let total_hours = *days * 24 + u32::from(*hours);
            if *micros == 0 {
                Some(format!("{sign}{total_hours:03}:{minutes:02}:{seconds:02}"))
            } else {
                Some(format!(
                    "{sign}{total_hours:03}:{minutes:02}:{seconds:02}.{micros:06}"
                ))
            }
        }
    }
}

fn quote_relation(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
}

fn quote_identifier(identifier: &str) -> String {
    common::quote_mysql_identifier(identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Engine, GridChange, SchemaObjectKind};
    use std::collections::BTreeMap;

    fn test_profile(engine: Engine, port: u16) -> ConnectionProfile {
        ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "mysql test".into(),
            engine,
            host: std::env::var("KOYO_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port,
            database: "koyo".into(),
            username: "root".into(),
            ssl_mode: SslMode::Disable,
            sqlite_path: String::new(),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn mysql_workflow() {
        let engine = std::env::var("KOYO_TEST_MYSQL_ENGINE")
            .ok()
            .and_then(|value| {
                if value == "mariadb" {
                    Some(Engine::Mariadb)
                } else {
                    Some(Engine::Mysql)
                }
            })
            .unwrap_or(Engine::Mysql);
        let port = std::env::var("KOYO_TEST_MYSQL_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(13307);
        let profile = test_profile(engine, port);
        let (mut session, _version, _user, _opts) =
            connect(&profile, "koyo").await.expect("connect");

        let tables = list_tables(&mut session).await.expect("list tables");
        assert!(tables.iter().any(|table| table.name == "users"));

        let objects = list_schema_objects(&mut session)
            .await
            .expect("list schema objects");
        assert!(objects.iter().any(|object| object.schema == "koyo"
            && object.name == "users"
            && object.kind == SchemaObjectKind::Table));
        let keys = list_foreign_keys(&mut session, "koyo", "users")
            .await
            .expect("list foreign keys");
        assert!(
            keys.iter()
                .all(|key| key.schema == "koyo" && key.table == "users")
        );

        let groups = list_database_metadata(&mut session)
            .await
            .expect("list database metadata");
        assert!(groups.iter().any(|group| group.name == "System Info"));
        assert!(groups.iter().any(|group| group.name == "Storage Engines"));
        assert!(!groups.iter().any(|group| group.name == "Schemas"));
        assert!(groups.iter().all(|group| !group.items.is_empty()));

        run_sql(
            &mut session,
            "DELETE FROM users WHERE email LIKE 'new-%@example.com'",
        )
        .await
        .expect("cleanup");
        let page = load_page(&mut session, "koyo", "users", 0, 100, &[], None)
            .await
            .expect("page");
        assert!(page.editable);
        let initial_rows = page.rows.len();
        assert!(initial_rows >= 2);

        let filtered = load_page(
            &mut session,
            "koyo",
            "users",
            0,
            100,
            &[TableFilter {
                column: "email".into(),
                operator: FilterOperator::Equals,
                value: Some("ada@example.com".into()),
            }],
            Some(&TableSort {
                column: "name".into(),
                descending: false,
            }),
        )
        .await
        .expect("filtered page");
        assert_eq!(filtered.rows.len(), 1);
        assert_eq!(
            filtered.rows[0].values[1].as_deref(),
            Some("ada@example.com")
        );

        let numeric = load_page(
            &mut session,
            "koyo",
            "users",
            0,
            100,
            &[TableFilter {
                column: "id".into(),
                operator: FilterOperator::GreaterThanOrEqual,
                value: Some("1".into()),
            }],
            None,
        )
        .await
        .expect("numeric filter");
        assert_eq!(numeric.rows.len(), initial_rows);

        let temporal = load_page(
            &mut session,
            "koyo",
            "users",
            0,
            100,
            &[TableFilter {
                column: "created_at".into(),
                operator: FilterOperator::GreaterThan,
                value: Some("2000-01-01 00:00:00".into()),
            }],
            None,
        )
        .await
        .expect("temporal filter");
        assert_eq!(temporal.rows.len(), initial_rows);

        let inserted_email = format!("new-{}@example.com", uuid::Uuid::new_v4());

        let inserted = apply_changes(
            &mut session,
            "koyo",
            "users",
            vec![GridChange::Insert {
                values: BTreeMap::from([
                    ("email".into(), Some(inserted_email.clone())),
                    ("name".into(), Some("New".into())),
                ]),
            }],
        )
        .await
        .expect("insert");
        assert_eq!(inserted.applied, 1);

        let page = load_page(&mut session, "koyo", "users", 0, 100, &[], None)
            .await
            .expect("page after insert");
        assert_eq!(page.rows.len(), initial_rows + 1);
        let new_row = page
            .rows
            .iter()
            .find(|row| row.values[1].as_deref() == Some(inserted_email.as_str()))
            .cloned()
            .expect("inserted row");

        apply_changes(
            &mut session,
            "koyo",
            "users",
            vec![GridChange::Update {
                key: new_row.key.clone(),
                version: String::new(),
                values: BTreeMap::from([
                    ("name".into(), Some("Renamed".into())),
                    ("created_at".into(), Some("2025-01-02 03:04:05".into())),
                ]),
                original: new_row.values.clone(),
            }],
        )
        .await
        .expect("update");

        let page = load_page(&mut session, "koyo", "users", 0, 100, &[], None)
            .await
            .expect("page after update");
        let new_row = page
            .rows
            .iter()
            .find(|row| row.values[1].as_deref() == Some(inserted_email.as_str()))
            .cloned()
            .expect("updated row");
        assert_eq!(new_row.values[3].as_deref(), Some("2025-01-02 03:04:05"));
        apply_changes(
            &mut session,
            "koyo",
            "users",
            vec![GridChange::Delete {
                key: new_row.key,
                version: String::new(),
                original: new_row.values,
            }],
        )
        .await
        .expect("delete");

        let output = run_sql(&mut session, "SELECT COUNT(*) FROM users")
            .await
            .expect("select");
        assert_eq!(
            output.result_sets[0].rows[0][0].as_deref(),
            Some(initial_rows.to_string().as_str())
        );
        let output = run_sql(&mut session, "UPDATE users SET name = 'Ada2' WHERE id = 1")
            .await
            .expect("update through SQL");
        assert_eq!(output.affected_rows, vec![1]);
        run_sql(&mut session, "UPDATE users SET name = 'Ada' WHERE id = 1")
            .await
            .expect("restore name");

        let binary_table = format!("koyo_bin_{}", uuid::Uuid::new_v4().simple());
        run_sql(
            &mut session,
            &format!(
                "CREATE TABLE {binary_table} (id INT PRIMARY KEY, payload VARBINARY(16));
                 INSERT INTO {binary_table} VALUES (1, x'00aa')"
            ),
        )
        .await
        .expect("create binary fixture");
        let binary_page = load_page(&mut session, "koyo", &binary_table, 0, 100, &[], None)
            .await
            .expect("load binary page");
        assert_eq!(binary_page.rows[0].values[1].as_deref(), Some("00AA"));
        apply_changes(
            &mut session,
            "koyo",
            &binary_table,
            vec![GridChange::Update {
                key: binary_page.rows[0].key.clone(),
                version: String::new(),
                values: BTreeMap::from([("payload".to_string(), Some("DEADBEEF".to_string()))]),
                original: binary_page.rows[0].values.clone(),
            }],
        )
        .await
        .expect("update binary");
        let binary_output = run_sql(
            &mut session,
            &format!("SELECT HEX(payload) FROM {binary_table} WHERE id = 1"),
        )
        .await
        .expect("read binary");
        assert_eq!(
            binary_output.result_sets[0].rows[0][0].as_deref(),
            Some("DEADBEEF")
        );
        run_sql(&mut session, &format!("DROP TABLE {binary_table}"))
            .await
            .expect("drop binary fixture");

        run_sql(&mut session, "START TRANSACTION")
            .await
            .expect("begin");
        assert!(has_open_transaction(&mut session).await.expect("status"));
        let blocked = apply_changes(&mut session, "koyo", "users", Vec::new()).await;
        assert!(
            blocked
                .as_ref()
                .is_err_and(|error| error.contains("before applying grid edits")),
            "grid apply must not commit an open SQL transaction: {blocked:?}"
        );
        run_sql(&mut session, "ROLLBACK").await.expect("rollback");
        assert!(!has_open_transaction(&mut session).await.expect("status"));

        let output = run_sql(&mut session, "SELECT 1 AS a; SELECT 2 AS b")
            .await
            .expect("multiple result sets");
        assert_eq!(output.result_sets.len(), 2);
        assert_eq!(output.result_sets[0].rows[0][0].as_deref(), Some("1"));
        assert_eq!(output.result_sets[1].rows[0][0].as_deref(), Some("2"));
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use crate::models::Engine;
    use std::time::Duration;

    #[tokio::test]
    #[ignore]
    async fn mysql_cancel_running_query() {
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "mysql cancel".into(),
            engine: Engine::Mysql,
            host: std::env::var("KOYO_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("KOYO_TEST_MYSQL_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(13307),
            database: "koyo".into(),
            username: "root".into(),
            ssl_mode: SslMode::Disable,
            sqlite_path: String::new(),
        };
        let (mut session, _version, _user, opts) =
            connect(&profile, "koyo").await.expect("connect");
        let connection_id = session.connection_id;
        let started = std::time::Instant::now();
        let task = tokio::spawn(async move { run_sql(&mut session, "SELECT SLEEP(10)").await });
        tokio::time::sleep(Duration::from_millis(1500)).await;
        cancel_query(&opts, connection_id).await.expect("cancel");
        let _ = task.await.expect("query task");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancelled query should return before the sleep ends"
        );
    }
}
