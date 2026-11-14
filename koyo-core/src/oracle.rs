use crate::common::{self, QUERY_ROW_LIMIT, TABLE_PAGE_LIMIT};
use crate::models::{
    ApplySummary, ColumnCatalogEntry, ColumnInfo, ConnectionProfile, FilterOperator,
    ForeignKeyInfo, GridChange, GridRow, MetadataGroup, MetadataItem, QueryOutput, QueryResultSet,
    SchemaObject, SchemaObjectKind, SslMode, TableFilter, TablePage, TableSort, TableSummary,
};
use oracledb::{Connection, OracleTimestamp, Row};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const SYSTEM_SCHEMAS: &str = "(
    'SYS', 'SYSTEM', 'SYSAUX', 'OUTLN', 'XDB', 'MDSYS', 'ORDSYS',
    'CTXSYS', 'DBSNMP', 'APPQOSSYS', 'AUDSYS', 'GSMADMIN_INTERNAL',
    'OJVMSYS', 'ORDDATA', 'WMSYS', 'LBACSYS', 'DVSYS', 'DVF',
    'GGSYS', 'GSMCATUSER', 'GSMUSER', 'REMOTE_SCHEDULER_AGENT',
    'SYSBACKUP', 'SYSDG', 'SYSKM', 'SYSRAC', 'SYS$UMF', 'XS$NULL'
)";

#[derive(Clone)]
pub struct OracleSession {
    connection: Arc<Mutex<Connection>>,
}

pub async fn connect(
    profile: &ConnectionProfile,
    password: &str,
) -> Result<(OracleSession, String, String), String> {
    let profile = profile.clone();
    let password = password.to_string();
    blocking(move || {
        let connect_string = if profile.ssl_mode == SslMode::VerifyFull {
            format!(
                "tcps://{}:{}/{}",
                profile.host.trim(),
                profile.port,
                profile.database.trim()
            )
        } else {
            format!(
                "{}:{}/{}",
                profile.host.trim(),
                profile.port,
                profile.database.trim()
            )
        };
        let config = oracledb::Config::default()
            .set_credentials(profile.username.trim(), &password)
            .set_connect_string(&connect_string)
            .map_err(|error| format!("Oracle connect string is invalid: {error}"))?
            .set_driver_name("Koyo");
        let connection = oracledb::connect(config)
            .map_err(|error| format!("Oracle connection failed: {error}"))?;
        let row = connection
            .query_row(
                "SELECT version, USER
                 FROM product_component_version
                 WHERE ROWNUM = 1",
                &[],
            )
            .map_err(|error| format!("Could not read Oracle server details: {error}"))?;
        let server_version = row.get::<String>(0).map_err(|error| error.to_string())?;
        let current_user = row.get::<String>(1).map_err(|error| error.to_string())?;
        Ok((
            OracleSession {
                connection: Arc::new(Mutex::new(connection)),
            },
            server_version,
            current_user,
        ))
    })
    .await
}

pub async fn list_tables(session: &OracleSession) -> Result<Vec<TableSummary>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let sql = format!(
            "SELECT t.owner, t.table_name,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM all_constraints c
                        WHERE c.owner = t.owner
                          AND c.table_name = t.table_name
                          AND c.constraint_type = 'P'
                    ) THEN 1 ELSE 0 END
             FROM all_tables t
             WHERE t.dropped = 'NO'
               AND t.owner NOT IN {SYSTEM_SCHEMAS}
             ORDER BY t.owner, t.table_name"
        );
        let rows = connection
            .query(&sql, &[])
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let row = row.map_err(|error| error.to_string())?;
            Ok(TableSummary {
                schema: row.get::<String>(0).map_err(|error| error.to_string())?,
                name: row.get::<String>(1).map_err(|error| error.to_string())?,
                editable: row.get::<i32>(2).map_err(|error| error.to_string())? != 0,
            })
        })
        .collect()
    })
    .await
}

pub async fn list_columns(session: &OracleSession) -> Result<Vec<ColumnCatalogEntry>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let sql = format!(
            "SELECT owner, table_name, column_name
             FROM all_tab_columns
             WHERE owner NOT IN {SYSTEM_SCHEMAS}
             ORDER BY owner, table_name, column_id"
        );
        let rows = connection
            .query(&sql, &[])
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let row = row.map_err(|error| error.to_string())?;
            Ok(ColumnCatalogEntry {
                schema: row.get::<String>(0).map_err(|error| error.to_string())?,
                table: row.get::<String>(1).map_err(|error| error.to_string())?,
                column: row.get::<String>(2).map_err(|error| error.to_string())?,
            })
        })
        .collect()
    })
    .await
}

