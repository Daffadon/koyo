use crate::common::{self, QUERY_ROW_LIMIT, TABLE_PAGE_LIMIT};
use crate::models::{
    ApplySummary, ColumnCatalogEntry, ColumnInfo, ConnectionProfile, Engine, FilterOperator,
    ForeignKeyInfo, GridChange, GridRow, MetadataGroup, MetadataItem, QueryOutput, QueryResultSet,
    SchemaObject, SchemaObjectKind, SslMode, TableFilter, TablePage, TableSort, TableSummary,
};
use chrono::{NaiveDate, TimeDelta};
use std::collections::BTreeMap;
use tiberius::time::{Date, DateTime, DateTime2, DateTimeOffset, SmallDateTime, Time};
use tiberius::{Client, ColumnData, Config, EncryptionLevel, Query, QueryItem};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type MssqlClient = Client<Compat<TcpStream>>;

pub struct MssqlSession {
    pub client: MssqlClient,
    pub spid: i16,
}

pub async fn connect(
    profile: &ConnectionProfile,
    password: &str,
    engine: Engine,
) -> Result<(MssqlSession, String, String, Config), String> {
    let mut config = Config::default();
    config.host(profile.host.trim());
    config.port(profile.port);
    config.database(profile.database.trim());
    config.authentication(tiberius::AuthMethod::sql_server(
        profile.username.trim(),
        password,
    ));
    config.application_name(if engine == Engine::SqlServer {
        "Koyo SQL Server"
    } else {
        "Koyo"
    });
    config.encryption(if profile.ssl_mode == SslMode::VerifyFull {
        EncryptionLevel::Required
    } else {
        EncryptionLevel::NotSupported
    });
    // Koyo cancels long statements explicitly; the driver's 30s deadline
    // leaves the connection desynchronized after expiry.
    config.command_timeout(None);
    let mut client = open_client(config.clone()).await?;
    let row = client
        .query(
            "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)),
                    SUSER_SNAME(), @@SPID",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_row()
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "SQL Server returned no connection details.".to_string())?;
    let server_version = row.get::<&str, _>(0).unwrap_or("unknown").to_string();
    let current_user = row
        .get::<&str, _>(1)
        .unwrap_or(profile.username.as_str())
        .to_string();
    let spid = row
        .get::<i16, _>(2)
        .ok_or_else(|| "Could not read SQL Server session id.".to_string())?;
    Ok((
        MssqlSession { client, spid },
        server_version,
        current_user,
        config,
    ))
}

pub async fn list_tables(session: &mut MssqlSession) -> Result<Vec<TableSummary>, String> {
    let rows = session
        .client
        .query(
            "SELECT s.name, t.name,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM sys.indexes i
                        WHERE i.object_id = t.object_id AND i.is_primary_key = 1
                    ) THEN 1 ELSE 0 END
             FROM sys.tables t
             JOIN sys.schemas s ON s.schema_id = t.schema_id
             WHERE t.is_ms_shipped = 0
             ORDER BY s.name, t.name",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            Ok(TableSummary {
                schema: row
                    .get::<&str, _>(0)
                    .ok_or_else(|| "SQL Server returned a NULL schema name.".to_string())?
                    .to_string(),
                name: row
                    .get::<&str, _>(1)
                    .ok_or_else(|| "SQL Server returned a NULL table name.".to_string())?
                    .to_string(),
                editable: row.get::<i32, _>(2).unwrap_or(0) != 0,
            })
        })
        .collect()
}

pub async fn list_columns(session: &mut MssqlSession) -> Result<Vec<ColumnCatalogEntry>, String> {
    let rows = session
        .client
        .query(
            "SELECT s.name, o.name, c.name
             FROM sys.columns c
             JOIN sys.objects o ON c.object_id = o.object_id
             JOIN sys.schemas s ON o.schema_id = s.schema_id
             WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0
             ORDER BY s.name, o.name, c.column_id",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            Ok(ColumnCatalogEntry {
                schema: row
                    .get::<&str, _>(0)
                    .ok_or_else(|| "SQL Server returned a NULL schema name.".to_string())?
                    .to_string(),
                table: row
                    .get::<&str, _>(1)
                    .ok_or_else(|| "SQL Server returned a NULL table name.".to_string())?
                    .to_string(),
                column: row
                    .get::<&str, _>(2)
                    .ok_or_else(|| "SQL Server returned a NULL column name.".to_string())?
                    .to_string(),
            })
        })
        .collect()
}

