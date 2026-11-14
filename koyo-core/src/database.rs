use crate::{
    common::{
        QUERY_ROW_LIMIT, TABLE_PAGE_LIMIT, TableMeta, ensure_filter_operator, ensure_one_row,
        ensure_writable, find_column, quote_identifier,
    },
    models::{
        ApplySummary, ColumnCatalogEntry, ColumnInfo, ConnectionInfo, ConnectionProfile, Engine,
        FilterKind, FilterOperator, ForeignKeyInfo, GridChange, GridRow, MetadataGroup,
        MetadataItem, QueryOutput, QueryResultSet, SchemaObject, SchemaObjectKind, TableFilter,
        TablePage, TableSort, TableSummary,
    },
    mssql, mysql, oracle,
    sqlite::{self, SqliteSession},
};
use futures_util::TryStreamExt;
use rustls_tokio_postgres::{MakeRustlsConnect, config_platform_verifier};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_postgres::{CancelToken, Client, SimpleQueryMessage, config::SslMode, types::ToSql};
use uuid::Uuid;

const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<Uuid, Arc<SessionEntry>>>,
}

struct SessionEntry {
    session: Mutex<Option<DbSession>>,
    cancel: Mutex<Option<CancelState>>,
    query_active: AtomicBool,
}

#[derive(Clone)]
enum CancelState {
    Postgres {
        token: CancelToken,
        ssl_mode: crate::models::SslMode,
    },
    Sqlite {
        interrupt: Arc<rusqlite::InterruptHandle>,
    },
    Mysql {
        opts: mysql_async::Opts,
        connection_id: u32,
    },
    Mssql {
        config: tiberius::Config,
        spid: i16,
    },
    Oracle,
}

enum DbSession {
    Postgres(Client),
    Sqlite(SqliteSession),
    Mysql(mysql::MysqlSession),
    Mssql(mssql::MssqlSession),
    Oracle(oracle::OracleSession),
}

struct QueryGuard<'a>(&'a AtomicBool);

impl<'a> QueryGuard<'a> {
    fn begin(active: &'a AtomicBool) -> Self {
        active.store(true, Ordering::SeqCst);
        Self(active)
    }
}

impl Drop for QueryGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

async fn get_session(
    manager: &SessionManager,
    session_id: Uuid,
) -> Result<Arc<SessionEntry>, String> {
    manager
        .sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "Session is no longer connected.".to_string())
}

pub async fn connect(
    manager: &SessionManager,
    profile: &ConnectionProfile,
    password: &str,
) -> Result<ConnectionInfo, String> {
    let profile = profile.clone();
    let session_id = Uuid::new_v4();

    match profile.engine {
        Engine::Sqlite => {
            let (session, server_version) = sqlite::connect(&profile.sqlite_path)?;
            let cancel = CancelState::Sqlite {
                interrupt: session.interrupt.clone(),
            };
            manager.sessions.lock().await.insert(
                session_id,
                Arc::new(SessionEntry {
                    session: Mutex::new(Some(DbSession::Sqlite(session))),
                    cancel: Mutex::new(Some(cancel)),
                    query_active: AtomicBool::new(false),
                }),
            );
            return Ok(ConnectionInfo {
                session_id,
                profile,
                server_version,
                current_user: "local".to_string(),
            });
        }
        Engine::Mysql | Engine::Mariadb => {
            let (session, server_version, current_user, opts) =
                mysql::connect(&profile, password).await?;
            let cancel = CancelState::Mysql {
                opts,
                connection_id: session.connection_id,
            };
            manager.sessions.lock().await.insert(
                session_id,
                Arc::new(SessionEntry {
                    session: Mutex::new(Some(DbSession::Mysql(session))),
                    cancel: Mutex::new(Some(cancel)),
                    query_active: AtomicBool::new(false),
                }),
            );
            return Ok(ConnectionInfo {
                session_id,
                profile,
                server_version,
                current_user,
            });
        }
        Engine::SqlServer => {
            let (session, server_version, current_user, config) =
                mssql::connect(&profile, password, profile.engine).await?;
            let cancel = CancelState::Mssql {
                config,
                spid: session.spid,
            };
            manager.sessions.lock().await.insert(
                session_id,
                Arc::new(SessionEntry {
                    session: Mutex::new(Some(DbSession::Mssql(session))),
                    cancel: Mutex::new(Some(cancel)),
                    query_active: AtomicBool::new(false),
                }),
            );
            return Ok(ConnectionInfo {
                session_id,
                profile,
                server_version,
                current_user,
            });
        }
        Engine::Oracle => {
            let (session, server_version, current_user) =
                oracle::connect(&profile, password).await?;
            manager.sessions.lock().await.insert(
                session_id,
                Arc::new(SessionEntry {
                    session: Mutex::new(Some(DbSession::Oracle(session))),
                    cancel: Mutex::new(Some(CancelState::Oracle)),
                    query_active: AtomicBool::new(false),
                }),
            );
            return Ok(ConnectionInfo {
                session_id,
                profile,
                server_version,
                current_user,
            });
        }
        Engine::Postgres => {}
    }

    let mut config = tokio_postgres::Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .user(&profile.username)
        .password(password)
        .dbname(&profile.database)
        .application_name("Koyo");
    config.ssl_mode(match profile.ssl_mode {
        crate::models::SslMode::VerifyFull => SslMode::Require,
        crate::models::SslMode::Disable => SslMode::Disable,
    });

    let client = match profile.ssl_mode {
        crate::models::SslMode::VerifyFull => {
            let tls = config_platform_verifier().map_err(|error| {
                format!("Could not initialize system TLS verification: {error}")
            })?;
            let (client, connection) = config
                .connect(MakeRustlsConnect::new(tls))
                .await
                .map_err(|error| format!("PostgreSQL connection failed: {error}"))?;
            tokio::spawn(async move {
                if let Err(error) = connection.await {
                    eprintln!("PostgreSQL connection closed: {error}");
                }
            });
            client
        }
        crate::models::SslMode::Disable => {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|error| format!("PostgreSQL connection failed: {error}"))?;
            tokio::spawn(async move {
                if let Err(error) = connection.await {
                    eprintln!("PostgreSQL connection closed: {error}");
                }
            });
            client
        }
    };

    let row = client
        .query_one(
            "SELECT current_user, current_setting('server_version')",
            &[],
        )
        .await
        .map_err(|error| format!("Could not read PostgreSQL server details: {error}"))?;
    let current_user: String = row
        .try_get(0)
        .map_err(|error| format!("Could not read current PostgreSQL user: {error}"))?;
    let server_version: String = row
        .try_get(1)
        .map_err(|error| format!("Could not read PostgreSQL server version: {error}"))?;

    let cancel = CancelState::Postgres {
        token: client.cancel_token(),
        ssl_mode: profile.ssl_mode,
    };
    manager.sessions.lock().await.insert(
        session_id,
        Arc::new(SessionEntry {
            session: Mutex::new(Some(DbSession::Postgres(client))),
            cancel: Mutex::new(Some(cancel)),
            query_active: AtomicBool::new(false),
        }),
    );
    Ok(ConnectionInfo {
        session_id,
        profile,
        server_version,
        current_user,
    })
}