pub async fn list_schema_objects(session: &OracleSession) -> Result<Vec<SchemaObject>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let sql = format!(
            "SELECT t.owner, t.table_name, 'table'
             FROM all_tables t
             WHERE t.dropped = 'NO'
               AND t.owner NOT IN {SYSTEM_SCHEMAS}
             UNION ALL
             SELECT v.owner, v.view_name, 'view'
             FROM all_views v
             WHERE v.owner NOT IN {SYSTEM_SCHEMAS}
             UNION ALL
             SELECT o.owner, o.object_name, 'function'
             FROM all_objects o
             WHERE o.object_type = 'FUNCTION'
               AND o.owner NOT IN {SYSTEM_SCHEMAS}
             UNION ALL
             SELECT g.owner, g.trigger_name, 'trigger'
             FROM all_triggers g
             WHERE g.owner NOT IN {SYSTEM_SCHEMAS}
             ORDER BY 1, 2, 3"
        );
        let rows = connection
            .query(&sql, &[])
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let row = row.map_err(|error| error.to_string())?;
            let kind = row.get::<String>(2).map_err(|error| error.to_string())?;
            Ok(SchemaObject {
                schema: row.get::<String>(0).map_err(|error| error.to_string())?,
                name: row.get::<String>(1).map_err(|error| error.to_string())?,
                kind: match kind.as_str() {
                    "table" => SchemaObjectKind::Table,
                    "view" => SchemaObjectKind::View,
                    "function" => SchemaObjectKind::Function,
                    "trigger" => SchemaObjectKind::Trigger,
                    other => return Err(format!("Unexpected Oracle object type: {other}")),
                },
            })
        })
        .collect()
    })
    .await
}

pub async fn list_foreign_keys(
    session: &OracleSession,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let connection = session.connection.clone();
    let schema = schema.to_string();
    let table = table.to_string();
    blocking(move || {
        let connection = lock(&connection)?;
        let sql = "SELECT c.owner, c.table_name, cc.column_name,
                          r.owner, r.table_name, rcc.column_name,
                          c.constraint_name
                   FROM all_constraints c
                   JOIN all_constraints r
                     ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name
                   JOIN all_cons_columns cc
                     ON cc.owner = c.owner AND cc.constraint_name = c.constraint_name
                   JOIN all_cons_columns rcc
                     ON rcc.owner = r.owner
                    AND rcc.constraint_name = r.constraint_name
                    AND rcc.position = cc.position
                   WHERE c.constraint_type = 'R'
                     AND ((c.owner = :1 AND c.table_name = :2)
                          OR (r.owner = :3 AND r.table_name = :4))
                   ORDER BY c.owner, c.table_name, c.constraint_name, cc.position";
        let params: Vec<Option<String>> = vec![
            Some(schema.clone()),
            Some(table.clone()),
            Some(schema),
            Some(table),
        ];
        let params = params
            .iter()
            .map(|value| value as &dyn oracledb::ToDbValue)
            .collect::<Vec<_>>();
        let cursor = connection
            .query(sql, params.as_slice())
            .map_err(|error| error.to_string())?;
        let mut keys = Vec::new();
        for row in cursor {
            let row = row.map_err(|error| error.to_string())?;
            keys.push(ForeignKeyInfo {
                schema: row.get::<String>(0).map_err(|error| error.to_string())?,
                table: row.get::<String>(1).map_err(|error| error.to_string())?,
                column: row.get::<String>(2).map_err(|error| error.to_string())?,
                referenced_schema: row.get::<String>(3).map_err(|error| error.to_string())?,
                referenced_table: row.get::<String>(4).map_err(|error| error.to_string())?,
                referenced_column: row.get::<String>(5).map_err(|error| error.to_string())?,
                constraint_name: row.get::<String>(6).map_err(|error| error.to_string())?,
            });
        }
        Ok(keys)
    })
    .await
}