pub async fn list_schema_objects(session: &mut MssqlSession) -> Result<Vec<SchemaObject>, String> {
    let rows = session
        .client
        .query(
            "SELECT s.name, t.name, CAST('table' AS nvarchar(16))
             FROM sys.tables t
             JOIN sys.schemas s ON s.schema_id = t.schema_id
             WHERE t.is_ms_shipped = 0
             UNION ALL
             SELECT s.name, v.name, CAST('view' AS nvarchar(16))
             FROM sys.views v
             JOIN sys.schemas s ON s.schema_id = v.schema_id
             WHERE v.is_ms_shipped = 0
             UNION ALL
             SELECT s.name, o.name, CAST('function' AS nvarchar(16))
             FROM sys.objects o
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             WHERE o.type IN ('FN', 'IF', 'TF') AND o.is_ms_shipped = 0
             UNION ALL
             SELECT s.name, tr.name, CAST('trigger' AS nvarchar(16))
             FROM sys.triggers tr
             JOIN sys.objects o ON o.object_id = tr.parent_id
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             WHERE tr.is_ms_shipped = 0 AND tr.parent_class = 1
             ORDER BY 1, 2, 3",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let kind = mssql_string(&row, 2)?;
            Ok(SchemaObject {
                schema: mssql_string(&row, 0)?,
                name: mssql_string(&row, 1)?,
                kind: match kind.as_str() {
                    "table" => SchemaObjectKind::Table,
                    "view" => SchemaObjectKind::View,
                    "function" => SchemaObjectKind::Function,
                    "trigger" => SchemaObjectKind::Trigger,
                    other => return Err(format!("Unexpected SQL Server object type: {other}")),
                },
            })
        })
        .collect()
}

pub async fn list_foreign_keys(
    session: &mut MssqlSession,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let mut query = Query::new(
        "SELECT ps.name, pt.name, pc.name,
                rs.name, rt.name, rc.name,
                fk.name
         FROM sys.foreign_keys fk
         JOIN sys.foreign_key_columns fkc
           ON fkc.constraint_object_id = fk.object_id
         JOIN sys.tables pt ON pt.object_id = fkc.parent_object_id
         JOIN sys.schemas ps ON ps.schema_id = pt.schema_id
         JOIN sys.columns pc
           ON pc.object_id = fkc.parent_object_id
          AND pc.column_id = fkc.parent_column_id
         JOIN sys.tables rt ON rt.object_id = fkc.referenced_object_id
         JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
         JOIN sys.columns rc
           ON rc.object_id = fkc.referenced_object_id
          AND rc.column_id = fkc.referenced_column_id
         WHERE (ps.name = @P1 AND pt.name = @P2)
            OR (rs.name = @P3 AND rt.name = @P4)
         ORDER BY ps.name, pt.name, fk.name, fkc.constraint_column_id",
    );
    query.bind(schema);
    query.bind(table);
    query.bind(schema);
    query.bind(table);
    let results = query
        .query(&mut session.client)
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    results
        .into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            Ok(ForeignKeyInfo {
                schema: mssql_string(&row, 0)?,
                table: mssql_string(&row, 1)?,
                column: mssql_string(&row, 2)?,
                referenced_schema: mssql_string(&row, 3)?,
                referenced_table: mssql_string(&row, 4)?,
                referenced_column: mssql_string(&row, 5)?,
                constraint_name: mssql_string(&row, 6)?,
            })
        })
        .collect()
}

fn mssql_string(row: &tiberius::Row, index: usize) -> Result<String, String> {
    row.get::<&str, _>(index)
        .ok_or_else(|| "SQL Server returned a NULL metadata value.".to_string())
        .map(str::to_string)
}