pub async fn disconnect(manager: &SessionManager, session_id: Uuid) -> Result<(), String> {
    let entry = get_session(manager, session_id).await?;
    if entry.query_active.load(Ordering::SeqCst) {
        if let Some(cancel) = entry.cancel.lock().await.clone() {
            let _ = send_cancel(&cancel).await;
        }
    }
    let cleanup_entry = Arc::clone(&entry);
    let mut session = match tokio::time::timeout(DISCONNECT_TIMEOUT, entry.session.lock()).await {
        Ok(session) => session,
        Err(_) => {
            // The operation still holds the session. Drop every credential
            // and the map entry now; a detached waiter clears the session as
            // soon as the operation releases it, so the id cannot leak.
            *entry.cancel.lock().await = None;
            entry.query_active.store(false, Ordering::SeqCst);
            manager.sessions.lock().await.remove(&session_id);
            tokio::spawn(async move {
                let mut session = cleanup_entry.session.lock().await;
                *session = None;
            });
            return Err(
                "A database operation is still running. Cancel it and try again.".to_string(),
            );
        }
    };
    *session = None;
    *entry.cancel.lock().await = None;
    entry.query_active.store(false, Ordering::SeqCst);
    manager.sessions.lock().await.remove(&session_id);
    Ok(())
}

pub async fn cancel_query(manager: &SessionManager, session_id: Uuid) -> Result<(), String> {
    let entry = get_session(manager, session_id).await?;
    if !entry.query_active.load(Ordering::SeqCst) {
        return Err("No query is running.".into());
    }
    let cancel = entry
        .cancel
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    send_cancel(&cancel).await
}

async fn send_cancel(cancel: &CancelState) -> Result<(), String> {
    match cancel {
        CancelState::Postgres { token, ssl_mode } => match ssl_mode {
            crate::models::SslMode::VerifyFull => {
                let tls = config_platform_verifier().map_err(|error| {
                    format!("Could not initialize system TLS verification: {error}")
                })?;
                token
                    .cancel_query(MakeRustlsConnect::new(tls))
                    .await
                    .map_err(|error| error.to_string())
            }
            crate::models::SslMode::Disable => token
                .cancel_query(tokio_postgres::NoTls)
                .await
                .map_err(|error| error.to_string()),
        },
        CancelState::Sqlite { interrupt } => {
            interrupt.interrupt();
            Ok(())
        }
        CancelState::Mysql {
            opts,
            connection_id,
        } => mysql::cancel_query(opts, *connection_id).await,
        CancelState::Mssql { config, spid } => mssql::cancel_query(config, *spid).await,
        CancelState::Oracle => oracle::cancel_query(),
    }
}

pub async fn list_tables(
    manager: &SessionManager,
    session_id: Uuid,
) -> Result<Vec<TableSummary>, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    let client = match session {
        DbSession::Postgres(client) => client,
        DbSession::Sqlite(sqlite_session) => return sqlite::list_tables(sqlite_session).await,
        DbSession::Mysql(mysql_session) => return mysql::list_tables(mysql_session).await,
        DbSession::Mssql(mssql_session) => return mssql::list_tables(mssql_session).await,
        DbSession::Oracle(oracle_session) => return oracle::list_tables(oracle_session).await,
    };
    let rows = client
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