pub async fn list_database_metadata(session: &OracleSession) -> Result<Vec<MetadataGroup>, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        let mut groups = Vec::new();

        let rows = connection
            .query(
                "SELECT username, TO_CHAR(created, 'YYYY-MM-DD')
                 FROM all_users
                 ORDER BY username
                 FETCH FIRST 200 ROWS ONLY",
                &[],
            )
            .map_err(|error| error.to_string())?;
        let mut items = Vec::new();
        for row in rows {
            let row = row.map_err(|error| error.to_string())?;
            items.push(MetadataItem {
                name: row.get::<String>(0).map_err(|error| error.to_string())?,
                detail: row.get::<String>(1).map_err(|error| error.to_string())?,
            });
        }
        if !items.is_empty() {
            groups.push(MetadataGroup {
                name: "Users".into(),
                items,
            });
        }

        let rows = connection
            .query(
                "SELECT tablespace_name, status, contents
                 FROM user_tablespaces
                 ORDER BY tablespace_name",
                &[],
            )
            .map_err(|error| error.to_string())?;
        let mut items = Vec::new();
        for row in rows {
            let row = row.map_err(|error| error.to_string())?;
            let status = row.get::<String>(1).map_err(|error| error.to_string())?;
            let contents = row.get::<String>(2).map_err(|error| error.to_string())?;
            items.push(MetadataItem {
                name: row.get::<String>(0).map_err(|error| error.to_string())?,
                detail: format!("{status}, {contents}"),
            });
        }
        if !items.is_empty() {
            groups.push(MetadataGroup {
                name: "Storage".into(),
                items,
            });
        }

        let row = connection
            .query_row(
                "SELECT (SELECT version FROM product_component_version WHERE ROWNUM = 1),
                        SYS_CONTEXT('USERENV', 'DB_NAME'),
                        SYS_CONTEXT('USERENV', 'CON_NAME'),
                        USER
                 FROM dual",
                &[],
            )
            .map_err(|error| error.to_string())?;
        groups.push(MetadataGroup {
            name: "System Info".into(),
            items: vec![
                MetadataItem {
                    name: "Version".into(),
                    detail: row.get::<String>(0).map_err(|error| error.to_string())?,
                },
                MetadataItem {
                    name: "Database".into(),
                    detail: row.get::<String>(1).map_err(|error| error.to_string())?,
                },
                MetadataItem {
                    name: "Container".into(),
                    detail: row.get::<String>(2).map_err(|error| error.to_string())?,
                },
                MetadataItem {
                    name: "Current User".into(),
                    detail: row.get::<String>(3).map_err(|error| error.to_string())?,
                },
            ],
        });

        Ok(groups)
    })
    .await
}

pub async fn load_page(
    session: &OracleSession,
    schema: &str,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    let connection = session.connection.clone();
    let schema = schema.to_string();
    let table = table.to_string();
    let filters = filters.to_vec();
    let sort = sort.cloned();
    blocking(move || {
        let connection = lock(&connection)?;
        load_page_sync(
            &connection,
            &schema,
            &table,
            offset,
            limit,
            &filters,
            sort.as_ref(),
        )
    })
    .await
}

pub async fn apply_changes(
    session: &OracleSession,
    schema: &str,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    let connection = session.connection.clone();
    let schema = schema.to_string();
    let table = table.to_string();
    blocking(move || {
        let connection = lock(&connection)?;
        apply_changes_sync(&connection, &schema, &table, changes)
    })
    .await
}

pub async fn run_sql(session: &OracleSession, sql: &str) -> Result<QueryOutput, String> {
    let connection = session.connection.clone();
    let sql = sql.to_string();
    blocking(move || {
        let connection = lock(&connection)?;
        run_sql_sync(&connection, &sql)
    })
    .await
}

#[cfg(test)]
pub async fn has_open_transaction(session: &OracleSession) -> Result<bool, String> {
    let connection = session.connection.clone();
    blocking(move || {
        let connection = lock(&connection)?;
        has_open_transaction_sync(&connection)
    })
    .await
}

fn has_open_transaction_sync(connection: &Connection) -> Result<bool, String> {
    let row = connection
        .query_row(
            "SELECT RAWTOHEX(DBMS_TRANSACTION.LOCAL_TRANSACTION_ID) FROM dual",
            &[],
        )
        .map_err(|error| format!("Could not read Oracle transaction state: {error}"))?;
    let transaction_id = row
        .get::<Option<String>>(0)
        .map_err(|error| error.to_string())?;
    Ok(transaction_id.is_some_and(|value| !value.is_empty()))
}

pub fn cancel_query() -> Result<(), String> {
    Err("Cancellation is not supported by the Oracle beta driver yet.".into())
}

