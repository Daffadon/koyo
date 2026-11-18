use gpui_kit::component::input::InputState;
use gpui_kit::*;
use koyo_core::common::ensure_filter_operator;
use koyo_core::models::{
    ColumnInfo, ConnectionProfile, Engine, FilterKind, FilterOperator, QueryResultSet, TableFilter,
    TablePage,
};

use super::state::{ConnectionDialog, ExportScope, QueryTab, Tab};

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

/// Tab text counts as unsaved when it differs from the starter SQL that
/// connection setup may still replace.
pub(crate) fn query_has_unsaved_sql(query: &QueryTab, cx: &App) -> bool {
    query.editor.read(cx).value() != query.starter
}

/// Active result set as CSV. NULL and an empty string both export as an empty
/// field; CSV cannot represent the difference without a non-standard marker.
pub(crate) fn results_as_csv(result: &QueryResultSet) -> String {
    if result.columns.is_empty() {
        return String::new();
    }
    let mut output = String::new();
    push_csv_row(&mut output, result.columns.iter().map(String::as_str));
    for row in &result.rows {
        push_csv_row(
            &mut output,
            (0..result.columns.len())
                .map(|index| row.get(index).and_then(Option::as_deref).unwrap_or("")),
        );
    }
    output
}

/// Default file name for a table export. Keep path separators out of it.
pub(crate) fn table_csv_file_name(schema: &str, table: &str) -> String {
    let name = format!("{schema}.{table}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{name}.csv")
}

/// Table page as CSV. The first page carries the header row.
pub(crate) fn table_page_as_csv(page: &TablePage, header: bool) -> String {
    if page.columns.is_empty() {
        return String::new();
    }
    let mut output = String::new();
    if header {
        push_csv_row(
            &mut output,
            page.columns.iter().map(|column| column.name.as_str()),
        );
    }
    for row in &page.rows {
        push_csv_row(
            &mut output,
            (0..page.columns.len()).map(|index| {
                row.values
                    .get(index)
                    .and_then(Option::as_deref)
                    .unwrap_or("")
            }),
        );
    }
    output
}

/// Resolve the export dialog's scope to an inclusive 1-based page range.
/// `total_pages` is 0 when the row count is unknown.
pub(crate) fn resolve_export_pages(
    scope: ExportScope,
    current_page: u32,
    page: &str,
    range_start: &str,
    range_end: &str,
    total_pages: u32,
) -> Result<(u32, Option<u32>), String> {
    let parse_page = |value: &str, label: &str| {
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("Enter a {label}."));
        }
        value
            .parse::<u32>()
            .map_err(|_| format!("{label} must be a whole number."))
            .and_then(|page| {
                if page == 0 {
                    Err(format!("{label} starts at 1."))
                } else {
                    Ok(page)
                }
            })
    };
    let (first, last) = match scope {
        ExportScope::CurrentPage => (current_page, Some(current_page)),
        ExportScope::Page => {
            let page = parse_page(page, "page number")?;
            (page, Some(page))
        }
        ExportScope::Range => {
            let first = parse_page(range_start, "start page")?;
            let last = parse_page(range_end, "end page")?;
            if last < first {
                return Err("End page must be at or after the start page.".into());
            }
            (first, Some(last))
        }
        ExportScope::All => (1, None),
    };
    if total_pages > 0 {
        if first > total_pages {
            return Err(format!("This table has {total_pages} pages."));
        }
        if let Some(last) = last
            && last > total_pages
        {
            return Err(format!("This table has {total_pages} pages."));
        }
    }
    Ok((first, last))
}

pub(crate) fn results_as_json(result: &QueryResultSet) -> String {
    let rows = result
        .rows
        .iter()
        .map(|row| {
            let mut object = serde_json::Map::with_capacity(result.columns.len());
            for (index, column) in result.columns.iter().enumerate() {
                let value = row
                    .get(index)
                    .and_then(Option::as_ref)
                    .map(|value| serde_json::Value::String(value.clone()))
                    .unwrap_or(serde_json::Value::Null);
                object.insert(column.clone(), value);
            }
            serde_json::Value::Object(object)
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn push_csv_row<'a>(output: &mut String, fields: impl Iterator<Item = &'a str>) {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            output.push(',');
        }
        push_csv_field(output, field);
    }
    output.push('\n');
}

fn push_csv_field(output: &mut String, field: &str) {
    let formula_like = field.chars().next().is_some_and(|first| {
        matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r') && field.parse::<f64>().is_err()
    });
    if formula_like {
        output.push('\'');
    }
    if !field.contains(|character| matches!(character, ',' | '"' | '\n' | '\r')) {
        output.push_str(field);
        return;
    }
    output.push('"');
    for character in field.chars() {
        if character == '"' {
            output.push('"');
        }
        output.push(character);
    }
    output.push('"');
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

/// Parse a DBeaver-style filter expression. Supported forms:
/// `column = value`, `column != value`, comparisons, `contains`, and
/// `is null` / `is not null` / `is true` / `is false`, joined with `AND`.
pub(crate) fn parse_filter_input(
    columns: &[ColumnInfo],
    input: &str,
) -> Result<Vec<TableFilter>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut filters = Vec::new();
    for condition in split_filter_conditions(input) {
        filters.push(parse_filter_condition(columns, condition)?);
    }
    Ok(filters)
}