pub async fn list_schema_objects(
    manager: &SessionManager,
    session_id: Uuid,
) -> Result<Vec<SchemaObject>, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    match session {
        DbSession::Postgres(client) => list_schema_objects_pg(client).await,
        DbSession::Sqlite(sqlite_session) => sqlite::list_schema_objects(sqlite_session).await,
        DbSession::Mysql(mysql_session) => mysql::list_schema_objects(mysql_session).await,
        DbSession::Mssql(mssql_session) => mssql::list_schema_objects(mssql_session).await,
        DbSession::Oracle(oracle_session) => oracle::list_schema_objects(oracle_session).await,
    }
}

pub async fn list_column_catalog(
    manager: &SessionManager,
    session_id: Uuid,
) -> Result<Vec<ColumnCatalogEntry>, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    match session {
        DbSession::Postgres(client) => list_columns_pg(client).await,
        DbSession::Sqlite(sqlite_session) => sqlite::list_columns(sqlite_session).await,
        DbSession::Mysql(mysql_session) => mysql::list_columns(mysql_session).await,
        DbSession::Mssql(mssql_session) => mssql::list_columns(mssql_session).await,
        DbSession::Oracle(oracle_session) => oracle::list_columns(oracle_session).await,
    }
}

pub async fn list_foreign_keys(
    manager: &SessionManager,
    session_id: Uuid,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    match session {
        DbSession::Postgres(client) => list_foreign_keys_pg(client, schema, table).await,
        DbSession::Sqlite(sqlite_session) => {
            sqlite::list_foreign_keys(sqlite_session, schema, table).await
        }
        DbSession::Mysql(mysql_session) => {
            mysql::list_foreign_keys(mysql_session, schema, table).await
        }
        DbSession::Mssql(mssql_session) => {
            mssql::list_foreign_keys(mssql_session, schema, table).await
        }
        DbSession::Oracle(oracle_session) => {
            oracle::list_foreign_keys(oracle_session, schema, table).await
        }
    }
}

async fn list_columns_pg(client: &Client) -> Result<Vec<ColumnCatalogEntry>, String> {
    let rows = client
        .query(
            "SELECT n.nspname::text, c.relname::text, a.attname::text
             FROM pg_catalog.pg_attribute a
             JOIN pg_catalog.pg_class c ON a.attrelid = c.oid
             JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid
             WHERE a.attnum > 0
               AND NOT a.attisdropped
               AND c.relkind IN ('r', 'p', 'v', 'm')
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg_toast%'
             ORDER BY 1, 2, a.attnum",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| ColumnCatalogEntry {
            schema: row.get(0),
            table: row.get(1),
            column: row.get(2),
        })
        .collect())
}

async fn list_schema_objects_pg(client: &Client) -> Result<Vec<SchemaObject>, String> {
    let rows = client
        .query(
            "SELECT n.nspname::text, c.relname::text,
                    CASE c.relkind WHEN 'v' THEN 'view' WHEN 'm' THEN 'view' ELSE 'table' END
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE c.relkind IN ('r', 'p', 'v', 'm')
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg_toast%'
             UNION ALL
             SELECT DISTINCT n.nspname::text, p.proname::text, 'function'
             FROM pg_catalog.pg_proc p
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE p.prokind <> 'p'
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg_toast%'
             UNION ALL
             SELECT DISTINCT n.nspname::text, t.tgname::text, 'trigger'
             FROM pg_catalog.pg_trigger t
             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE NOT t.tgisinternal
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg_toast%'
             ORDER BY 1, 2, 3",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            let kind = row
                .try_get::<_, String>(2)
                .map_err(|error| error.to_string())?;
            Ok(SchemaObject {
                schema: row.try_get(0).map_err(|error| error.to_string())?,
                name: row.try_get(1).map_err(|error| error.to_string())?,
                kind: schema_object_kind(&kind)?,
            })
        })
        .collect()
}

async fn list_foreign_keys_pg(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let rows = client
        .query(
            "SELECT sn.nspname::text, sc.relname::text, sa.attname::text,
                    tn.nspname::text, tc.relname::text, ta.attname::text,
                    con.conname::text
             FROM pg_catalog.pg_constraint con
             JOIN pg_catalog.pg_class sc ON sc.oid = con.conrelid
             JOIN pg_catalog.pg_namespace sn ON sn.oid = sc.relnamespace
             JOIN pg_catalog.pg_class tc ON tc.oid = con.confrelid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = tc.relnamespace
             JOIN LATERAL unnest(con.conkey, con.confkey)
                  WITH ORDINALITY AS key(src_attnum, target_attnum, position) ON true
             JOIN pg_catalog.pg_attribute sa
                  ON sa.attrelid = sc.oid AND sa.attnum = key.src_attnum
             JOIN pg_catalog.pg_attribute ta
                  ON ta.attrelid = tc.oid AND ta.attnum = key.target_attnum
             WHERE con.contype = 'f'
               AND ((sn.nspname = $1 AND sc.relname = $2)
                    OR (tn.nspname = $1 AND tc.relname = $2))
             ORDER BY sn.nspname, sc.relname, con.conname, key.position",
            &[&schema, &table],
        )
        .await
        .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            Ok(ForeignKeyInfo {
                schema: row.try_get(0).map_err(|error| error.to_string())?,
                table: row.try_get(1).map_err(|error| error.to_string())?,
                column: row.try_get(2).map_err(|error| error.to_string())?,
                referenced_schema: row.try_get(3).map_err(|error| error.to_string())?,
                referenced_table: row.try_get(4).map_err(|error| error.to_string())?,
                referenced_column: row.try_get(5).map_err(|error| error.to_string())?,
                constraint_name: row.try_get(6).map_err(|error| error.to_string())?,
            })
        })
        .collect()
}