fn load_page_sync(
    connection: &Connection,
    schema: &str,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    let meta = load_meta(connection, schema, table)?;
    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let selected = meta
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::new();
    let predicate = build_filter_predicate(&meta.columns, filters, &mut params)?;
    let order = build_order_by(&meta.columns, &meta.primary_indices, sort)?;
    let mut sql = format!("SELECT {selected} FROM {}", quote_relation(schema, table));
    if let Some(predicate) = predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate);
    }
    sql.push_str(&format!(
        " ORDER BY {order} OFFSET {offset} ROWS FETCH NEXT {} ROWS ONLY",
        limit + 1
    ));
    let params = params
        .iter()
        .map(|value| value as &dyn oracledb::ToDbValue)
        .collect::<Vec<_>>();
    let cursor = connection
        .query(&sql, params.as_slice())
        .map_err(|error| error.to_string())?;
    let mut page_rows = Vec::with_capacity(limit as usize + 1);
    for row in cursor {
        let row = row.map_err(|error| error.to_string())?;
        let values = (0..meta.columns.len())
            .map(|index| cell_text(&row, index))
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
    Ok(TablePage {
        editable: meta.editable(),
        columns: meta.columns,
        rows: page_rows,
        offset,
        limit,
        has_more,
    })
}

fn apply_changes_sync(
    connection: &Connection,
    schema: &str,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if has_open_transaction_sync(connection)? {
        return Err(
            "Commit or roll back the SQL tab's transaction before applying grid edits.".into(),
        );
    }
    let meta = load_meta(connection, schema, table)?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }
    let mut applied = 0;
    for change in changes {
        let result = match change {
            GridChange::Insert { values } => build_insert(connection, schema, table, &meta, values),
            GridChange::Update {
                values, original, ..
            } => build_update(connection, schema, table, &meta, values, original),
            GridChange::Delete { original, .. } => {
                build_delete(connection, schema, table, &meta, original)
            }
        };
        match result {
            Ok(count) => {
                if let Err(error) = common::ensure_one_row(count) {
                    let _ = connection.rollback();
                    return Err(error);
                }
                applied += 1;
            }
            Err(error) => {
                let _ = connection.rollback();
                return Err(error);
            }
        }
    }
    connection
        .commit()
        .map_err(|error| format!("COMMIT_OUTCOME_UNKNOWN: {error}"))?;
    Ok(ApplySummary { applied })
}

fn run_sql_sync(connection: &Connection, sql: &str) -> Result<QueryOutput, String> {
    let keyword = first_keyword(sql);
    let is_query = matches!(keyword.as_str(), "select" | "with" | "explain");
    if !is_query {
        let result = connection
            .execute(sql, &[])
            .map_err(|error| error.to_string())?;
        return Ok(QueryOutput {
            result_sets: Vec::new(),
            affected_rows: if result.rows_affected() > 0 {
                vec![result.rows_affected()]
            } else {
                Vec::new()
            },
            truncated: false,
        });
    }
    let cursor = connection
        .query(sql, &[])
        .map_err(|error| error.to_string())?;
    let columns = cursor
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut truncated = false;
    for row in cursor {
        let row = row.map_err(|error| error.to_string())?;
        if rows.len() >= QUERY_ROW_LIMIT {
            truncated = true;
            continue;
        }
        rows.push(
            (0..columns.len())
                .map(|index| cell_text(&row, index))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(QueryOutput {
        result_sets: vec![QueryResultSet { columns, rows }],
        affected_rows: Vec::new(),
        truncated,
    })
}

struct OracleMeta {
    columns: Vec<ColumnInfo>,
    primary_indices: Vec<usize>,
}

impl OracleMeta {
    fn editable(&self) -> bool {
        !self.primary_indices.is_empty()
    }
}

fn load_meta(connection: &Connection, schema: &str, table: &str) -> Result<OracleMeta, String> {
    let params: [&dyn oracledb::ToDbValue; 2] = [&schema, &table];
    let rows = connection
        .query(
            "SELECT c.column_name, c.data_type, c.data_length, c.data_precision,
                    c.data_scale, c.nullable,
                    CASE WHEN pk.column_name IS NULL THEN 0 ELSE 1 END,
                    c.virtual_column, c.identity_column
             FROM all_tab_cols c
             LEFT JOIN (
                 SELECT cc.owner, cc.table_name, cc.column_name
                 FROM all_constraints cons
                 JOIN all_cons_columns cc
                   ON cc.owner = cons.owner
                  AND cc.constraint_name = cons.constraint_name
                 WHERE cons.constraint_type = 'P'
             ) pk
               ON pk.owner = c.owner
              AND pk.table_name = c.table_name
              AND pk.column_name = c.column_name
             WHERE c.owner = :1 AND c.table_name = :2
               AND c.hidden_column = 'NO'
             ORDER BY c.column_id",
            &params,
        )
        .map_err(|error| error.to_string())?;
    let mut columns = Vec::new();
    for row in rows {
        let row = row.map_err(|error| error.to_string())?;
        let base_type = row.get::<String>(1).map_err(|error| error.to_string())?;
        let data_length = row.get::<u32>(2).map_err(|error| error.to_string())?;
        let precision = row
            .get::<Option<u32>>(3)
            .map_err(|error| error.to_string())?;
        let scale = row
            .get::<Option<i32>>(4)
            .map_err(|error| error.to_string())?;
        let data_type = display_type(&base_type, data_length, precision, scale);
        columns.push(ColumnInfo {
            name: row.get::<String>(0).map_err(|error| error.to_string())?,
            data_type,
            filter_type: base_type.clone(),
            filter_kind: common::filter_kind(&base_type),
            sortable: !matches!(
                base_type.as_str(),
                "BLOB" | "CLOB" | "NCLOB" | "JSON" | "XMLTYPE" | "OBJECT" | "BFILE"
            ),
            nullable: row
                .get::<String>(5)
                .map_err(|error| error.to_string())?
                .eq_ignore_ascii_case("Y"),
            primary_key: row.get::<i32>(6).map_err(|error| error.to_string())? != 0,
            generated: row
                .get::<String>(7)
                .map_err(|error| error.to_string())?
                .eq_ignore_ascii_case("YES"),
            identity: row
                .get::<String>(8)
                .map_err(|error| error.to_string())?
                .eq_ignore_ascii_case("YES"),
        });
    }
    if columns.is_empty() {
        return Err("Table has no visible columns.".into());
    }
    let primary_indices = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| index)
        .collect();
    Ok(OracleMeta {
        columns,
        primary_indices,
    })
}

fn first_keyword(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut index = 0;
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index + 1 < bytes.len() && bytes[index] == b'-' && bytes[index + 1] == b'-' {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        break;
    }
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
        index += 1;
    }
    sql[start..index].to_ascii_lowercase()
}