fn split_filter_conditions(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        let is_and = index + 3 <= bytes.len()
            && bytes[index..index + 3].eq_ignore_ascii_case(b"and")
            && index > 0
            && bytes[index - 1].is_ascii_whitespace()
            && bytes
                .get(index + 3)
                .is_some_and(|byte| byte.is_ascii_whitespace());
        if is_and {
            parts.push(input[start..index].trim());
            index += 3;
            start = index;
            continue;
        }
        index += 1;
    }
    parts.push(input[start..].trim());
    parts
}

fn parse_filter_condition(columns: &[ColumnInfo], condition: &str) -> Result<TableFilter, String> {
    let condition = condition.trim();
    let lower = condition.to_ascii_lowercase();
    for (suffix, operator) in [
        (" is not null", FilterOperator::IsNotNull),
        (" is null", FilterOperator::IsNull),
        (" is true", FilterOperator::IsTrue),
        (" is false", FilterOperator::IsFalse),
    ] {
        if let Some(column_text) = lower.strip_suffix(suffix) {
            let column = find_filter_column(columns, condition[..column_text.len()].trim())?;
            ensure_filter_operator(column, operator)?;
            return Ok(TableFilter {
                column: column.name.clone(),
                operator,
                value: None,
            });
        }
    }
    if let Some(position) = lower.find(" contains ") {
        let column = find_filter_column(columns, condition[..position].trim())?;
        let value = unquote_filter_value(&condition[position + " contains ".len()..]);
        if value.is_empty() {
            return Err(format!("Filter on {} needs a value.", column.name));
        }
        ensure_filter_operator(column, FilterOperator::Contains)?;
        return Ok(TableFilter {
            column: column.name.clone(),
            operator: FilterOperator::Contains,
            value: Some(value),
        });
    }

    let (position, operator_text) = find_filter_operator(condition)
        .ok_or_else(|| format!("Use an operator in filter: {condition}"))?;
    let column = find_filter_column(columns, condition[..position].trim())?;
    let value = unquote_filter_value(&condition[position + operator_text.len()..]);
    if value.is_empty() {
        return Err(format!("Filter on {} needs a value.", column.name));
    }
    let mut operator = match operator_text {
        "=" => FilterOperator::Equals,
        "!=" | "<>" => FilterOperator::NotEquals,
        ">" => FilterOperator::GreaterThan,
        ">=" => FilterOperator::GreaterThanOrEqual,
        "<" => FilterOperator::LessThan,
        "<=" => FilterOperator::LessThanOrEqual,
        _ => unreachable!(),
    };
    if column.filter_kind == FilterKind::Boolean {
        if operator == FilterOperator::Equals {
            operator = match value.to_ascii_lowercase().as_str() {
                "true" | "1" => FilterOperator::IsTrue,
                "false" | "0" => FilterOperator::IsFalse,
                _ => return Err("Boolean filters use true or false.".into()),
            };
        }
    }
    ensure_filter_operator(column, operator)?;
    Ok(TableFilter {
        column: column.name.clone(),
        operator,
        value: filter_needs_value(operator).then_some(value),
    })
}

fn find_filter_column<'a>(columns: &'a [ColumnInfo], name: &str) -> Result<&'a ColumnInfo, String> {
    let name = name.trim().trim_matches('"');
    columns
        .iter()
        .find(|column| column.name == name)
        .or_else(|| {
            columns
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| format!("Column {name} does not exist in this table."))
}

fn find_filter_operator(condition: &str) -> Option<(usize, &'static str)> {
    let bytes = condition.as_bytes();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        let remaining = &condition[index..];
        for operator in [">=", "<=", "!=", "<>", "=", ">", "<"] {
            if remaining.starts_with(operator) {
                return Some((index, operator));
            }
        }
        index += 1;
    }
    None
}

fn unquote_filter_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
        {
            return value[1..value.len() - 1].replace("''", "'");
        }
    }
    value.to_string()
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

#[cfg(test)]
mod tests {
    use koyo_core::models::{ColumnInfo, FilterKind, FilterOperator, QueryResultSet};

    use super::{
        parse_filter_input, resolve_export_pages, results_as_csv, results_as_json,
        table_csv_file_name, table_page_as_csv,
    };
    use crate::app::ExportScope;

    #[test]
    fn csv_quotes_commas_quotes_newlines_and_nulls() {
        let result = QueryResultSet {
            columns: vec!["name".into(), "note, quoted".into()],
            rows: vec![
                vec![Some("Ada".into()), Some("a,b".into())],
                vec![Some("quote\"d".into()), None],
                vec![Some("line\nbreak".into()), Some(String::new())],
            ],
        };

        assert_eq!(
            results_as_csv(&result),
            "name,\"note, quoted\"\nAda,\"a,b\"\n\"quote\"\"d\",\n\"line\nbreak\",\n"
        );
    }

