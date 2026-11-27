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

fn filter_and_positions(input: &str) -> Vec<usize> {
    let bytes = input.as_bytes();
    let mut positions = Vec::new();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == active {
                if bytes.get(index + 1) == Some(&active) {
                    index += 2;
                    continue;
                }
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
            positions.push(index);
            index += 3;
            continue;
        }
        index += 1;
    }
    positions
}

fn current_filter_condition_start(input: &str, cursor: usize) -> usize {
    let cursor = input.floor_char_boundary(cursor.min(input.len()));
    filter_and_positions(&input[..cursor])
        .last()
        .map(|index| index + 3)
        .unwrap_or(0)
}

/// Split a leading column token from the rest of a condition. Returns the
/// unquoted column name and the byte offset in `text` where the remainder begins.
fn split_leading_column(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == b'"' {
        let mut index = 1;
        while index < bytes.len() {
            if bytes[index] == b'"' {
                if bytes.get(index + 1) == Some(&b'"') {
                    index += 2;
                    continue;
                }
                let name = text[1..index].replace("\"\"", "\"");
                return Some((name, index + 1));
            }
            index += 1;
        }
        return None;
    }
    let end = text
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace() || matches!(ch, '=' | '!' | '<' | '>'))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    (end > 0).then(|| (text[..end].to_string(), end))
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
    let cond_start = current_filter_condition_start(input, cursor);
    let cond_raw = &input[cond_start..cursor];
    let trimmed_offset = cond_raw.len() - cond_raw.trim_start().len();
    let cond = &cond_raw[trimmed_offset..];

    let Some((column_name, after_column)) = split_leading_column(cond) else {
        return columns
            .iter()
            .take(8)
            .map(|column| FilterSuggestion {
                label: column.name.clone(),
                insert: format!("{} ", column.name),
            })
            .collect();
    };

    if after_column == cond.len() {
        let lowercase = column_name.to_ascii_lowercase();
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

    let op_prefix = cond[after_column..].trim_start().to_ascii_lowercase();
    let Some(column) = columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(&column_name))
    else {
        return Vec::new();
    };
    filter_operator_suggestions(column.filter_kind)
        .iter()
        .filter(|(label, _)| label.to_ascii_lowercase().starts_with(&op_prefix))
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
    let cond_start = current_filter_condition_start(input, cursor);
    let cond_raw = &input[cond_start..cursor];
    let trimmed_start = cond_start + (cond_raw.len() - cond_raw.trim_start().len());
    let cond = &input[trimmed_start..cursor];

    let start = if let Some((_, after_column)) = split_leading_column(cond) {
        if after_column == cond.len() {
            trimmed_start
        } else {
            let rest = &cond[after_column..];
            let op_offset = rest.len() - rest.trim_start().len();
            let op_start = trimmed_start + after_column + op_offset;
            let op_prefix = &input[op_start..cursor];
            if insert
                .to_ascii_lowercase()
                .starts_with(&op_prefix.to_ascii_lowercase())
            {
                op_start
            } else {
                filter_token_start(input, cursor)
            }
        }
    } else {
        filter_token_start(input, cursor)
    };

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
    let mut parts = Vec::new();
    let mut start = 0;
    for index in filter_and_positions(input) {
        parts.push(input[start..index].trim());
        start = index + 3;
    }
    parts.push(input[start..].trim());
    parts
}

fn parse_filter_condition(columns: &[ColumnInfo], condition: &str) -> Result<TableFilter, String> {
    let condition = condition.trim();
    let Some((column_name, after_column)) = split_leading_column(condition) else {
        return Err(format!("Use an operator in filter: {condition}"));
    };
    let column = find_filter_column(columns, &column_name)?;
    let rest = condition[after_column..].trim();
    let rest_lower = rest.to_ascii_lowercase();

    let unary_words = rest_lower.split_ascii_whitespace().collect::<Vec<_>>();
    let unary = match unary_words.as_slice() {
        ["is", "not", "null"] => Some(FilterOperator::IsNotNull),
        ["is", "null"] => Some(FilterOperator::IsNull),
        ["is", "true"] => Some(FilterOperator::IsTrue),
        ["is", "false"] => Some(FilterOperator::IsFalse),
        _ => None,
    };
    if let Some(operator) = unary {
        ensure_filter_operator(column, operator)?;
        return Ok(TableFilter {
            column: column.name.clone(),
            operator,
            value: None,
        });
    }

    if let Some(after_contains) = rest_lower.strip_prefix("contains")
        && (after_contains.is_empty()
            || after_contains
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_whitespace))
    {
        let (value, was_quoted) = unquote_filter_value(&rest["contains".len()..]);
        if value.is_empty() && !was_quoted {
            return Err(format!("Filter on {} needs a value.", column.name));
        }
        ensure_filter_operator(column, FilterOperator::Contains)?;
        return Ok(TableFilter {
            column: column.name.clone(),
            operator: FilterOperator::Contains,
            value: Some(value),
        });
    }

    let operator_text = [">=", "<=", "!=", "<>", "=", ">", "<"]
        .into_iter()
        .find(|operator| rest.starts_with(operator))
        .ok_or_else(|| format!("Use an operator in filter: {condition}"))?;
    let (value, was_quoted) = unquote_filter_value(&rest[operator_text.len()..]);
    let mut operator = match operator_text {
        "=" => FilterOperator::Equals,
        "!=" | "<>" => FilterOperator::NotEquals,
        ">" => FilterOperator::GreaterThan,
        ">=" => FilterOperator::GreaterThanOrEqual,
        "<" => FilterOperator::LessThan,
        "<=" => FilterOperator::LessThanOrEqual,
        _ => unreachable!(),
    };
    if value.is_empty()
        && !(was_quoted && matches!(operator, FilterOperator::Equals | FilterOperator::NotEquals))
    {
        return Err(format!("Filter on {} needs a value.", column.name));
    }
    if column.filter_kind == FilterKind::Boolean && operator == FilterOperator::Equals {
        operator = match value.to_ascii_lowercase().as_str() {
            "true" | "1" => FilterOperator::IsTrue,
            "false" | "0" => FilterOperator::IsFalse,
            _ => return Err("Boolean filters use true or false.".into()),
        };
    }
    ensure_filter_operator(column, operator)?;
    Ok(TableFilter {
        column: column.name.clone(),
        operator,
        value: filter_needs_value(operator).then_some(value),
    })
}

fn find_filter_column<'a>(columns: &'a [ColumnInfo], name: &str) -> Result<&'a ColumnInfo, String> {
    let name = name.trim();
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

fn unquote_filter_value(value: &str) -> (String, bool) {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'' {
            return (value[1..value.len() - 1].replace("''", "'"), true);
        }
        if bytes[0] == b'"' && bytes[value.len() - 1] == b'"' {
            return (value[1..value.len() - 1].replace("\"\"", "\""), true);
        }
    }
    (value.to_string(), false)
}
