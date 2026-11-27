use koyo_core::models::{ColumnInfo, Engine, ForeignKeyInfo, QueryResultSet, TablePage};

use crate::app::ExportScope;

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
    let raw = if schema.is_empty() {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    };
    let name = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = name.trim_start_matches('.');
    let name = if name.is_empty() { "table" } else { name };
    format!("{name}.csv")
}

pub(crate) fn quote_sql_identifier(engine: Engine, name: &str) -> String {
    match engine {
        Engine::Mysql | Engine::Mariadb => format!("`{}`", name.replace('`', "``")),
        Engine::SqlServer => format!("[{}]", name.replace(']', "]]")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// Small preview query so "Query table" cannot accidentally load a whole
/// warehouse table into memory.
pub(crate) fn table_preview_sql(engine: Engine, schema: &str, table: &str) -> String {
    let relation = if schema.is_empty() {
        quote_sql_identifier(engine, table)
    } else {
        format!(
            "{}.{}",
            quote_sql_identifier(engine, schema),
            quote_sql_identifier(engine, table)
        )
    };
    match engine {
        Engine::SqlServer => format!("SELECT TOP 100 * FROM {relation};"),
        Engine::Oracle => format!("SELECT * FROM {relation} FETCH FIRST 100 ROWS ONLY;"),
        _ => format!("SELECT * FROM {relation} LIMIT 100;"),
    }
}

/// Best-effort CREATE TABLE from the loaded column and foreign-key metadata.
/// Defaults, checks, and generated expressions are not available here.
pub(crate) fn table_ddl(
    engine: Engine,
    schema: &str,
    table: &str,
    columns: &[ColumnInfo],
    foreign_keys: &[ForeignKeyInfo],
) -> String {
    let quote = |name: &str| quote_sql_identifier(engine, name);
    let relation = if schema.is_empty() {
        quote(table)
    } else {
        format!("{}.{}", quote(schema), quote(table))
    };
    let mut items = Vec::with_capacity(columns.len() + 1);
    for column in columns {
        let mut line = format!("    {} {}", quote(&column.name), column.data_type);
        if !column.nullable || column.primary_key {
            line.push_str(" NOT NULL");
        }
        items.push(line);
    }
    let primary = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| quote(&column.name))
        .collect::<Vec<_>>();
    if !primary.is_empty() {
        items.push(format!("    PRIMARY KEY ({})", primary.join(", ")));
    }
    for fk in foreign_keys
        .iter()
        .filter(|fk| fk.schema == schema && fk.table == table)
    {
        let constraint = if fk.constraint_name.is_empty() {
            String::new()
        } else {
            format!("CONSTRAINT {} ", quote(&fk.constraint_name))
        };
        let referenced = if fk.referenced_schema.is_empty() {
            quote(&fk.referenced_table)
        } else {
            format!(
                "{}.{}",
                quote(&fk.referenced_schema),
                quote(&fk.referenced_table)
            )
        };
        items.push(format!(
            "    {constraint}FOREIGN KEY ({}) REFERENCES {referenced} ({})",
            quote(&fk.column),
            quote(&fk.referenced_column)
        ));
    }
    let mut lines = vec![
        "-- Generated from current metadata.".to_string(),
        "-- Defaults, checks, and generated expressions are not included.".to_string(),
        format!("CREATE TABLE {relation} ("),
    ];
    if !items.is_empty() {
        lines.push(items.join(",\n"));
    }
    lines.push(");".to_string());
    lines.join("\n")
}

pub(crate) fn default_export_path(file_name: &str) -> String {
    let directory = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .map(|home| {
            let downloads = home.join("Downloads");
            if downloads.is_dir() { downloads } else { home }
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    directory.join(file_name).display().to_string()
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
    let mut used = std::collections::HashSet::with_capacity(result.columns.len());
    let keys = result
        .columns
        .iter()
        .map(|column| {
            let mut key = column.clone();
            let mut suffix = 2;
            while !used.insert(key.clone()) {
                key = format!("{column}_{suffix}");
                suffix += 1;
            }
            key
        })
        .collect::<Vec<_>>();
    let rows = result
        .rows
        .iter()
        .map(|row| {
            let mut object = serde_json::Map::with_capacity(keys.len());
            for (index, key) in keys.iter().enumerate() {
                let value = row
                    .get(index)
                    .and_then(Option::as_ref)
                    .map(|value| serde_json::Value::String(value.clone()))
                    .unwrap_or(serde_json::Value::Null);
                object.insert(key.clone(), value);
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
    let trimmed = field.trim_start();
    let formula_like = field.starts_with(['\t', '\r', '\n'])
        || (trimmed.starts_with(['=', '+', '-', '@']) && field.trim().parse::<f64>().is_err());
    let needs_quotes = field.contains(|character| matches!(character, ',' | '"' | '\n' | '\r'));
    if !needs_quotes && !formula_like {
        output.push_str(field);
        return;
    }
    output.push('"');
    if formula_like {
        output.push('\'');
    }
    for character in field.chars() {
        if character == '"' {
            output.push('"');
        }
        output.push(character);
    }
    output.push('"');
}
