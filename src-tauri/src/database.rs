use crate::{
    models::{
        ApplySummary, ColumnInfo, ConnectionInfo, GridChange, GridRow, ProfileSummary, QueryOutput,
        QueryResultSet, TablePage, TableSummary,
    },
    storage,
};
use futures_util::TryStreamExt;
use rustls_tokio_postgres::{MakeRustlsConnect, config_platform_verifier};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::State;
use tokio::sync::Mutex;
use tokio_postgres::{CancelToken, Client, SimpleQueryMessage, config::SslMode, types::ToSql};

const QUERY_ROW_LIMIT: usize = 1_000;
const TABLE_PAGE_LIMIT: u32 = 200;
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct AppState {
    session: Mutex<Option<DbSession>>,
    cancel: Mutex<Option<CancelState>>,
    query_active: AtomicBool,
}

#[derive(Clone)]
struct CancelState {
    token: CancelToken,
    ssl_mode: crate::models::SslMode,
}

struct DbSession {
    client: Client,
}

struct TableMeta {
    columns: Vec<ColumnInfo>,
    relation_kind: String,
}

impl TableMeta {
    fn primary_columns(&self) -> impl Iterator<Item = &ColumnInfo> {
        self.columns.iter().filter(|column| column.primary_key)
    }

    fn editable(&self) -> bool {
        matches!(self.relation_kind.as_str(), "r" | "p")
            && self.columns.iter().any(|column| column.primary_key)
    }
}

