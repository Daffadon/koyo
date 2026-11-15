use gpui_kit::component::input::InputState;
use gpui_kit::*;
use koyo_core::models::{ConnectionProfile, Engine, FilterKind, FilterOperator};

use super::state::{ConnectionDialog, Tab};

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

pub(crate) fn starter_sql(engine: Engine) -> String {
    match engine {
        Engine::Postgres => "SELECT current_database(), current_user;".into(),
        Engine::Sqlite => "SELECT sqlite_version();".into(),
        Engine::Mysql | Engine::Mariadb => "SELECT DATABASE(), CURRENT_USER();".into(),
        Engine::SqlServer => "SELECT DB_NAME(), SUSER_SNAME();".into(),
        Engine::Oracle => "SELECT SYS_CONTEXT('USERENV', 'DB_NAME'), USER FROM dual;".into(),
    }
}

pub(crate) fn engine_label(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "PostgreSQL",
        Engine::Sqlite => "SQLite",
        Engine::Mysql => "MySQL",
        Engine::Mariadb => "MariaDB",
        Engine::SqlServer => "SQL Server",
        Engine::Oracle => "Oracle (beta)",
    }
}

pub(crate) fn filter_needs_value(operator: FilterOperator) -> bool {
    matches!(
        operator,
        FilterOperator::Contains
            | FilterOperator::Equals
            | FilterOperator::NotEquals
            | FilterOperator::LessThan
            | FilterOperator::LessThanOrEqual
            | FilterOperator::GreaterThan
            | FilterOperator::GreaterThanOrEqual
    )
}

pub(crate) fn default_operator(kind: FilterKind) -> FilterOperator {
    match kind {
        FilterKind::Text => FilterOperator::Contains,
        FilterKind::Number | FilterKind::Temporal => FilterOperator::Equals,
        FilterKind::Boolean => FilterOperator::IsTrue,
        FilterKind::Other => FilterOperator::IsNull,
    }
}

pub(crate) fn operator_options(kind: FilterKind) -> Vec<(FilterOperator, &'static str)> {
    match kind {
        FilterKind::Text => vec![
            (FilterOperator::Contains, "contains"),
            (FilterOperator::Equals, "equals"),
            (FilterOperator::NotEquals, "not equals"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Number | FilterKind::Temporal => vec![
            (FilterOperator::Equals, "equals"),
            (FilterOperator::NotEquals, "not equals"),
            (FilterOperator::LessThan, "less than"),
            (FilterOperator::LessThanOrEqual, "at most"),
            (FilterOperator::GreaterThan, "greater than"),
            (FilterOperator::GreaterThanOrEqual, "at least"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Boolean => vec![
            (FilterOperator::IsTrue, "is true"),
            (FilterOperator::IsFalse, "is false"),
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
        FilterKind::Other => vec![
            (FilterOperator::IsNull, "is null"),
            (FilterOperator::IsNotNull, "is not null"),
        ],
    }
}

pub(crate) fn set_input(
    input: &Entity<InputState>,
    value: &str,
    window: &mut Window,
    cx: &mut App,
) {
    input.update(cx, |input, cx| {
        input.set_value(value.to_string(), window, cx);
    });
}

fn set_placeholder(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| {
        input.set_placeholder(value, window, cx);
    });
}

pub(crate) fn apply_defaults(
    dialog: &mut ConnectionDialog,
    engine: Engine,
    window: &mut Window,
    cx: &mut App,
) {
    let (host, port, database, database_placeholder, username, sqlite_disabled, sqlite_path) =
        match engine {
            Engine::Postgres => (
                "localhost",
                5432,
                "postgres",
                "postgres",
                "postgres",
                false,
                "",
            ),
            Engine::Mysql | Engine::Mariadb => {
                ("localhost", 3306, "", "database name", "root", false, "")
            }
            Engine::SqlServer => (
                "localhost",
                1433,
                "master",
                "database name",
                "sa",
                false,
                "",
            ),
            Engine::Oracle => (
                "localhost",
                1521,
                "FREEPDB1",
                "service name",
                "system",
                false,
                "",
            ),
            Engine::Sqlite => ("", 0, "", "database name", "", true, ""),
        };
    set_input(&dialog.host, host, window, cx);
    set_input(&dialog.port, &port.to_string(), window, cx);
    set_input(&dialog.database, database, window, cx);
    set_placeholder(&dialog.database, database_placeholder, window, cx);
    set_input(&dialog.username, username, window, cx);
    set_input(&dialog.sqlite_path, sqlite_path, window, cx);
    set_input(&dialog.password, "", window, cx);
    dialog.ssl_disable = sqlite_disabled;
}

pub(crate) fn this_notice(tab: &mut Tab, message: impl Into<String>) {
    tab.notice = message.into();
}