fn schema_object_kind(kind: &str) -> Result<SchemaObjectKind, String> {
    match kind {
        "table" => Ok(SchemaObjectKind::Table),
        "view" => Ok(SchemaObjectKind::View),
        "function" => Ok(SchemaObjectKind::Function),
        "trigger" => Ok(SchemaObjectKind::Trigger),
        other => Err(format!("Unknown schema object kind: {other}")),
    }
}

pub async fn list_database_metadata(
    manager: &SessionManager,
    session_id: Uuid,
) -> Result<Vec<MetadataGroup>, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    match session {
        DbSession::Postgres(client) => list_database_metadata_pg(client).await,
        DbSession::Sqlite(sqlite_session) => sqlite::list_database_metadata(sqlite_session).await,
        DbSession::Mysql(mysql_session) => mysql::list_database_metadata(mysql_session).await,
        DbSession::Mssql(mssql_session) => mssql::list_database_metadata(mssql_session).await,
        DbSession::Oracle(oracle_session) => oracle::list_database_metadata(oracle_session).await,
    }
}

fn metadata_group(name: &str, items: Vec<MetadataItem>) -> Option<MetadataGroup> {
    (!items.is_empty()).then(|| MetadataGroup {
        name: name.to_string(),
        items,
    })
}

async fn list_database_metadata_pg(client: &Client) -> Result<Vec<MetadataGroup>, String> {
    let mut groups = Vec::new();

    let rows = client
        .query(
            "SELECT evtname::text, evtevent::text, evtenabled::text
             FROM pg_catalog.pg_event_trigger
             ORDER BY evtname",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            let event: String = row.try_get(1).map_err(|error| error.to_string())?;
            let enabled: String = row.try_get(2).map_err(|error| error.to_string())?;
            Ok(MetadataItem {
                name: row.try_get(0).map_err(|error| error.to_string())?,
                detail: format!("{event} ({enabled})"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    groups.extend(metadata_group("Event Triggers", items));

    let rows = client
        .query(
            "SELECT e.extname::text, e.extversion::text, n.nspname::text
             FROM pg_catalog.pg_extension e
             JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
             ORDER BY e.extname",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            let version: String = row.try_get(1).map_err(|error| error.to_string())?;
            let schema: String = row.try_get(2).map_err(|error| error.to_string())?;
            Ok(MetadataItem {
                name: row.try_get(0).map_err(|error| error.to_string())?,
                detail: format!("{version}, schema {schema}"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    groups.extend(metadata_group("Extensions", items));

    let rows = client
        .query(
            "SELECT spcname::text,
                    COALESCE(NULLIF(pg_tablespace_location(oid), ''), 'default location')
             FROM pg_catalog.pg_tablespace
             ORDER BY spcname",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            Ok(MetadataItem {
                name: row.try_get(0).map_err(|error| error.to_string())?,
                detail: row.try_get(1).map_err(|error| error.to_string())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    groups.extend(metadata_group("Storage", items));

    let row = client
        .query_one(
            "SELECT version(), current_database(), current_user,
                    pg_encoding_to_char(d.encoding), d.datcollate
             FROM pg_catalog.pg_database d
             WHERE d.datname = current_database()",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let version: String = row.try_get(0).map_err(|error| error.to_string())?;
    let database: String = row.try_get(1).map_err(|error| error.to_string())?;
    let user: String = row.try_get(2).map_err(|error| error.to_string())?;
    let encoding: String = row.try_get(3).map_err(|error| error.to_string())?;
    let collation: String = row.try_get(4).map_err(|error| error.to_string())?;
    groups.extend(metadata_group(
        "System Info",
        vec![
            MetadataItem {
                name: "Version".into(),
                detail: version,
            },
            MetadataItem {
                name: "Database".into(),
                detail: database,
            },
            MetadataItem {
                name: "Current User".into(),
                detail: user,
            },
            MetadataItem {
                name: "Encoding".into(),
                detail: format!("{encoding}, collation {collation}"),
            },
        ],
    ));

    let rows = client
        .query(
            "SELECT rolname::text, rolsuper, rolcanlogin, rolcreaterole, rolcreatedb
             FROM pg_catalog.pg_roles
             ORDER BY rolname
             LIMIT 200",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    let items = rows
        .into_iter()
        .map(|row| {
            let mut attributes = Vec::new();
            if row.try_get::<_, bool>(1).unwrap_or(false) {
                attributes.push("superuser");
            }
            if row.try_get::<_, bool>(2).unwrap_or(false) {
                attributes.push("login");
            }
            if row.try_get::<_, bool>(3).unwrap_or(false) {
                attributes.push("create role");
            }
            if row.try_get::<_, bool>(4).unwrap_or(false) {
                attributes.push("create db");
            }
            Ok(MetadataItem {
                name: row.try_get(0).map_err(|error| error.to_string())?,
                detail: if attributes.is_empty() {
                    "no attributes".to_string()
                } else {
                    attributes.join(", ")
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    groups.extend(metadata_group("Roles", items));

    Ok(groups)
}

pub async fn load_table_page(
    manager: &SessionManager,
    session_id: Uuid,
    schema: String,
    table: String,
    offset: u32,
    limit: u32,
    filters: Vec<TableFilter>,
    sort: Option<TableSort>,
) -> Result<TablePage, String> {
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    let client = match session {
        DbSession::Postgres(client) => client,
        DbSession::Sqlite(sqlite_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return sqlite::load_page(
                sqlite_session,
                &schema,
                &table,
                offset,
                limit,
                &filters,
                sort.as_ref(),
            )
            .await;
        }
        DbSession::Mysql(mysql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mysql::load_page(
                mysql_session,
                &schema,
                &table,
                offset,
                limit,
                &filters,
                sort.as_ref(),
            )
            .await;
        }
        DbSession::Mssql(mssql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mssql::load_page(
                mssql_session,
                &schema,
                &table,
                offset,
                limit,
                &filters,
                sort.as_ref(),
            )
            .await;
        }
        DbSession::Oracle(oracle_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return oracle::load_page(
                oracle_session,
                &schema,
                &table,
                offset,
                limit,
                &filters,
                sort.as_ref(),
            )
            .await;
        }
    };
    let meta = load_table_meta(client, &schema, &table).await?;
    if meta.columns.is_empty() {
        return Err("Table has no visible columns.".into());
    }

    let limit = limit.clamp(1, TABLE_PAGE_LIMIT);
    let fetch_limit = limit + 1;
    let (sql, params) = build_table_page_query(
        &schema,
        &table,
        &meta,
        offset,
        fetch_limit,
        &filters,
        sort.as_ref(),
    )?;

    let rows = {
        let _active = QueryGuard::begin(&entry.query_active);
        client
            .query(&sql, &sql_params(&params))
            .await
            .map_err(|error| error.to_string())?
    };
    let mut page_rows = Vec::with_capacity(fetch_limit as usize);
    for row in rows {
        let version: String = row.try_get(0).map_err(|error| error.to_string())?;
        let values = (0..meta.columns.len())
            .map(|index| {
                row.try_get::<_, Option<String>>(index + 1)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let key = meta
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.primary_key)
            .map(|(index, _)| values[index].clone())
            .collect();
        page_rows.push(GridRow {
            version,
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

fn build_table_page_query(
    schema: &str,
    table: &str,
    meta: &TableMeta,
    offset: u32,
    limit: u32,
    filters: &[TableFilter],
    sort: Option<&TableSort>,
) -> Result<(String, Vec<Option<String>>), String> {
    let selected = meta
        .columns
        .iter()
        .map(|column| format!("{}::text", quote_identifier(&column.name)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::new();
    let predicate = build_filter_predicate(meta, filters, &mut params)?;
    let order = build_order_by(meta, sort)?;
    let mut sql = format!(
        "SELECT xmin::text, {selected} FROM {}",
        quote_relation(schema, table)
    );
    if let Some(predicate) = predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate);
    }
    sql.push_str(&format!(" ORDER BY {order} LIMIT {limit} OFFSET {offset}"));
    Ok((sql, params))
}

fn build_filter_predicate(
    meta: &TableMeta,
    filters: &[TableFilter],
    params: &mut Vec<Option<String>>,
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
                params.push(Some(value.to_string()));
                format!(
                    "strpos(lower({identifier}::text), lower(${})) > 0",
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
                format!(
                    "{identifier} {operator} {}",
                    cast_placeholder(params.len(), column)
                )
            }
            FilterOperator::IsNull => format!("{identifier} IS NULL"),
            FilterOperator::IsNotNull => format!("{identifier} IS NOT NULL"),
            FilterOperator::IsTrue => format!("{identifier} IS TRUE"),
            FilterOperator::IsFalse => format!("{identifier} IS FALSE"),
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
    for column in meta.primary_columns() {
        if sort_column.is_some_and(|sort| sort.name == column.name) {
            continue;
        }
        terms.push(format!("{} ASC", quote_identifier(&column.name)));
    }
    if meta.primary_columns().next().is_none() {
        if meta.relation_kind.as_deref() == Some("p") {
            terms.push("tableoid".to_string());
        }
        terms.push("ctid".to_string());
    }
    Ok(terms.join(", "))
}

pub async fn apply_grid_changes(
    manager: &SessionManager,
    session_id: Uuid,
    schema: String,
    table: String,
    changes: Vec<GridChange>,
) -> Result<ApplySummary, String> {
    if changes.is_empty() {
        return Ok(ApplySummary { applied: 0 });
    }
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    let client = match session {
        DbSession::Postgres(client) => client,
        DbSession::Sqlite(sqlite_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return sqlite::apply_changes(sqlite_session, &schema, &table, changes).await;
        }
        DbSession::Mysql(mysql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mysql::apply_changes(mysql_session, &schema, &table, changes).await;
        }
        DbSession::Mssql(mssql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mssql::apply_changes(mssql_session, &schema, &table, changes).await;
        }
        DbSession::Oracle(oracle_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return oracle::apply_changes(oracle_session, &schema, &table, changes).await;
        }
    };
    let meta = load_table_meta(client, &schema, &table).await?;
    if !meta.editable() {
        return Err("Grid editing requires a table with a primary key.".into());
    }

    let _active = QueryGuard::begin(&entry.query_active);
    let transaction = client
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
                ..
            } => {
                let (sql, params) = build_update(&schema, &table, &meta, key, version, values)?;
                let refs = sql_params(&params);
                let count = transaction
                    .execute(&sql, &refs)
                    .await
                    .map_err(|error| error.to_string())?;
                ensure_one_row(count)?;
            }
            GridChange::Delete { key, version, .. } => {
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

pub async fn run_sql(
    manager: &SessionManager,
    session_id: Uuid,
    sql: String,
) -> Result<QueryOutput, String> {
    if sql.trim().is_empty() {
        return Err("Enter SQL to run.".into());
    }
    let entry = get_session(manager, session_id).await?;
    let mut session_guard = entry.session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "Session is no longer connected.".to_string())?;
    let client = match session {
        DbSession::Postgres(client) => client,
        DbSession::Sqlite(sqlite_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return sqlite::run_sql(sqlite_session, &sql).await;
        }
        DbSession::Mysql(mysql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mysql::run_sql(mysql_session, &sql).await;
        }
        DbSession::Mssql(mssql_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return mssql::run_sql(mssql_session, &sql).await;
        }
        DbSession::Oracle(oracle_session) => {
            let _active = QueryGuard::begin(&entry.query_active);
            return oracle::run_sql(oracle_session, &sql).await;
        }
    };
    let _active = QueryGuard::begin(&entry.query_active);
    collect_query(client, &sql).await
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
                    pg_catalog.format_type(a.atttypid, NULL),
                    CASE COALESCE(base.typcategory, ty.typcategory)
                        WHEN 'S' THEN 'text'
                        WHEN 'N' THEN 'number'
                        WHEN 'D' THEN 'temporal'
                        WHEN 'T' THEN 'temporal'
                        WHEN 'B' THEN 'boolean'
                        ELSE 'other'
                    END,
                    EXISTS (
                        SELECT 1
                        FROM pg_catalog.pg_opclass oc
                        JOIN pg_catalog.pg_am am ON am.oid = oc.opcmethod
                        WHERE am.amname = 'btree'
                          AND oc.opcdefault
                          AND oc.opcintype = COALESCE(base.oid, ty.oid)
                    ),
                    NOT a.attnotnull, a.attgenerated <> '', a.attidentity <> '',
                    COALESCE(i.indisprimary AND a.attnum = ANY(i.indkey), false)
             FROM pg_catalog.pg_class t
             JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
             JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid
             JOIN pg_catalog.pg_type ty ON ty.oid = a.atttypid
             LEFT JOIN pg_catalog.pg_type base ON base.oid = ty.typbasetype
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
        let filter_kind = match row
            .try_get::<_, String>(4)
            .map_err(|error| error.to_string())?
            .as_str()
        {
            "text" => FilterKind::Text,
            "number" => FilterKind::Number,
            "temporal" => FilterKind::Temporal,
            "boolean" => FilterKind::Boolean,
            _ => FilterKind::Other,
        };
        let data_type: String = row.try_get(2).map_err(|error| error.to_string())?;
        let filter_type: String = row.try_get(3).map_err(|error| error.to_string())?;
        if !valid_type_name(&data_type) || !valid_type_name(&filter_type) {
            return Err("PostgreSQL returned an invalid column type name.".into());
        }
        columns.push(ColumnInfo {
            name: row.try_get(1).map_err(|error| error.to_string())?,
            data_type,
            filter_type,
            filter_kind,
            sortable: row.try_get(5).map_err(|error| error.to_string())?,
            nullable: row.try_get(6).map_err(|error| error.to_string())?,
            primary_key: row.try_get(9).map_err(|error| error.to_string())?,
            generated: row.try_get(7).map_err(|error| error.to_string())?,
            identity: row.try_get(8).map_err(|error| error.to_string())?,
        });
    }
    Ok(TableMeta::new(columns, Some(relation_kind)))
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
        let column = find_column(&meta.columns, &name)?;
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
        let column = find_column(&meta.columns, &name)?;
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

fn cast_placeholder(index: usize, column: &ColumnInfo) -> String {
    format!("${index}::text::{}", column.filter_type)
}

fn valid_type_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut quoted = false;
    for character in name.chars() {
        if character == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted
            || character.is_ascii_alphanumeric()
            || matches!(
                character,
                '_' | '$' | ' ' | '(' | ')' | ',' | '.' | '[' | ']'
            )
        {
            continue;
        }
        return false;
    }
    !quoted
}

fn sql_params(values: &[Option<String>]) -> Vec<&(dyn ToSql + Sync)> {
    values
        .iter()
        .map(|value| value as &(dyn ToSql + Sync))
        .collect()
}

fn quote_relation(schema: &str, table: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(table))
}

#[cfg(test)]
mod tests {
    use super::{
        ColumnInfo, FilterKind, FilterOperator, TableFilter, TableMeta, TableSort,
        build_table_page_query, quote_identifier, valid_type_name,
    };

    fn column(
        name: &str,
        data_type: &str,
        filter_kind: FilterKind,
        primary_key: bool,
    ) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            filter_type: data_type.to_string(),
            filter_kind,
            sortable: true,
            nullable: true,
            primary_key,
            generated: false,
            identity: false,
        }
    }

    fn table_meta(columns: Vec<ColumnInfo>, relation_kind: &str) -> TableMeta {
        TableMeta::new(columns, Some(relation_kind.to_string()))
    }

    #[test]
    fn quotes_embedded_identifier_quotes() {
        assert_eq!(quote_identifier("odd\"table"), "\"odd\"\"table\"");
    }

    #[test]
    fn validates_catalog_type_names() {
        assert!(valid_type_name("timestamp with time zone"));
        assert!(valid_type_name("character varying(32)"));
        assert!(valid_type_name("public.\"odd type\"[]"));
        assert!(valid_type_name("public.\"order-status\""));
        assert!(valid_type_name("public.\"price$\""));
        assert!(!valid_type_name("text); SELECT pg_sleep(1); --"));
        assert!(!valid_type_name("\"unterminated"));
        assert!(!valid_type_name(""));
    }

    #[test]
    fn builds_parameterized_filters_and_sorted_order() {
        let meta = table_meta(
            vec![
                column("id", "integer", FilterKind::Number, true),
                column("name", "text", FilterKind::Text, false),
                column("active", "boolean", FilterKind::Boolean, false),
            ],
            "r",
        );
        let filters = vec![
            TableFilter {
                column: "name".into(),
                operator: FilterOperator::Contains,
                value: Some("a_b%".into()),
            },
            TableFilter {
                column: "id".into(),
                operator: FilterOperator::GreaterThan,
                value: Some("2".into()),
            },
            TableFilter {
                column: "active".into(),
                operator: FilterOperator::IsTrue,
                value: None,
            },
        ];
        let sort = TableSort {
            column: "name".into(),
            descending: true,
        };
        let (sql, params) =
            build_table_page_query("public", "items", &meta, 10, 101, &filters, Some(&sort))
                .expect("build page query");

        assert!(sql.contains("strpos(lower(\"name\"::text), lower($1)) > 0"));
        assert!(sql.contains("\"id\" > $2::text::integer"));
        assert!(sql.contains("\"active\" IS TRUE"));
        assert!(sql.contains("ORDER BY \"name\" DESC, \"id\" ASC"));
        assert!(sql.contains("LIMIT 101 OFFSET 10"));
        assert_eq!(params, vec![Some("a_b%".into()), Some("2".into())]);
    }

    #[test]
    fn casts_use_type_without_typmod() {
        use super::build_update;
        use std::collections::BTreeMap;

        let mut code = column("code", "character varying(3)", FilterKind::Text, true);
        code.filter_type = "character varying".into();
        let meta = table_meta(vec![code], "r");
        let filter = TableFilter {
            column: "code".into(),
            operator: FilterOperator::Equals,
            value: Some("abcdef".into()),
        };
        let (sql, _) = build_table_page_query("public", "items", &meta, 0, 101, &[filter], None)
            .expect("build filter query");
        assert!(sql.contains("\"code\" = $1::text::character varying"));
        assert!(!sql.contains("character varying(3)"));

        let values = BTreeMap::from([("code".to_string(), Some("abcdef".to_string()))]);
        let (update_sql, _) = build_update(
            "public",
            "items",
            &meta,
            vec![Some("a".into())],
            "7".into(),
            values,
        )
        .expect("build update");
        assert!(update_sql.contains("\"code\" = $1::text::character varying"));
        assert!(!update_sql.contains("character varying(3)"));
    }

    #[test]
    fn rejects_sort_on_column_without_btree_order() {
        let mut payload = column("payload", "json", FilterKind::Other, false);
        payload.sortable = false;
        let meta = table_meta(vec![payload], "r");
        let sort = TableSort {
            column: "payload".into(),
            descending: false,
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[], Some(&sort)).is_err()
        );
    }

    #[test]
    fn keyless_tables_use_ctid_for_stable_paging() {
        let meta = table_meta(vec![column("label", "text", FilterKind::Text, false)], "r");
        let (default_sql, _) = build_table_page_query("public", "items", &meta, 0, 101, &[], None)
            .expect("build default query");
        assert!(default_sql.contains("ORDER BY ctid"));

        let sort = TableSort {
            column: "label".into(),
            descending: false,
        };
        let (sorted_sql, _) =
            build_table_page_query("public", "items", &meta, 0, 101, &[], Some(&sort))
                .expect("build sorted query");
        assert!(sorted_sql.contains("ORDER BY \"label\" ASC, ctid"));

        let partitioned = table_meta(vec![column("label", "text", FilterKind::Text, false)], "p");
        let (partitioned_sql, _) =
            build_table_page_query("public", "items", &partitioned, 0, 101, &[], None)
                .expect("build partitioned query");
        assert!(partitioned_sql.contains("ORDER BY tableoid, ctid"));
    }

    #[test]
    fn rejects_operator_that_does_not_fit_column_type() {
        let meta = table_meta(vec![column("id", "integer", FilterKind::Number, true)], "r");
        let filter = TableFilter {
            column: "id".into(),
            operator: FilterOperator::Contains,
            value: Some("1".into()),
        };
        assert!(build_table_page_query("public", "items", &meta, 0, 101, &[filter], None).is_err());
    }

    #[test]
    fn rejects_missing_values_and_unknown_columns() {
        let meta = table_meta(vec![column("name", "text", FilterKind::Text, false)], "r");
        let missing_value = TableFilter {
            column: "name".into(),
            operator: FilterOperator::Equals,
            value: Some(String::new()),
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[missing_value], None)
                .is_err()
        );

        let unknown_column = TableFilter {
            column: "missing".into(),
            operator: FilterOperator::IsNull,
            value: None,
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[unknown_column], None)
                .is_err()
        );

        let unknown_sort = TableSort {
            column: "missing".into(),
            descending: false,
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[], Some(&unknown_sort))
                .is_err()
        );
    }

    #[test]
    fn other_columns_only_accept_null_filters() {
        let meta = table_meta(
            vec![column("payload", "jsonb", FilterKind::Other, false)],
            "r",
        );
        let null_filter = TableFilter {
            column: "payload".into(),
            operator: FilterOperator::IsNull,
            value: None,
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[null_filter], None).is_ok()
        );

        let equals_filter = TableFilter {
            column: "payload".into(),
            operator: FilterOperator::Equals,
            value: Some("{}".into()),
        };
        assert!(
            build_table_page_query("public", "items", &meta, 0, 101, &[equals_filter], None)
                .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires KOYO_TEST_DATABASE_URL"]
    async fn applies_staged_grid_changes_and_rejects_stale_rows() {
        use super::{
            build_delete, build_insert, build_update, ensure_one_row, list_database_metadata_pg,
            list_foreign_keys_pg, list_schema_objects_pg, load_table_meta, sql_params,
        };
        use crate::models::SchemaObjectKind;
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
        let child = format!("koyo_child_test_{suffix}");

        client
            .batch_execute(&format!(
                "CREATE TABLE public.{table} (
                     id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                     name text NOT NULL,
                     note text,
                     code varchar(3)
                 );
                 CREATE TABLE public.{keyless} (label text);
                 CREATE TABLE public.{child} (
                     id bigint PRIMARY KEY,
                     parent_id bigint REFERENCES public.{table}(id)
                 );
                 INSERT INTO public.{table} (name, note, code) VALUES ('seed', 'old', 'abc');"
            ))
            .await
            .expect("create fixture tables");

        let objects = list_schema_objects_pg(&client)
            .await
            .expect("list schema objects");
        assert!(objects.iter().any(|object| object.schema == "public"
            && object.name == table
            && object.kind == SchemaObjectKind::Table));

        let keys = list_foreign_keys_pg(&client, "public", &child)
            .await
            .expect("list foreign keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].table, child);
        assert_eq!(keys[0].column, "parent_id");
        assert_eq!(keys[0].referenced_schema, "public");
        assert_eq!(keys[0].referenced_table, table);
        assert_eq!(keys[0].referenced_column, "id");
        assert!(!keys[0].constraint_name.is_empty());

        let inbound = list_foreign_keys_pg(&client, "public", &table)
            .await
            .expect("list foreign keys by target");
        assert!(
            inbound
                .iter()
                .any(|key| key.table == child && key.referenced_table == table)
        );

        let groups = list_database_metadata_pg(&client)
            .await
            .expect("list database metadata");
        assert!(groups.iter().any(|group| group.name == "System Info"));
        assert!(groups.iter().any(|group| group.name == "Roles"));
        assert!(!groups.iter().any(|group| group.name == "Schemas"));
        assert!(groups.iter().all(|group| !group.items.is_empty()));
        assert!(
            groups
                .iter()
                .flat_map(|group| group.items.iter())
                .all(|item| !item.name.is_empty())
        );

        let meta = load_table_meta(&client, "public", &table)
            .await
            .expect("read table metadata");
        assert!(meta.editable());
        assert_eq!(meta.columns[0].filter_kind, FilterKind::Number);
        assert_eq!(meta.columns[1].filter_kind, FilterKind::Text);
        assert!(
            !load_table_meta(&client, "public", &keyless)
                .await
                .expect("read keyless metadata")
                .editable()
        );

        let filters = vec![TableFilter {
            column: "name".into(),
            operator: FilterOperator::Contains,
            value: Some("see".into()),
        }];
        let sort = TableSort {
            column: "name".into(),
            descending: true,
        };
        let (page_sql, page_params) =
            build_table_page_query("public", &table, &meta, 0, 10, &filters, Some(&sort))
                .expect("build filtered page query");
        let page_rows = client
            .query(&page_sql, &sql_params(&page_params))
            .await
            .expect("execute filtered page query");
        assert_eq!(page_rows.len(), 1);
        assert_eq!(
            page_rows[0]
                .try_get::<_, String>(2)
                .expect("read filtered name"),
            "seed"
        );

        let over_length = TableFilter {
            column: "code".into(),
            operator: FilterOperator::Equals,
            value: Some("abcdef".into()),
        };
        let (code_sql, code_params) =
            build_table_page_query("public", &table, &meta, 0, 10, &[over_length], None)
                .expect("build over-length filter query");
        let code_rows = client
            .query(&code_sql, &sql_params(&code_params))
            .await
            .expect("execute over-length filter query");
        assert!(code_rows.is_empty());

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
                "DROP TABLE public.{child}; DROP TABLE public.{table}; DROP TABLE public.{keyless};"
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