fn display_type(
    base_type: &str,
    data_length: u32,
    precision: Option<u32>,
    scale: Option<i32>,
) -> String {
    match base_type {
        "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" | "RAW" => {
            format!("{base_type}({data_length})")
        }
        "NUMBER" => match (precision, scale) {
            (Some(precision), Some(scale)) => format!("NUMBER({precision},{scale})"),
            _ => "NUMBER".into(),
        },
        "TIMESTAMP" => match scale {
            Some(scale) => format!("TIMESTAMP({scale})"),
            None => "TIMESTAMP".into(),
        },
        _ => base_type.to_string(),
    }
}

fn build_filter_predicate(
    columns: &[ColumnInfo],
    filters: &[TableFilter],
    params: &mut Vec<Option<String>>,
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
                params.push(Some(value.to_string()));
                format!(
                    "INSTR(LOWER(TO_CHAR({identifier})), LOWER(:{})) > 0",
                    params.len()
                )
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
                params.push(Some(value.to_string()));
                let placeholder = format!(":{}", params.len());
                format!(
                    "{identifier} {operator} {}",
                    bind_expression(column, &placeholder)
                )
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
    connection: &Connection,
    schema: &str,
    table: &str,
    meta: &OracleMeta,
    values: BTreeMap<String, Option<String>>,
) -> Result<u64, String> {
    let mut names = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());
    let mut params = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = common::find_column(&meta.columns, &name)?;
        common::ensure_writable(column)?;
        names.push(quote_identifier(&name));
        let placeholder = format!(":{}", params.len() + 1);
        placeholders.push(bind_expression(column, &placeholder));
        params.push(value);
    }
    if names.is_empty() {
        let column = meta
            .columns
            .iter()
            .find(|column| !column.identity && !column.generated)
            .ok_or_else(|| "This table has no writable column for an empty insert.".to_string())?;
        let sql = format!(
            "INSERT INTO {} ({}) VALUES (DEFAULT)",
            quote_relation(schema, table),
            quote_identifier(&column.name)
        );
        return connection
            .execute(&sql, &[])
            .map(|result| result.rows_affected())
            .map_err(|error| error.to_string());
    }
    let params = params
        .iter()
        .map(|value| value as &dyn oracledb::ToDbValue)
        .collect::<Vec<_>>();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_relation(schema, table),
        names.join(", "),
        placeholders.join(", ")
    );
    connection
        .execute(&sql, params.as_slice())
        .map(|result| result.rows_affected())
        .map_err(|error| error.to_string())
}