pub async fn list_database_metadata(
    session: &mut MssqlSession,
) -> Result<Vec<MetadataGroup>, String> {
    let mut groups = Vec::new();

    let rows = session
        .client
        .query(
            "SELECT TOP (200) name, type_desc, is_disabled
             FROM sys.server_principals
             WHERE type IN ('S', 'U', 'G') AND name NOT LIKE '##%'
             ORDER BY name",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let disabled = row.get::<bool, _>(2).unwrap_or(false);
            Ok(MetadataItem {
                name: mssql_string(&row, 0)?,
                detail: format!(
                    "{}, {}",
                    mssql_string(&row, 1)?,
                    if disabled { "disabled" } else { "enabled" }
                ),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !items.is_empty() {
        groups.push(MetadataGroup {
            name: "Logins".into(),
            items,
        });
    }

    let rows = session
        .client
        .query(
            "SELECT f.name, f.type_desc, COUNT(df.file_id)
             FROM sys.filegroups f
             LEFT JOIN sys.database_files df
               ON df.data_space_id = f.data_space_id
             GROUP BY f.name, f.type_desc
             ORDER BY f.name",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            Ok(MetadataItem {
                name: mssql_string(&row, 0)?,
                detail: format!(
                    "{}, {} file(s)",
                    mssql_string(&row, 1)?,
                    row.get::<i32, _>(2).unwrap_or(0)
                ),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !items.is_empty() {
        groups.push(MetadataGroup {
            name: "Storage".into(),
            items,
        });
    }

    let row = session
        .client
        .query(
            "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)),
                    CAST(SERVERPROPERTY('Edition') AS nvarchar(128)),
                    CAST(SERVERPROPERTY('Collation') AS nvarchar(128)),
                    DB_NAME(), SUSER_SNAME()",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_row()
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "SQL Server returned no server details.".to_string())?;
    groups.push(MetadataGroup {
        name: "System Info".into(),
        items: vec![
            MetadataItem {
                name: "Version".into(),
                detail: mssql_string(&row, 0)?,
            },
            MetadataItem {
                name: "Edition".into(),
                detail: mssql_string(&row, 1)?,
            },
            MetadataItem {
                name: "Collation".into(),
                detail: mssql_string(&row, 2)?,
            },
            MetadataItem {
                name: "Database".into(),
                detail: mssql_string(&row, 3)?,
            },
            MetadataItem {
                name: "Login".into(),
                detail: mssql_string(&row, 4)?,
            },
        ],
    });

    Ok(groups)
}

pub async fn load_page(
    session: &mut MssqlSession,
    schema: &str,
    table: &str,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<TablePage, String> {
    let meta = load_meta(&mut session.client, schema, table).await?;
    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let selected = meta
        .columns
        .iter()
        .map(display_expression)
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

    let mut query = Query::new(sql);
    for value in &params {
        query.bind(value.as_deref());
    }
    let results = query
        .query(&mut session.client)
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    let mut page_rows = Vec::with_capacity(results.first().map_or(0, Vec::len));
    for row in results.into_iter().next().unwrap_or_default() {
        let values = row
            .cells()
            .map(|(_, value)| column_text(value))
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
    session: &mut MssqlSession,
    schema: &str,
    table: &str,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if has_open_transaction(session).await? {
        return Err(
            "Commit or roll back the SQL tab's transaction before applying grid edits.".into(),
        );
    }
    let meta = load_meta(&mut session.client, schema, table).await?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }
    session
        .client
        .begin_transaction()
        .await
        .map_err(|error| error.to_string())?;
    let mut applied = 0;
    for change in changes {
        let built = match change {
            GridChange::Insert { values } => build_insert(schema, table, &meta, values),
            GridChange::Update {
                values, original, ..
            } => build_update(schema, table, &meta, values, original),
            GridChange::Delete { original, .. } => build_delete(schema, table, &meta, original),
        };
        let (sql, params) = match built {
            Ok(built) => built,
            Err(error) => {
                let _ = session.client.rollback_transaction().await;
                return Err(error);
            }
        };
        let mut query = Query::new(sql);
        for value in &params {
            query.bind(value.as_deref());
        }
        match query.execute(&mut session.client).await {
            Ok(result) => {
                let count = result.rows_affected().iter().copied().sum::<u64>();
                if let Err(error) = common::ensure_one_row(count) {
                    let _ = session.client.rollback_transaction().await;
                    return Err(error);
                }
                applied += 1;
            }
            Err(error) => {
                let _ = session.client.rollback_transaction().await;
                return Err(error.to_string());
            }
        }
    }
    session
        .client
        .commit_transaction()
        .await
        .map_err(|error| format!("COMMIT_OUTCOME_UNKNOWN: {error}"))?;
    Ok(ApplySummary { applied })
}

pub async fn run_sql(session: &mut MssqlSession, sql: &str) -> Result<QueryOutput, String> {
    let mut stream = session
        .client
        .simple_query(sql)
        .await
        .map_err(|error| error.to_string())?;
    let mut result_sets: Vec<QueryResultSet> = Vec::new();
    let mut truncated = false;
    while let Some(item) = futures_util::TryStreamExt::try_next(&mut stream)
        .await
        .map_err(|error| error.to_string())?
    {
        match item {
            QueryItem::Metadata(metadata) => {
                let index = metadata.result_index();
                while result_sets.len() <= index {
                    result_sets.push(QueryResultSet {
                        columns: Vec::new(),
                        rows: Vec::new(),
                    });
                }
                result_sets[index] = QueryResultSet {
                    columns: metadata
                        .columns()
                        .iter()
                        .map(|column| column.name().to_string())
                        .collect(),
                    rows: Vec::new(),
                };
            }
            QueryItem::Row(row) => {
                let index = row.result_index();
                while result_sets.len() <= index {
                    result_sets.push(QueryResultSet {
                        columns: row
                            .columns()
                            .iter()
                            .map(|column| column.name().to_string())
                            .collect(),
                        rows: Vec::new(),
                    });
                }
                let result = &mut result_sets[index];
                if result.rows.len() >= QUERY_ROW_LIMIT {
                    truncated = true;
                    continue;
                }
                result
                    .rows
                    .push(row.cells().map(|(_, value)| column_text(value)).collect());
            }
        }
    }
    drop(stream);

    let affected_rows = if result_sets.is_empty() {
        let mut affected = Vec::new();
        if let Ok(stream) = session.client.query("SELECT @@ROWCOUNT", &[]).await {
            if let Ok(Some(row)) = stream.into_row().await {
                if let Some(count) = row.get::<i32, _>(0).filter(|count| *count > 0) {
                    affected.push(count as u64);
                }
            }
        }
        affected
    } else {
        Vec::new()
    };
    Ok(QueryOutput {
        result_sets,
        affected_rows,
        truncated,
    })
}

pub async fn has_open_transaction(session: &mut MssqlSession) -> Result<bool, String> {
    let value = session
        .client
        .query("SELECT @@TRANCOUNT", &[])
        .await
        .map_err(|error| error.to_string())?
        .into_row()
        .await
        .map_err(|error| error.to_string())?
        .and_then(|row| row.get::<i32, _>(0))
        .unwrap_or(0);
    Ok(value > 0)
}

pub async fn cancel_query(config: &Config, spid: i16) -> Result<(), String> {
    let mut client = open_client(config.clone()).await?;
    client
        .simple_query(format!("KILL {spid}"))
        .await
        .map_err(|error| format!("Could not cancel SQL Server query: {error}"))?
        .into_results()
        .await
        .map_err(|error| format!("Could not cancel SQL Server query: {error}"))?;
    Ok(())
}

async fn open_client(config: Config) -> Result<MssqlClient, String> {
    let tcp = TcpStream::connect(config.get_addr())
        .await
        .map_err(|error| format!("SQL Server connection failed: {error}"))?;
    tcp.set_nodelay(true)
        .map_err(|error| format!("Could not configure SQL Server socket: {error}"))?;
    Client::connect(config, tcp.compat_write())
        .await
        .map_err(|error| format!("SQL Server connection failed: {error}"))
}

struct MssqlMeta {
    columns: Vec<ColumnInfo>,
    primary_indices: Vec<usize>,
}

impl MssqlMeta {
    fn editable(&self) -> bool {
        !self.primary_indices.is_empty()
    }
}

async fn load_meta(
    client: &mut MssqlClient,
    schema: &str,
    table: &str,
) -> Result<MssqlMeta, String> {
    let mut query = Query::new(
        "SELECT c.name, ty.name, c.max_length, c.precision, c.scale,
                c.is_nullable, CASE WHEN pk.column_id IS NULL THEN 0 ELSE 1 END,
                c.is_computed, c.is_identity
         FROM sys.columns c
         JOIN sys.types ty ON ty.user_type_id = c.user_type_id
         JOIN sys.tables t ON t.object_id = c.object_id
         JOIN sys.schemas s ON s.schema_id = t.schema_id
         LEFT JOIN (
             SELECT ic.object_id, ic.column_id
             FROM sys.index_columns ic
             JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id
             WHERE i.is_primary_key = 1
         ) pk ON pk.object_id = c.object_id AND pk.column_id = c.column_id
         WHERE s.name = @P1 AND t.name = @P2
         ORDER BY c.column_id",
    );
    query.bind(schema);
    query.bind(table);
    let results = query
        .query(client)
        .await
        .map_err(|error| error.to_string())?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    let rows = results.into_iter().next().unwrap_or_default();
    if rows.is_empty() {
        return Err("Table has no visible columns.".into());
    }
    let columns = rows
        .into_iter()
        .map(|row| {
            let base_type = row
                .get::<&str, _>(1)
                .ok_or_else(|| "SQL Server returned a NULL column type.".to_string())?
                .to_string();
            let data_type = display_type(
                &base_type,
                row.get::<i16, _>(2).unwrap_or(0),
                row.get::<u8, _>(3).unwrap_or(0),
                row.get::<u8, _>(4).unwrap_or(0),
            );
            Ok(ColumnInfo {
                name: row
                    .get::<&str, _>(0)
                    .ok_or_else(|| "SQL Server returned a NULL column name.".to_string())?
                    .to_string(),
                data_type,
                filter_type: base_type.clone(),
                filter_kind: common::filter_kind(&base_type),
                sortable: !matches!(
                    base_type.as_str(),
                    "text" | "ntext" | "image" | "xml" | "geography" | "geometry"
                ),
                nullable: row.get::<bool, _>(5).unwrap_or(false),
                primary_key: row.get::<i32, _>(6).unwrap_or(0) != 0,
                generated: row.get::<bool, _>(7).unwrap_or(false),
                identity: row.get::<bool, _>(8).unwrap_or(false),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let primary_indices = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| index)
        .collect();
    Ok(MssqlMeta {
        columns,
        primary_indices,
    })
}

fn display_type(base_type: &str, max_length: i16, precision: u8, scale: u8) -> String {
    match base_type {
        "nvarchar" | "nchar" => {
            if max_length == -1 {
                format!("{base_type}(max)")
            } else {
                format!("{base_type}({})", i32::from(max_length) / 2)
            }
        }
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length == -1 {
                format!("{base_type}(max)")
            } else {
                format!("{base_type}({max_length})")
            }
        }
        "decimal" | "numeric" => format!("{base_type}({precision},{scale})"),
        "datetime2" | "time" | "datetimeoffset" => {
            format!("{base_type}({scale})")
        }
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
                    "CHARINDEX(LOWER(@P{}), LOWER(CONVERT(nvarchar(max), {identifier}))) > 0",
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
                format!("{identifier} {operator} @P{}", params.len())
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
    meta: &MssqlMeta,
    values: BTreeMap<String, Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    let mut names = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());
    let mut params = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = common::find_column(&meta.columns, &name)?;
        common::ensure_writable(column)?;
        names.push(quote_identifier(&name));
        let placeholder = format!("@P{}", params.len() + 1);
        placeholders.push(binary_assignment(column, &value, &placeholder)?);
        params.push(value);
    }
    if names.is_empty() {
        return Ok((
            format!(
                "INSERT INTO {} DEFAULT VALUES",
                quote_relation(schema, table)
            ),
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
    meta: &MssqlMeta,
    values: BTreeMap<String, Option<String>>,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
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
        let placeholder = format!("@P{}", params.len() + 1);
        assignments.push(format!(
            "{} = {}",
            quote_identifier(&name),
            binary_assignment(column, &value, &placeholder)?
        ));
        params.push(value);
    }
    let predicate = original_predicate(&meta.columns, &original, &mut params);
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
    meta: &MssqlMeta,
    original: Vec<Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    if original.len() != meta.columns.len() {
        return Err("The table shape changed. Reload before retrying.".into());
    }
    let mut params = Vec::with_capacity(original.len());
    let predicate = original_predicate(&meta.columns, &original, &mut params);
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
    params: &mut Vec<Option<String>>,
) -> String {
    columns
        .iter()
        .zip(original)
        .map(|(column, value)| {
            params.push(value.clone());
            let placeholder = format!("@P{}", params.len());
            let expression = display_expression(column);
            format!(
                "({expression} = {placeholder} OR ({expression} IS NULL AND {placeholder} IS NULL))"
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn is_binary(column: &ColumnInfo) -> bool {
    let declared = column.filter_type.to_ascii_lowercase();
    declared.contains("binary") || declared == "image"
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

fn binary_assignment(
    column: &ColumnInfo,
    value: &Option<String>,
    placeholder: &str,
) -> Result<String, String> {
    if !is_binary(column) {
        return Ok(placeholder.to_string());
    }
    match value {
        Some(_) => {
            ensure_binary_hex(value)?;
            Ok(format!("CONVERT(varbinary(max), {placeholder}, 2)"))
        }
        None => Ok(placeholder.to_string()),
    }
}

fn display_expression(column: &ColumnInfo) -> String {
    let identifier = quote_identifier(&column.name);
    if matches!(column.filter_type.as_str(), "float" | "real") {
        format!("CONVERT(nvarchar(max), {identifier}, 3)")
    } else if is_binary(column) {
        format!("CONVERT(nvarchar(max), {identifier}, 2)")
    } else if matches!(
        column.filter_type.as_str(),
        "date" | "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" | "time"
    ) {
        format!("CONVERT(nvarchar(max), {identifier}, 126)")
    } else {
        format!("CONVERT(nvarchar(max), {identifier})")
    }
}

fn column_text(value: &ColumnData<'_>) -> Option<String> {
    match value {
        ColumnData::U8(value) => value.map(|value| value.to_string()),
        ColumnData::I16(value) => value.map(|value| value.to_string()),
        ColumnData::I32(value) => value.map(|value| value.to_string()),
        ColumnData::I64(value) => value.map(|value| value.to_string()),
        ColumnData::F32(value) => value.map(|value| value.to_string()),
        ColumnData::F64(value) => value.map(|value| value.to_string()),
        ColumnData::Bit(value) => value.map(|value| value.to_string()),
        ColumnData::String(value) => value.as_ref().map(|value| value.to_string()),
        ColumnData::Guid(value) => value.map(|value| value.to_string()),
        ColumnData::Binary(value) => value.as_ref().map(|value| hex(value)),
        ColumnData::Numeric(value) => value.map(|value| value.to_string()),
        ColumnData::Xml(value) => value.as_ref().map(|value| value.to_string()),
        ColumnData::DateTime(value) => value.map(datetime_text),
        ColumnData::SmallDateTime(value) => value.map(small_datetime_text),
        ColumnData::Time(value) => value.map(time_text),
        ColumnData::Date(value) => value.map(date_text),
        ColumnData::DateTime2(value) => value.map(datetime2_text),
        ColumnData::DateTimeOffset(value) => value.map(datetime_offset_text),
    }
}

fn datetime_text(value: DateTime) -> String {
    let date = NaiveDate::from_ymd_opt(1900, 1, 1)
        .expect("valid base date")
        .checked_add_signed(TimeDelta::days(i64::from(value.days())))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1900, 1, 1).expect("valid base date"));
    let seconds = value.seconds_fragments() / 300;
    let millis = ((value.seconds_fragments() % 300) as f64 / 300.0 * 1000.0).round() as u32;
    format!(
        "{} {:02}:{:02}:{:02}.{:03}",
        date.format("%Y-%m-%d"),
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60,
        millis
    )
}

fn small_datetime_text(value: SmallDateTime) -> String {
    let date = NaiveDate::from_ymd_opt(1900, 1, 1)
        .expect("valid base date")
        .checked_add_signed(TimeDelta::days(i64::from(value.days())))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1900, 1, 1).expect("valid base date"));
    let seconds = u32::from(value.seconds_fragments()) / 300;
    format!(
        "{} {:02}:{:02}:{:02}",
        date.format("%Y-%m-%d"),
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

fn date_text(value: Date) -> String {
    NaiveDate::from_ymd_opt(1, 1, 1)
        .expect("valid base date")
        .checked_add_signed(TimeDelta::days(i64::from(value.days())))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1, 1, 1).expect("valid base date"))
        .format("%Y-%m-%d")
        .to_string()
}

fn time_text(value: Time) -> String {
    let divisor = 10u64.saturating_pow(u32::from(value.scale()));
    let total_nanos = if divisor == 0 {
        0
    } else {
        value.increments().saturating_mul(1_000_000_000) / divisor
    };
    let total_seconds = total_nanos / 1_000_000_000;
    let nanos = total_nanos % 1_000_000_000;
    format!(
        "{:02}:{:02}:{:02}.{:07}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60,
        nanos / 100
    )
}

fn datetime2_text(value: DateTime2) -> String {
    format!("{} {}", date_text(value.date()), time_text(value.time()))
}

fn datetime_offset_text(value: DateTimeOffset) -> String {
    let offset = value.offset();
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.unsigned_abs();
    format!(
        "{} {}{:02}:{:02}",
        datetime2_text(value.datetime2()),
        sign,
        offset / 60,
        offset % 60
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn quote_relation(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
}

fn quote_identifier(identifier: &str) -> String {
    common::quote_mssql_identifier(identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{GridChange, SchemaObjectKind};
    use std::collections::BTreeMap;

    #[tokio::test]
    #[ignore]
    async fn mssql_workflow() {
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "mssql test".into(),
            engine: Engine::SqlServer,
            host: std::env::var("KOYO_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("KOYO_TEST_MSSQL_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1433),
            database: "koyo".into(),
            username: "sa".into(),
            ssl_mode: SslMode::Disable,
            sqlite_path: String::new(),
        };
        let (mut session, _version, _user, _config) =
            connect(&profile, "Koyo!Passw0rd", Engine::SqlServer)
                .await
                .expect("connect");

        let tables = list_tables(&mut session).await.expect("list tables");
        assert!(
            tables
                .iter()
                .any(|table| table.schema == "dbo" && table.name == "users")
        );

        let objects = list_schema_objects(&mut session)
            .await
            .expect("list schema objects");
        assert!(objects.iter().any(|object| object.schema == "dbo"
            && object.name == "users"
            && object.kind == SchemaObjectKind::Table));
        let keys = list_foreign_keys(&mut session, "dbo", "users")
            .await
            .expect("list foreign keys");
        assert!(
            keys.iter()
                .all(|key| key.schema == "dbo" && key.table == "users")
        );

        let groups = list_database_metadata(&mut session)
            .await
            .expect("list database metadata");
        assert!(groups.iter().any(|group| group.name == "System Info"));
        assert!(groups.iter().any(|group| group.name == "Storage"));
        assert!(groups.iter().any(|group| group.name == "Logins"));
        assert!(!groups.iter().any(|group| group.name == "Schemas"));
        assert!(groups.iter().all(|group| !group.items.is_empty()));

        run_sql(
            &mut session,
            "DELETE FROM dbo.users WHERE email LIKE N'new-%@example.com'",
        )
        .await
        .expect("cleanup");
        let page = load_page(&mut session, "dbo", "users", 0, 100, &[], None)
            .await
            .expect("page");
        assert!(page.editable);
        let initial_rows = page.rows.len();
        assert!(initial_rows >= 2);
        let filtered = load_page(
            &mut session,
            "dbo",
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
            "dbo",
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
            "dbo",
            "users",
            0,
            100,
            &[TableFilter {
                column: "created_at".into(),
                operator: FilterOperator::GreaterThan,
                value: Some("2000-01-01T00:00:00".into()),
            }],
            None,
        )
        .await
        .expect("temporal filter");
        assert_eq!(temporal.rows.len(), initial_rows);

        let inserted_email = format!("new-{}@example.com", uuid::Uuid::new_v4());
        apply_changes(
            &mut session,
            "dbo",
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

        let page = load_page(&mut session, "dbo", "users", 0, 100, &[], None)
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
            "dbo",
            "users",
            vec![GridChange::Update {
                key: new_row.key.clone(),
                version: String::new(),
                values: BTreeMap::from([
                    ("name".into(), Some("Renamed".into())),
                    ("created_at".into(), Some("2025-01-02T03:04:05".into())),
                ]),
                original: new_row.values.clone(),
            }],
        )
        .await
        .expect("update");

        let page = load_page(&mut session, "dbo", "users", 0, 100, &[], None)
            .await
            .expect("page after update");
        let new_row = page
            .rows
            .iter()
            .find(|row| row.values[1].as_deref() == Some(inserted_email.as_str()))
            .cloned()
            .expect("updated row");
        assert_eq!(new_row.values[3].as_deref(), Some("2025-01-02T03:04:05"));
        apply_changes(
            &mut session,
            "dbo",
            "users",
            vec![GridChange::Delete {
                key: new_row.key,
                version: String::new(),
                original: new_row.values,
            }],
        )
        .await
        .expect("delete");

        let output = run_sql(&mut session, "SELECT COUNT(*) AS n FROM dbo.users")
            .await
            .expect("select");
        assert_eq!(
            output.result_sets[0].rows[0][0].as_deref(),
            Some(initial_rows.to_string().as_str())
        );
        let output = run_sql(
            &mut session,
            "UPDATE dbo.users SET name = name WHERE id = 1",
        )
        .await
        .expect("update through SQL");
        assert_eq!(output.affected_rows, vec![1]);

        let binary_table = format!("koyo_bin_{}", uuid::Uuid::new_v4().simple());
        run_sql(
            &mut session,
            &format!("CREATE TABLE dbo.{binary_table} (id INT PRIMARY KEY, payload VARBINARY(16))"),
        )
        .await
        .expect("create binary fixture");
        run_sql(
            &mut session,
            &format!("INSERT INTO dbo.{binary_table} VALUES (1, 0x00AA)"),
        )
        .await
        .expect("insert binary fixture");
        let binary_page = load_page(&mut session, "dbo", &binary_table, 0, 100, &[], None)
            .await
            .expect("load binary page");
        assert_eq!(binary_page.rows[0].values[1].as_deref(), Some("00AA"));
        apply_changes(
            &mut session,
            "dbo",
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
            &format!(
                "SELECT CONVERT(varchar(max), payload, 2) FROM dbo.{binary_table} WHERE id = 1"
            ),
        )
        .await
        .expect("read binary");
        assert_eq!(
            binary_output.result_sets[0].rows[0][0].as_deref(),
            Some("DEADBEEF")
        );
        run_sql(&mut session, &format!("DROP TABLE dbo.{binary_table}"))
            .await
            .expect("drop binary fixture");

        run_sql(&mut session, "BEGIN TRANSACTION")
            .await
            .expect("begin");
        assert!(has_open_transaction(&mut session).await.expect("status"));
        let blocked = apply_changes(&mut session, "dbo", "users", Vec::new()).await;
        assert!(
            blocked
                .as_ref()
                .is_err_and(|error| error.contains("before applying grid edits")),
            "grid apply must not commit an open SQL transaction: {blocked:?}"
        );
        run_sql(&mut session, "ROLLBACK TRANSACTION")
            .await
            .expect("rollback");
        assert!(!has_open_transaction(&mut session).await.expect("status"));
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    #[ignore]
    async fn mssql_cancel_running_query() {
        let profile = ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "mssql cancel".into(),
            engine: Engine::SqlServer,
            host: std::env::var("KOYO_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("KOYO_TEST_MSSQL_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(11433),
            database: "koyo".into(),
            username: "sa".into(),
            ssl_mode: SslMode::Disable,
            sqlite_path: String::new(),
        };
        let (mut session, _version, _user, config) =
            connect(&profile, "Koyo!Passw0rd", Engine::SqlServer)
                .await
                .expect("connect");
        let spid = session.spid;
        let started = std::time::Instant::now();
        let task =
            tokio::spawn(async move { run_sql(&mut session, "WAITFOR DELAY '00:00:10'").await });
        tokio::time::sleep(Duration::from_millis(1000)).await;
        cancel_query(&config, spid).await.expect("cancel");
        let _ = task.await.expect("query task");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancelled query should return before the delay ends"
        );
    }
}
