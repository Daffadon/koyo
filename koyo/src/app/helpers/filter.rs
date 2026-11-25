use koyo_core::common::ensure_filter_operator;
use koyo_core::models::{ColumnInfo, Engine, FilterKind, FilterOperator, TableFilter};

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
#[derive(Clone)]
pub(crate) struct FilterSuggestion {
    pub(crate) label: String,
    pub(crate) insert: String,
}

fn filter_token_start(input: &str, cursor: usize) -> usize {
    let cursor = input.floor_char_boundary(cursor.min(input.len()));
    input[..cursor]
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !(character.is_alphanumeric() || matches!(character, '_' | '.' | '=' | '!' | '<' | '>'))
        })
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0)
}

fn current_filter_condition_start(input: &str, cursor: usize) -> usize {
    let bytes = input[..input.floor_char_boundary(cursor.min(input.len()))].as_bytes();
    let mut quote = None;
    let mut start = 0;
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
        let is_and = index + 3 <= bytes.len()
            && bytes[index..index + 3].eq_ignore_ascii_case(b"and")
            && index > 0
            && bytes[index - 1].is_ascii_whitespace()
            && bytes
                .get(index + 3)
                .is_some_and(|byte| byte.is_ascii_whitespace());
        if is_and {
            start = index + 3;
            index += 3;
            continue;
        }
        index += 1;
    }
    start
}

fn filter_operator_suggestions(kind: FilterKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        FilterKind::Text => &[
            ("contains", "contains "),
            ("=", "= "),
            ("!=", "!= "),
            ("is null", "is null "),
            ("is not null", "is not null "),
        ],
        FilterKind::Number | FilterKind::Temporal => &[
            ("=", "= "),
            ("!=", "!= "),
            (">", "> "),
            (">=", ">= "),
            ("<", "< "),
            ("<=", "<= "),
            ("is null", "is null "),
            ("is not null", "is not null "),
        ],
        FilterKind::Boolean => &[
            ("is true", "is true "),
            ("is false", "is false "),
            ("is null", "is null "),
            ("is not null", "is not null "),
        ],
        FilterKind::Other => &[("is null", "is null "), ("is not null", "is not null ")],
    }
}

pub(crate) fn filter_suggestions(
    columns: &[ColumnInfo],
    input: &str,
    cursor: usize,
) -> Vec<FilterSuggestion> {
    let cursor = input.floor_char_boundary(cursor.min(input.len()));
    let token_start = filter_token_start(input, cursor);
    let token = &input[token_start..cursor];
    let prefix = input[current_filter_condition_start(input, token_start)..token_start].trim();
    let lowercase = token.to_ascii_lowercase();

    if prefix.is_empty() {
        return columns
            .iter()
            .filter(|column| column.name.to_ascii_lowercase().starts_with(&lowercase))
            .take(8)
            .map(|column| FilterSuggestion {
                label: column.name.clone(),
                insert: format!("{} ", column.name),
            })
            .collect();
    }

    let Some(column) = columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(prefix))
    else {
        return Vec::new();
    };
    filter_operator_suggestions(column.filter_kind)
        .iter()
        .filter(|(label, _)| label.to_ascii_lowercase().starts_with(&lowercase))
        .take(8)
        .map(|(label, insert)| FilterSuggestion {
            label: (*label).to_string(),
            insert: (*insert).to_string(),
        })
        .collect()
}

pub(crate) fn insert_filter_suggestion(
    input: &str,
    cursor: usize,
    insert: &str,
) -> (String, usize) {
    let cursor = input.floor_char_boundary(cursor.min(input.len()));
    let start = filter_token_start(input, cursor);
    let mut value = String::with_capacity(input.len() + insert.len());
    value.push_str(&input[..start]);
    value.push_str(insert);
    value.push_str(&input[cursor..]);
    (value, start + insert.len())
}

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
    let mut quote = None;
    for (index, character) in condition.char_indices() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            continue;
        }
        let remaining = &condition[index..];
        for operator in [">=", "<=", "!=", "<>", "=", ">", "<"] {
            if remaining.starts_with(operator) {
                return Some((index, operator));
            }
        }
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