fn build_update(
    connection: &Connection,
    schema: &str,
    table: &str,
    meta: &OracleMeta,
    values: BTreeMap<String, Option<String>>,
    original: Vec<Option<String>>,
) -> Result<u64, String> {
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
        let placeholder = format!(":{}", params.len() + 1);
        assignments.push(format!(
            "{} = {}",
            quote_identifier(&name),
            bind_expression(column, &placeholder)
        ));
        params.push(value);
    }
    let predicate = original_predicate(&meta.columns, &original, &mut params);
    let params = params
        .iter()
        .map(|value| value as &dyn oracledb::ToDbValue)
        .collect::<Vec<_>>();
    let sql = format!(
        "UPDATE {} SET {} WHERE {predicate}",
        quote_relation(schema, table),
        assignments.join(", ")
    );
    connection
        .execute(&sql, params.as_slice())
        .map(|result| result.rows_affected())
        .map_err(|error| error.to_string())
}

fn build_delete(
    connection: &Connection,
    schema: &str,
    table: &str,
    meta: &OracleMeta,
    original: Vec<Option<String>>,
) -> Result<u64, String> {
    if original.len() != meta.columns.len() {
        return Err("The table shape changed. Reload before retrying.".into());
    }
    let mut params = Vec::with_capacity(original.len());
    let predicate = original_predicate(&meta.columns, &original, &mut params);
    let params = params
        .iter()
        .map(|value| value as &dyn oracledb::ToDbValue)
        .collect::<Vec<_>>();
    let sql = format!(
        "DELETE FROM {} WHERE {predicate}",
        quote_relation(schema, table)
    );
    connection
        .execute(&sql, params.as_slice())
        .map(|result| result.rows_affected())
        .map_err(|error| error.to_string())
}

fn bind_expression(column: &ColumnInfo, placeholder: &str) -> String {
    let base_type = column.filter_type.to_ascii_uppercase();
    if base_type == "DATE" {
        format!("CAST(TO_TIMESTAMP({placeholder}, 'YYYY-MM-DD\"T\"HH24:MI:SS.FF9') AS DATE)")
    } else if base_type.contains("TIMESTAMP") && base_type.contains("WITH LOCAL TIME ZONE") {
        format!("TO_TIMESTAMP_TZ({placeholder}, 'YYYY-MM-DD\"T\"HH24:MI:SS.FF9')")
    } else if base_type.contains("TIMESTAMP") && base_type.contains("WITH TIME ZONE") {
        format!("TO_TIMESTAMP_TZ({placeholder}, 'YYYY-MM-DD\"T\"HH24:MI:SS.FF9TZH:TZM')")
    } else if base_type.starts_with("TIMESTAMP") {
        format!("TO_TIMESTAMP({placeholder}, 'YYYY-MM-DD\"T\"HH24:MI:SS.FF9')")
    } else if matches!(
        base_type.as_str(),
        "NUMBER" | "INTEGER" | "FLOAT" | "BINARY_INTEGER"
    ) {
        format!("TO_NUMBER({placeholder})")
    } else if base_type == "BINARY_FLOAT" {
        format!("TO_BINARY_FLOAT({placeholder})")
    } else if base_type == "BINARY_DOUBLE" {
        format!("TO_BINARY_DOUBLE({placeholder})")
    } else if base_type == "RAW" || base_type == "LONG RAW" {
        format!("HEXTORAW({placeholder})")
    } else {
        placeholder.to_string()
    }
}

