use gpui_kit::*;
use koyo_core::models::{ConnectionProfile, Engine, SchemaObjectKind};
use sqlformat::{Dialect as SqlDialect, FormatOptions, QueryParams};

use super::quote_sql_identifier;
use crate::app::QueryTab;

pub(crate) fn query_tab_title(profile: Option<&ConnectionProfile>) -> SharedString {
    let Some(profile) = profile else {
        return "SQL".into();
    };
    let mut database = profile.database.trim().to_string();
    if database.is_empty() && profile.engine.is_sqlite() {
        database = std::path::Path::new(&profile.sqlite_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("SQLite")
            .to_string();
    }
    if database.is_empty() {
        "SQL".into()
    } else {
        format!("SQL-{database}").into()
    }
}

pub(crate) fn format_sql(engine: Engine, sql: &str) -> String {
    let dialect = match engine {
        Engine::Postgres => SqlDialect::PostgreSql,
        Engine::SqlServer => SqlDialect::SQLServer,
        _ => SqlDialect::Generic,
    };
    let options = FormatOptions {
        uppercase: Some(true),
        dialect,
        ..Default::default()
    };
    sqlformat::format(sql, &QueryParams::None, &options)
        .trim_end()
        .to_string()
}

pub(crate) fn starter_sql(engine: Engine) -> String {
    match engine {
        Engine::Postgres => "SELECT current_database(), current_user;".into(),
        Engine::Sqlite => "SELECT sqlite_version();".into(),
        Engine::Mysql | Engine::Mariadb => "SELECT DATABASE(), CURRENT_USER();".into(),
        Engine::SqlServer => "SELECT DB_NAME(), SUSER_SNAME();".into(),
        Engine::Oracle => "SELECT SYS_CONTEXT('USERENV', 'DB_NAME'), USER FROM dual;".into(),
    }
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// SQL that opens a function or trigger definition in a query tab. Views and
/// tables stay on the data grid path.
pub(crate) fn object_definition_sql(
    engine: Engine,
    kind: SchemaObjectKind,
    schema: &str,
    name: &str,
) -> String {
    let schema_literal = sql_literal(schema);
    let name_literal = sql_literal(name);
    match (engine, kind) {
        (Engine::Postgres, SchemaObjectKind::Function) => format!(
            "SELECT pg_get_functiondef(p.oid) AS definition\nFROM pg_proc p\nJOIN pg_namespace n ON n.oid = p.pronamespace\nWHERE n.nspname = {schema_literal}\n  AND p.proname = {name_literal}\nORDER BY p.oid;"
        ),
        (Engine::Postgres, SchemaObjectKind::Trigger) => format!(
            "SELECT pg_get_triggerdef(t.oid) AS definition\nFROM pg_trigger t\nJOIN pg_class c ON c.oid = t.tgrelid\nJOIN pg_namespace n ON n.oid = c.relnamespace\nWHERE n.nspname = {schema_literal}\n  AND t.tgname = {name_literal};"
        ),
        (Engine::Mysql | Engine::Mariadb, SchemaObjectKind::Function) => format!(
            "SHOW CREATE FUNCTION {}.{};",
            quote_sql_identifier(engine, schema),
            quote_sql_identifier(engine, name)
        ),
        (Engine::Mysql | Engine::Mariadb, SchemaObjectKind::Trigger) => format!(
            "SHOW CREATE TRIGGER {}.{};",
            quote_sql_identifier(engine, schema),
            quote_sql_identifier(engine, name)
        ),
        (Engine::SqlServer, SchemaObjectKind::Function | SchemaObjectKind::Trigger) => {
            let object = format!(
                "{}.{}",
                quote_sql_identifier(engine, schema),
                quote_sql_identifier(engine, name)
            );
            format!(
                "SELECT OBJECT_DEFINITION(OBJECT_ID(N{})) AS definition;",
                sql_literal(&object)
            )
        }
        (Engine::Oracle, SchemaObjectKind::Function | SchemaObjectKind::Trigger) => format!(
            "SELECT text AS definition\nFROM all_source\nWHERE owner = {schema_literal}\n  AND name = {name_literal}\n  AND type = '{}'\nORDER BY line;",
            if kind == SchemaObjectKind::Function {
                "FUNCTION"
            } else {
                "TRIGGER"
            }
        ),
        (Engine::Sqlite, SchemaObjectKind::Trigger) => format!(
            "SELECT sql AS definition\nFROM {}.sqlite_master\nWHERE type = 'trigger'\n  AND name = {name_literal};",
            quote_sql_identifier(engine, schema)
        ),
        (Engine::Sqlite, SchemaObjectKind::Function) => format!(
            "-- SQLite functions are built in or provided by extensions.\nSELECT name\nFROM pragma_function_list\nWHERE name = {name_literal};"
        ),
        (_, SchemaObjectKind::Table | SchemaObjectKind::View) => String::new(),
    }
}

/// Tab text counts as unsaved when it differs from the starter SQL that
/// connection setup may still replace.
pub(crate) fn query_has_unsaved_sql(query: &QueryTab, cx: &App) -> bool {
    query.editor.read(cx).value() != query.starter
}