#[tauri::command]
pub async fn connect_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: uuid::Uuid,
) -> Result<ConnectionInfo, String> {
    let profile = storage::load_profiles(&app)?
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "Connection profile does not exist.".to_string())?;
    let password = storage::load_password(profile.id)?;

    let mut config = tokio_postgres::Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .user(&profile.username)
        .password(&password)
        .dbname(&profile.database)
        .application_name("Koyo");
    config.ssl_mode(match profile.ssl_mode {
        crate::models::SslMode::VerifyFull => SslMode::Require,
        crate::models::SslMode::Disable => SslMode::Disable,
    });

    let tls_config = match profile.ssl_mode {
        crate::models::SslMode::VerifyFull => config_platform_verifier()
            .map_err(|error| format!("Could not initialize system TLS verification: {error}"))?,
        // TLS is off for this profile, so this config is unreachable but keeps local
        // connections working when the platform trust store is unavailable.
        crate::models::SslMode::Disable => rustls_tokio_postgres::config_no_verify(),
    };
    let tls = MakeRustlsConnect::new(tls_config);
    let (client, connection) = config
        .connect(tls)
        .await
        .map_err(|error| format!("PostgreSQL connection failed: {error}"))?;

    tauri::async_runtime::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("PostgreSQL connection closed: {error}");
        }
    });

    let row = client
        .query_one(
            "SELECT current_user, current_setting('server_version')",
            &[],
        )
        .await
        .map_err(|error| format!("Could not read PostgreSQL server details: {error}"))?;
    let current_user: String = row.get(0);
    let server_version: String = row.get(1);

    let info = ConnectionInfo {
        profile: ProfileSummary::from(&profile),
        server_version,
        current_user,
    };
    let cancel = CancelState {
        token: client.cancel_token(),
        ssl_mode: profile.ssl_mode,
    };
    *state.cancel.lock().await = Some(cancel);
    state.query_active.store(false, Ordering::SeqCst);
    *state.session.lock().await = Some(DbSession { client });
    Ok(info)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    if state.query_active.load(Ordering::SeqCst) {
        if let Some(cancel) = state.cancel.lock().await.clone() {
            let _ = send_cancel(&cancel).await;
        }
    }
    let mut session = tokio::time::timeout(DISCONNECT_TIMEOUT, state.session.lock())
        .await
        .map_err(|_| {
            "A database operation is still running. Cancel it and try again.".to_string()
        })?;
    *session = None;
    *state.cancel.lock().await = None;
    state.query_active.store(false, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn cancel_query(state: State<'_, AppState>) -> Result<(), String> {
    if !state.query_active.load(Ordering::SeqCst) {
        return Err("No query is running.".into());
    }
    let cancel = state
        .cancel
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Connect to PostgreSQL first.".to_string())?;
    send_cancel(&cancel).await
}

async fn send_cancel(cancel: &CancelState) -> Result<(), String> {
    match cancel.ssl_mode {
        crate::models::SslMode::VerifyFull => {
            let tls = config_platform_verifier().map_err(|error| {
                format!("Could not initialize system TLS verification: {error}")
            })?;
            cancel
                .token
                .cancel_query(MakeRustlsConnect::new(tls))
                .await
                .map_err(|error| error.to_string())
        }
        crate::models::SslMode::Disable => cancel
            .token
            .cancel_query(tokio_postgres::NoTls)
            .await
            .map_err(|error| error.to_string()),
    }
}

#[tauri::command]
pub async fn list_tables(state: State<'_, AppState>) -> Result<Vec<TableSummary>, String> {
    let session = state.session.lock().await;
    let session = session
        .as_ref()
        .ok_or_else(|| "Connect to PostgreSQL first.".to_string())?;
    let rows = session
        .client
        .query(
            "SELECT n.nspname::text, t.relname::text,
                    EXISTS (
                        SELECT 1 FROM pg_catalog.pg_index i
                        WHERE i.indrelid = t.oid AND i.indisprimary
                    )
             FROM pg_catalog.pg_class t
             JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
             WHERE t.relkind IN ('r', 'p')
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg_toast%'
             ORDER BY n.nspname, t.relname",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            Ok(TableSummary {
                schema: row.try_get(0).map_err(|error| error.to_string())?,
                name: row.try_get(1).map_err(|error| error.to_string())?,
                editable: row.try_get(2).map_err(|error| error.to_string())?,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn load_table_page(
    state: State<'_, AppState>,
    schema: String,
    table: String,
    offset: u32,
    limit: u32,
) -> Result<TablePage, String> {
    let session = state.session.lock().await;
    let session = session
        .as_ref()
        .ok_or_else(|| "Connect to PostgreSQL first.".to_string())?;
    let meta = load_table_meta(&session.client, &schema, &table).await?;
    if meta.columns.is_empty() {
        return Err("Table has no visible columns.".into());
    }

    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let fetch_limit = limit + 1;
    let selected = meta
        .columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let order = meta
        .primary_columns()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>();
    let order = if order.is_empty() {
        "ctid".to_string()
    } else {
        order.join(", ")
    };
    let sql = format!(
        "SELECT xmin::text, {selected} FROM {}.{} ORDER BY {order} LIMIT {fetch_limit} OFFSET {offset}",
        quote_identifier(&schema),
        quote_identifier(&table)
    );

    let messages = session
        .client
        .simple_query(&sql)
        .await
        .map_err(|error| error.to_string())?;
    let mut rows = Vec::with_capacity(fetch_limit as usize);
    for message in messages {
        if let SimpleQueryMessage::Row(row) = message {
            let version = row
                .get(0)
                .ok_or_else(|| "PostgreSQL returned a row without a version.".to_string())?
                .to_owned();
            let values = (0..meta.columns.len())
                .map(|index| row.get(index + 1).map(str::to_owned))
                .collect::<Vec<_>>();
            let key = meta
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| column.primary_key)
                .map(|(index, _)| values[index].clone())
                .collect();
            rows.push(GridRow {
                version,
                key,
                values,
            });
        }
    }

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let editable = meta.editable();
    Ok(TablePage {
        columns: meta.columns,
        rows,
        offset,
        limit,
        has_more,
        editable,
    })
}

#[tauri::command]
pub async fn apply_grid_changes(
    state: State<'_, AppState>,
    schema: String,
    table: String,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if changes.is_empty() {
        return Ok(ApplySummary { applied: 0 });
    }
    let mut session_guard = state.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Connect to PostgreSQL first.".to_string())?;
    let meta = load_table_meta(&session.client, &schema, &table).await?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }

    let transaction = session
        .client
        .transaction()
        .await
        .map_err(|error| error.to_string())?;
    let mut applied = 0;
    for change in changes {
        match change {
            GridChange::Insert { values } => {
                let (sql, params) = build_insert(&schema, &table, &meta, values)?;
                let refs = sql_params(&params);
                let count = transaction
                    .execute(&sql, &refs)
                    .await
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
            GridChange::Update {
                key,
                version,
                values,
            } => {
                let (sql, params) = build_update(&schema, &table, &meta, key, version, values)?;
                let refs = sql_params(&params);
                let count = transaction
                    .execute(&sql, &refs)
                    .await
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
            GridChange::Delete { key, version } => {
                let (sql, params) = build_delete(&schema, &table, &meta, key, version)?;
                let refs = sql_params(&params);
                let count = transaction
                    .execute(&sql, &refs)
                    .await
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
        }
        applied += 1;
    }

    transaction
        .commit()
        .await
        .map_err(|error| format!("COMMIT_OUTCOME_UNKNOWN: {error}"))?;
    Ok(ApplySummary { applied })
}

#[tauri::command]
pub async fn run_sql(state: State<'_, AppState>, sql: String) -> Result<QueryOutput, String> {
    if sql.trim().is_empty() {
        return Err("Enter SQL to run.".into());
    }
    let session = state.session.lock().await;
    let session = session
        .as_ref()
        .ok_or_else(|| "Connect to PostgreSQL first.".to_string())?;
    state.query_active.store(true, Ordering::SeqCst);
    let result = collect_query(&session.client, &sql).await;
    state.query_active.store(false, Ordering::SeqCst);
    result
}

async fn collect_query(client: &Client, sql: &str) -> Result<QueryOutput, String> {
    let stream = client
        .simple_query_raw(sql)
        .await
        .map_err(|error| error.to_string())?;
    futures_util::pin_mut!(stream);

    let mut result_sets: Vec<QueryResultSet> = Vec::new();
    let mut affected_rows = Vec::new();
    let mut truncated = false;
    while let Some(message) = stream.try_next().await.map_err(|error| error.to_string())? {
        match message {
            SimpleQueryMessage::RowDescription(columns) => {
                result_sets.push(QueryResultSet {
                    columns: columns
                        .iter()
                        .map(|column| column.name().to_owned())
                        .collect(),
                    rows: Vec::new(),
                });
            }
            SimpleQueryMessage::Row(row) => {
                let Some(result_set) = result_sets.last_mut() else {
                    continue;
                };
                if result_set.rows.len() < QUERY_ROW_LIMIT {
                    result_set.rows.push(
                        (0..row.len())
                            .map(|index| row.get(index).map(str::to_owned))
                            .collect(),
                    );
                } else {
                    truncated = true;
                }
            }
            SimpleQueryMessage::CommandComplete(count) => affected_rows.push(count),
            _ => {}
        }
    }

    Ok(QueryOutput {
        result_sets,
        affected_rows,
        truncated,
    })
}

async fn load_table_meta(client: &Client, schema: &str, table: &str) -> Result<TableMeta, String> {
    let rows = client
        .query(
            "SELECT t.relkind::text, a.attname::text,
                    pg_catalog.format_type(a.atttypid, a.atttypmod),
                    NOT a.attnotnull, a.attgenerated <> '', a.attidentity <> '',
                    COALESCE(i.indisprimary AND a.attnum = ANY(i.indkey), false)
             FROM pg_catalog.pg_class t
             JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid
             LEFT JOIN pg_catalog.pg_index i
               ON i.indrelid = t.oid AND i.indisprimary
             WHERE n.nspname::text = $1 AND t.relname::text = $2
               AND t.relkind IN ('r', 'p')
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .map_err(|error| error.to_string())?;
    if rows.is_empty() {
        return Err("Table does not exist or is not a regular PostgreSQL table.".into());
    }

    let mut columns = Vec::with_capacity(rows.len());
    let mut relation_kind = String::new();
    for row in rows {
        relation_kind = row.try_get(0).map_err(|error| error.to_string())?;
        columns.push(ColumnInfo {
            name: row.try_get(1).map_err(|error| error.to_string())?,
            data_type: row.try_get(2).map_err(|error| error.to_string())?,
            nullable: row.try_get(3).map_err(|error| error.to_string())?,
            generated: row.try_get(4).map_err(|error| error.to_string())?,
            identity: row.try_get(5).map_err(|error| error.to_string())?,
            primary_key: row.try_get(6).map_err(|error| error.to_string())?,
        });
    }
    Ok(TableMeta {
        columns,
        relation_kind,
    })
}

fn build_insert(
    schema: &str,
    table: &str,
    meta: &TableMeta,
    values: BTreeMap<String, Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    let mut params = Vec::with_capacity(values.len());
    let mut names = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = find_column(meta, &name)?;
        ensure_writable(column)?;
        names.push(quote_identifier(&name));
        params.push(value);
        placeholders.push(cast_placeholder(params.len(), column));
    }

    let relation = quote_relation(schema, table);
    let sql = if names.is_empty() {
        format!("INSERT INTO {relation} DEFAULT VALUES")
    } else {
        format!(
            "INSERT INTO {relation} ({}) VALUES ({})",
            names.join(", "),
            placeholders.join(", ")
        )
    };
    Ok((sql, params))
}

fn build_update(
    schema: &str,
    table: &str,
    meta: &TableMeta,
    key: Vec<Option<String>>,
    version: String,
    values: BTreeMap<String, Option<String>>,
) -> Result<(String, Vec<Option<String>>), String> {
    if values.is_empty() {
        return Err("Update contains no changed values.".into());
    }
    let mut params = Vec::with_capacity(values.len() + key.len() + 1);
    let mut assignments = Vec::with_capacity(values.len());
    for (name, value) in values {
        let column = find_column(meta, &name)?;
        ensure_writable(column)?;
        params.push(value);
        assignments.push(format!(
            "{} = {}",
            quote_identifier(&name),
            cast_placeholder(params.len(), column)
        ));
    }
    let predicate = append_key_predicate(meta, key, version, &mut params)?;
    let sql = format!(
        "UPDATE {} SET {} WHERE {predicate}",
        quote_relation(schema, table),
        assignments.join(", ")
    );
    Ok((sql, params))
}

fn build_delete(
    schema: &str,
    table: &str,
    meta: &TableMeta,
    key: Vec<Option<String>>,
    version: String,
) -> Result<(String, Vec<Option<String>>), String> {
    let mut params = Vec::with_capacity(key.len() + 1);
    let predicate = append_key_predicate(meta, key, version, &mut params)?;
    Ok((
        format!(
            "DELETE FROM {} WHERE {predicate}",
            quote_relation(schema, table)
        ),
        params,
    ))
}

fn append_key_predicate(
    meta: &TableMeta,
    key: Vec<Option<String>>,
    version: String,
    params: &mut Vec<Option<String>>,
) -> Result<String, String> {
    let primary_count = meta.primary_columns().count();
    if primary_count == 0 || key.len() != primary_count {
        return Err("Grid edit has invalid primary-key values.".into());
    }

    let mut predicates = Vec::with_capacity(primary_count + 1);
    for (column, value) in meta.primary_columns().zip(key) {
        if value.is_none() {
            return Err("Primary-key values cannot be NULL.".into());
        }
        params.push(value);
        predicates.push(format!(
            "{} = {}",
            quote_identifier(&column.name),
            cast_placeholder(params.len(), column)
        ));
    }
    params.push(Some(version));
    predicates.push(format!("xmin::text = ${}::text", params.len()));
    Ok(predicates.join(" AND "))
}

fn find_column<'a>(meta: &'a TableMeta, name: &str) -> Result<&'a ColumnInfo, String> {
    meta.columns
        .iter()
        .find(|column| column.name == name)
        .ok_or_else(|| format!("Unknown table column: {name}"))
}

fn ensure_writable(column: &ColumnInfo) -> Result<(), String> {
    if column.generated || column.identity {
        return Err(format!(
            "Column {} is generated and cannot be edited.",
            column.name
        ));
    }
    Ok(())
}

fn cast_placeholder(index: usize, column: &ColumnInfo) -> String {
    format!("${index}::text::{}", column.data_type)
}

fn sql_params(values: &[Option<String>]) -> Vec<&(dyn ToSql + Sync)> {
    values
        .iter()
        .map(|value| value as &(dyn ToSql + Sync))
        .collect()
}

fn ensure_one_row(count: u64) -> Result<(), String> {
    if count == 1 {
        Ok(())
    } else {
        Err(
            "A row changed or disappeared since it was loaded. The staged batch was rolled back."
                .into(),
        )
    }
}

fn quote_relation(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::quote_identifier;

    #[test]
    fn quotes_embedded_identifier_quotes() {
        assert_eq!(quote_identifier("odd\"table"), "\"odd\"\"table\"");
    }

    #[tokio::test]
    #[ignore = "requires KOYO_TEST_DATABASE_URL"]
    async fn applies_staged_grid_changes_and_rejects_stale_rows() {
        use super::{
            build_delete, build_insert, build_update, ensure_one_row, load_table_meta, sql_params,
        };
        use std::collections::BTreeMap;
        use tokio_postgres::NoTls;

        let url = std::env::var("KOYO_TEST_DATABASE_URL")
            .expect("set KOYO_TEST_DATABASE_URL to a disposable PostgreSQL database");
        let (mut client, connection) = tokio_postgres::connect(&url, NoTls)
            .await
            .expect("connect to test database");
        let connection_task = tokio::spawn(connection);
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let table = format!("koyo_grid_test_{suffix}");
        let keyless = format!("koyo_keyless_test_{suffix}");

        client
            .batch_execute(&format!(
                "CREATE TABLE public.{table} (
                     id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                     name text NOT NULL,
                     note text
                 );
                 CREATE TABLE public.{keyless} (label text);
                 INSERT INTO public.{table} (name, note) VALUES ('seed', 'old');"
            ))
            .await
            .expect("create fixture tables");

        let meta = load_table_meta(&client, "public", &table)
            .await
            .expect("read table metadata");
        assert!(meta.editable());
        assert!(
            !load_table_meta(&client, "public", &keyless)
                .await
                .expect("read keyless metadata")
                .editable()
        );

        let row = client
            .query_one(
                &format!("SELECT xmin::text, id::text FROM public.{table}"),
                &[],
            )
            .await
            .expect("read original row");
        let version: String = row.get(0);
        let id: String = row.get(1);
        let key = vec![Some(id)];

        let mut update_values = BTreeMap::new();
        update_values.insert("name".to_string(), Some("changed".to_string()));
        let (sql, params) = build_update(
            "public",
            &table,
            &meta,
            key.clone(),
            version.clone(),
            update_values,
        )
        .expect("build update");
        let transaction = client
            .transaction()
            .await
            .expect("begin update transaction");
        let count = transaction
            .execute(&sql, &sql_params(&params))
            .await
            .expect("execute update");
        ensure_one_row(count).expect("single row updated");
        transaction.commit().await.expect("commit update");

        let updated: String = client
            .query_one(&format!("SELECT name FROM public.{table}"), &[])
            .await
            .expect("read updated row")
            .get(0);
        assert_eq!(updated, "changed");

        let mut stale_values = BTreeMap::new();
        stale_values.insert("name".to_string(), Some("stale".to_string()));
        let (stale_sql, stale_params) = build_update(
            "public",
            &table,
            &meta,
            key.clone(),
            version.clone(),
            stale_values,
        )
        .expect("build stale update");
        let transaction = client.transaction().await.expect("begin stale transaction");
        let count = transaction
            .execute(&stale_sql, &sql_params(&stale_params))
            .await
            .expect("execute stale update");
        assert!(ensure_one_row(count).is_err());
        transaction
            .rollback()
            .await
            .expect("roll back stale update");

        let (delete_sql, delete_params) =
            build_delete("public", &table, &meta, key, version).expect("build stale delete");
        let transaction = client
            .transaction()
            .await
            .expect("begin delete transaction");
        let count = transaction
            .execute(&delete_sql, &sql_params(&delete_params))
            .await
            .expect("execute stale delete");
        assert!(ensure_one_row(count).is_err());
        transaction.rollback().await.expect("roll back delete");

        let mut insert_values = BTreeMap::new();
        insert_values.insert("name".to_string(), Some("new".to_string()));
        insert_values.insert("note".to_string(), None);
        let (insert_sql, insert_params) =
            build_insert("public", &table, &meta, insert_values).expect("build insert");
        let count = client
            .execute(&insert_sql, &sql_params(&insert_params))
            .await
            .expect("execute insert");
        ensure_one_row(count).expect("single row inserted");

        client
            .batch_execute(&format!(
                "DROP TABLE public.{table}; DROP TABLE public.{keyless};"
            ))
            .await
            .expect("drop fixture tables");
        connection_task.abort();
    }

    #[tokio::test]
    #[ignore = "requires KOYO_TEST_DATABASE_URL"]
    async fn cancels_a_running_statement() {
        use super::collect_query;
        use tokio_postgres::NoTls;

        let url = std::env::var("KOYO_TEST_DATABASE_URL")
            .expect("set KOYO_TEST_DATABASE_URL to a disposable PostgreSQL database");
        let (client, connection) = tokio_postgres::connect(&url, NoTls)
            .await
            .expect("connect to test database");
        let connection_task = tokio::spawn(connection);
        let cancel = client.cancel_token();
        let query =
            tokio::spawn(async move { collect_query(&client, "SELECT pg_sleep(10)").await });

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        cancel.cancel_query(NoTls).await.expect("send cancellation");
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), query)
            .await
            .expect("cancelled query returns promptly")
            .expect("query task completes");
        assert!(result.is_err(), "cancelled query should fail");
        connection_task.abort();
    }
}