fn original_predicate(
    columns: &[ColumnInfo],
    original: &[Option<String>],
    params: &mut Vec<Option<String>>,
) -> String {
    columns
        .iter()
        .zip(original)
        .filter(|(column, _)| is_comparable(column))
        .map(|(column, value)| {
            params.push(value.clone());
            let placeholder = format!(":{}", params.len());
            let identifier = quote_identifier(&column.name);
            let bind = bind_expression(column, &placeholder);
            format!("DECODE({identifier}, {bind}, 1, 0) = 1")
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn is_comparable(column: &ColumnInfo) -> bool {
    !matches!(
        column.filter_type.as_str(),
        "CLOB" | "NCLOB" | "BLOB" | "BFILE" | "JSON" | "XMLTYPE" | "OBJECT"
    )
}

fn cell_text(row: &Row, index: usize) -> Result<Option<String>, String> {
    let column = row
        .columns()
        .get(index)
        .ok_or_else(|| "Oracle returned an unexpected column index.".to_string())?;
    let type_name = column.db_type().name();
    match type_name {
        "DB_TYPE_VARCHAR"
        | "DB_TYPE_NVARCHAR"
        | "DB_TYPE_CHAR"
        | "DB_TYPE_NCHAR"
        | "DB_TYPE_LONG"
        | "DB_TYPE_LONG_NVARCHAR"
        | "DB_TYPE_NUMBER"
        | "DB_TYPE_BINARY_INTEGER"
        | "DB_TYPE_ROWID"
        | "DB_TYPE_UROWID" => row
            .get::<Option<String>>(index)
            .map_err(|error| error.to_string()),
        "DB_TYPE_BINARY_FLOAT" => row
            .get::<Option<f32>>(index)
            .map(|value| value.map(|value| value.to_string()))
            .map_err(|error| error.to_string()),
        "DB_TYPE_BINARY_DOUBLE" => row
            .get::<Option<f64>>(index)
            .map(|value| value.map(|value| value.to_string()))
            .map_err(|error| error.to_string()),
        "DB_TYPE_DATE" | "DB_TYPE_TIMESTAMP" | "DB_TYPE_TIMESTAMP_TZ" | "DB_TYPE_TIMESTAMP_LTZ" => {
            row.get::<Option<OracleTimestamp>>(index)
                .map(|value| value.map(|value| timestamp_text(&value)))
                .map_err(|error| error.to_string())
        }
        "DB_TYPE_BOOLEAN" => row
            .get::<Option<bool>>(index)
            .map(|value| value.map(|value| value.to_string()))
            .map_err(|error| error.to_string()),
        "DB_TYPE_RAW" | "DB_TYPE_LONG_RAW" => row
            .get::<Option<Vec<u8>>>(index)
            .map(|value| value.map(|value| hex(&value)))
            .map_err(|error| error.to_string()),
        "DB_TYPE_CLOB" | "DB_TYPE_NCLOB" | "DB_TYPE_BLOB" | "DB_TYPE_BFILE" | "DB_TYPE_JSON"
        | "DB_TYPE_XMLTYPE" | "DB_TYPE_OBJECT" => Ok(Some(format!("[{type_name}]"))),
        _ => row
            .get::<Option<String>>(index)
            .map_err(|error| format!("{type_name}: {error}")),
    }
}

fn timestamp_text(value: &OracleTimestamp) -> String {
    let mut text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}",
        value.year(),
        value.month(),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.nanoseconds()
    );
    if value.is_utc() {
        text.push('Z');
    }
    text
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn quote_relation(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
}

fn quote_identifier(identifier: &str) -> String {
    common::quote_identifier(identifier)
}

fn lock(
    connection: &Arc<Mutex<Connection>>,
) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
    connection
        .lock()
        .map_err(|_| "Oracle connection lock was poisoned.".to_string())
}

async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("Oracle worker failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{GridChange, SchemaObjectKind};
    use std::collections::BTreeMap;

    #[tokio::test]
    #[ignore]
    async fn oracle_workflow() {
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "oracle test".into(),
            engine: crate::models::Engine::Oracle,
            host: std::env::var("KOYO_TEST_ORACLE_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("KOYO_TEST_ORACLE_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1521),
            database: std::env::var("KOYO_TEST_ORACLE_SERVICE")
                .unwrap_or_else(|_| "FREEPDB1".into()),
            username: std::env::var("KOYO_TEST_ORACLE_USER").unwrap_or_else(|_| "system".into()),
            ssl_mode: SslMode::Disable,
            sqlite_path: String::new(),
        };
        let password = std::env::var("KOYO_TEST_ORACLE_PASSWORD").unwrap_or_else(|_| "koyo".into());
        let (session, _version, _user) = connect(&profile, &password).await.expect("connect");
        let output = run_sql(&session, "SELECT 1 AS n FROM dual")
            .await
            .expect("select");
        assert_eq!(output.result_sets[0].rows[0][0].as_deref(), Some("1"));
        let output = run_sql(&session, "-- leading comment\nSELECT 7 AS n FROM dual")
            .await
            .expect("commented select");
        assert_eq!(output.result_sets[0].rows[0][0].as_deref(), Some("7"));
        let tables = list_tables(&session).await.expect("list tables");
        assert!(
            tables
                .iter()
                .any(|table| table.schema == "KOYO" && table.name == "USERS")
        );

        let objects = list_schema_objects(&session)
            .await
            .expect("list schema objects");
        assert!(objects.iter().any(|object| object.schema == "KOYO"
            && object.name == "USERS"
            && object.kind == SchemaObjectKind::Table));
        let keys = list_foreign_keys(&session, "KOYO", "USERS")
            .await
            .expect("list foreign keys");
        assert!(
            keys.iter()
                .all(|key| key.schema == "KOYO" && key.table == "USERS")
        );

        let groups = list_database_metadata(&session)
            .await
            .expect("list database metadata");
        assert!(groups.iter().any(|group| group.name == "System Info"));
        assert!(groups.iter().any(|group| group.name == "Users"));
        assert!(groups.iter().any(|group| group.name == "Storage"));
        assert!(!groups.iter().any(|group| group.name == "Schemas"));
        assert!(groups.iter().all(|group| !group.items.is_empty()));

        run_sql(
            &session,
            "DELETE FROM koyo.users WHERE email LIKE 'new-%@example.com'",
        )
        .await
        .expect("cleanup");
        run_sql(&session, "COMMIT").await.expect("commit cleanup");
        let page = load_page(&session, "KOYO", "USERS", 0, 100, &[], None)
            .await
            .expect("page");
        assert!(page.editable);
        let initial_rows = page.rows.len();
        assert!(initial_rows >= 2);
        let filtered = load_page(
            &session,
            "KOYO",
            "USERS",
            0,
            100,
            &[TableFilter {
                column: "EMAIL".into(),
                operator: FilterOperator::Equals,
                value: Some("ada@example.com".into()),
            }],
            Some(&TableSort {
                column: "NAME".into(),
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
            &session,
            "KOYO",
            "USERS",
            0,
            100,
            &[TableFilter {
                column: "ID".into(),
                operator: FilterOperator::GreaterThanOrEqual,
                value: Some("1".into()),
            }],
            None,
        )
        .await
        .expect("numeric filter");
        assert_eq!(numeric.rows.len(), initial_rows);

        let temporal = load_page(
            &session,
            "KOYO",
            "USERS",
            0,
            100,
            &[TableFilter {
                column: "CREATED_AT".into(),
                operator: FilterOperator::GreaterThan,
                value: Some("2000-01-01T00:00:00.000000000".into()),
            }],
            None,
        )
        .await
        .expect("temporal filter");
        assert_eq!(temporal.rows.len(), initial_rows);

        let inserted_email = format!("new-{}@example.com", uuid::Uuid::new_v4());
        apply_changes(
            &session,
            "KOYO",
            "USERS",
            vec![GridChange::Insert {
                values: BTreeMap::from([
                    ("EMAIL".into(), Some(inserted_email.clone())),
                    ("NAME".into(), Some("New".into())),
                ]),
            }],
        )
        .await
        .expect("insert");

        let page = load_page(&session, "KOYO", "USERS", 0, 100, &[], None)
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
            &session,
            "KOYO",
            "USERS",
            vec![GridChange::Update {
                key: new_row.key.clone(),
                version: String::new(),
                values: BTreeMap::from([
                    ("NAME".into(), Some("Renamed".into())),
                    (
                        "CREATED_AT".into(),
                        Some("2025-01-02T03:04:05.000000000".into()),
                    ),
                ]),
                original: new_row.values.clone(),
            }],
        )
        .await
        .expect("update");

        let page = load_page(&session, "KOYO", "USERS", 0, 100, &[], None)
            .await
            .expect("page after update");
        let new_row = page
            .rows
            .iter()
            .find(|row| row.values[1].as_deref() == Some(inserted_email.as_str()))
            .cloned()
            .expect("updated row");
        assert_eq!(
            new_row.values[3].as_deref(),
            Some("2025-01-02T03:04:05.000000000")
        );
        apply_changes(
            &session,
            "KOYO",
            "USERS",
            vec![GridChange::Delete {
                key: new_row.key,
                version: String::new(),
                original: new_row.values,
            }],
        )
        .await
        .expect("delete");

        let output = run_sql(&session, "SELECT COUNT(*) FROM koyo.users")
            .await
            .expect("select count");
        assert_eq!(
            output.result_sets[0].rows[0][0].as_deref(),
            Some(initial_rows.to_string().as_str())
        );

        let output = run_sql(&session, "UPDATE koyo.users SET name = 'Ada2' WHERE id = 1")
            .await
            .expect("update through SQL");
        assert_eq!(output.affected_rows, vec![1]);
        assert!(has_open_transaction(&session).await.expect("status"));
        let blocked = apply_changes(&session, "KOYO", "USERS", Vec::new()).await;
        assert!(
            blocked
                .as_ref()
                .is_err_and(|error| error.contains("before applying grid edits")),
            "grid apply must not commit an open SQL transaction: {blocked:?}"
        );
        run_sql(&session, "ROLLBACK").await.expect("rollback");
        assert!(!has_open_transaction(&session).await.expect("status"));
    }
}