    #[test]
    fn csv_guards_spreadsheet_formulas_but_keeps_numbers() {
        let result = QueryResultSet {
            columns: vec!["value".into()],
            rows: vec![
                vec![Some("=HYPERLINK(\"https://example.test\")".into())],
                vec![Some("-12.5".into())],
                vec![Some("-1+2".into())],
                vec![Some("@cmd".into())],
            ],
        };

        assert_eq!(
            results_as_csv(&result),
            "value\n'\"=HYPERLINK(\"\"https://example.test\"\")\"\n-12.5\n'-1+2\n'@cmd\n"
        );
    }

    #[test]
    fn json_result_keeps_nulls_and_column_names() {
        let result = QueryResultSet {
            columns: vec!["name".into(), "note".into()],
            rows: vec![vec![Some("Ada".into()), None]],
        };
        let json = results_as_json(&result);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed[0]["name"], "Ada");
        assert!(parsed[0]["note"].is_null());
    }

    #[test]
    fn table_csv_writes_header_once_and_guards_formulas() {
        let page = koyo_core::models::TablePage {
            columns: vec![ColumnInfo {
                name: "name".into(),
                data_type: "text".into(),
                filter_type: "text".into(),
                filter_kind: FilterKind::Text,
                sortable: true,
                nullable: true,
                primary_key: false,
                generated: false,
                identity: false,
            }],
            rows: vec![koyo_core::models::GridRow {
                version: String::new(),
                key: Vec::new(),
                values: vec![Some("=cmd".into())],
                truncated: Vec::new(),
            }],
            offset: 0,
            limit: 100,
            total_rows: Some(1),
            has_more: false,
            editable: false,
        };

        assert_eq!(table_page_as_csv(&page, true), "name\n'=cmd\n");
        assert_eq!(table_page_as_csv(&page, false), "'=cmd\n");
    }

    #[test]
    fn table_export_file_name_stays_a_file_name() {
        assert_eq!(table_csv_file_name("main", "people"), "main.people.csv");
        assert_eq!(table_csv_file_name("a/b", "c\\d"), "a_b.c_d.csv");
    }

    #[test]
    fn export_scope_resolves_and_validates_pages() {
        assert_eq!(
            resolve_export_pages(ExportScope::CurrentPage, 3, "", "", "", 5),
            Ok((3, Some(3)))
        );
        assert_eq!(
            resolve_export_pages(ExportScope::Page, 0, "4", "", "", 5),
            Ok((4, Some(4)))
        );
        assert_eq!(
            resolve_export_pages(ExportScope::Range, 0, "", "2", "4", 5),
            Ok((2, Some(4)))
        );
        assert_eq!(
            resolve_export_pages(ExportScope::All, 0, "", "", "", 5),
            Ok((1, None))
        );
        assert!(resolve_export_pages(ExportScope::Page, 0, "0", "", "", 5).is_err());
        assert!(resolve_export_pages(ExportScope::Range, 0, "", "4", "2", 5).is_err());
        assert!(resolve_export_pages(ExportScope::All, 0, "", "", "", 0).is_ok());
        assert!(resolve_export_pages(ExportScope::Page, 0, "6", "", "", 5).is_err());
    }

    #[test]
    fn csv_without_columns_is_empty() {
        let result = QueryResultSet {
            columns: Vec::new(),
            rows: Vec::new(),
        };

        assert!(results_as_csv(&result).is_empty());
    }

    fn filter_column(name: &str, kind: FilterKind) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: "text".into(),
            filter_type: "text".into(),
            filter_kind: kind,
            sortable: true,
            nullable: true,
            primary_key: false,
            generated: false,
            identity: false,
        }
    }

    #[test]
    fn filter_input_parses_and_conditions() {
        let columns = vec![
            filter_column("name", FilterKind::Text),
            filter_column("age", FilterKind::Number),
        ];
        let filters = parse_filter_input(&columns, "name contains 'A AND B' AND age >= 30")
            .expect("filter expression");
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].column, "name");
        assert_eq!(filters[0].operator, FilterOperator::Contains);
        assert_eq!(filters[0].value.as_deref(), Some("A AND B"));
        assert_eq!(filters[1].operator, FilterOperator::GreaterThanOrEqual);
        assert_eq!(filters[1].value.as_deref(), Some("30"));
    }

    #[test]
    fn filter_input_maps_boolean_equality() {
        let columns = vec![filter_column("enabled", FilterKind::Boolean)];
        let filters = parse_filter_input(&columns, "enabled = false").expect("boolean filter");
        assert_eq!(filters[0].operator, FilterOperator::IsFalse);
        assert_eq!(filters[0].value, None);
    }

    #[test]
    fn filter_input_rejects_unknown_columns() {
        let columns = vec![filter_column("name", FilterKind::Text)];
        assert!(parse_filter_input(&columns, "missing = 1").is_err());
    }
}
