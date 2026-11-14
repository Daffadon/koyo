use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use anyhow::Result;
use gpui_kit::base::input::{CompletionProvider, Rope};
use gpui_kit::{App, Task, Window};
use koyo_core::models::{ColumnCatalogEntry, SchemaObject, SchemaObjectKind, TableSummary};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Position, Range as LspRange, TextEdit,
};

/// Snapshot of the connected database's names, handed to the editor's
/// completion provider. Refreshed whenever the explorer metadata reloads.
#[derive(Default)]
pub(crate) struct CompletionCatalog {
    schemas: BTreeSet<String>,
    tables: BTreeSet<(String, String)>,
    columns: BTreeSet<(String, String, String)>,
}

impl CompletionCatalog {
    pub(crate) fn replace(
        &mut self,
        tables: &[TableSummary],
        objects: &[SchemaObject],
        columns: &[ColumnCatalogEntry],
    ) {
        self.schemas = objects
            .iter()
            .map(|object| object.schema.clone())
            .chain(tables.iter().map(|table| table.schema.clone()))
            .chain(columns.iter().map(|column| column.schema.clone()))
            .collect();
        self.tables = objects
            .iter()
            .filter(|object| {
                matches!(
                    object.kind,
                    SchemaObjectKind::Table | SchemaObjectKind::View
                )
            })
            .map(|object| (object.schema.clone(), object.name.clone()))
            .chain(
                tables
                    .iter()
                    .map(|table| (table.schema.clone(), table.name.clone())),
            )
            .collect();
        self.columns = columns
            .iter()
            .map(|column| {
                (
                    column.schema.clone(),
                    column.table.clone(),
                    column.column.clone(),
                )
            })
            .collect();
    }

    pub(crate) fn clear(&mut self) {
        self.schemas.clear();
        self.tables.clear();
        self.columns.clear();
    }
}

pub(crate) struct SqlCompletionProvider {
    catalog: Rc<RefCell<CompletionCatalog>>,
}

impl SqlCompletionProvider {
    pub(crate) fn new(catalog: Rc<RefCell<CompletionCatalog>>) -> Self {
        Self { catalog }
    }
}

impl CompletionProvider for SqlCompletionProvider {
    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        new_text.chars().last().is_some_and(|character| {
            character.is_alphanumeric() || character == '_' || character == '.'
        })
    }

    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let text = text.to_string();
        let offset = offset.min(text.len());
        let offset = text.floor_char_boundary(offset);
        let before = &text[..offset];
        let token_start = before
            .char_indices()
            .rev()
            .find(|(_, character)| {
                !(character.is_alphanumeric() || *character == '_' || *character == '.')
            })
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);
        let token = &before[token_start..];
        let qualifier = token
            .rsplit_once('.')
            .map(|(qualifier, _)| clean_identifier(qualifier));
        let query = token.rsplit('.').next().unwrap_or(token);
        let starts_with = |value: &str| starts_with_ignore_case(value, query);

        let refs = parse_table_refs(before);
        let qualified_ref = qualifier
            .as_deref()
            .and_then(|qualifier| resolve_table_ref(&refs, qualifier));
        let replacement_start =
            token_start + token.rfind('.').map(|dot| dot + 1).unwrap_or_default();

        let catalog = self.catalog.borrow();
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();

        if qualifier.is_none() {
            for keyword in SQL_KEYWORDS {
                if starts_with(keyword) {
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: (*keyword).to_string(),
                            kind: Some(CompletionItemKind::KEYWORD),
                            detail: Some("SQL syntax".to_string()),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        // When the cursor is still on a table reference with no alias,
        // existing aliases belong to other tables. Showing them here invites
        // the user to create a duplicate; only alias candidates are relevant.
        let adding_alias = adding_table_alias(&refs);
        if qualifier.is_none() && !adding_alias {
            for alias in refs.iter().filter_map(|table_ref| table_ref.alias.as_ref()) {
                if starts_with(alias) {
                    let table = refs
                        .iter()
                        .find(|table_ref| table_ref.alias.as_deref() == Some(alias))
                        .map(table_ref_label)
                        .unwrap_or_default();
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: alias.clone(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(format!("{table} alias")),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        if qualifier.is_none()
            && let Some(last) = refs.last()
        {
            for alias in suggest_aliases(last, &refs) {
                if starts_with(&alias) {
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: alias,
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(format!("{} table alias", last.table)),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        if qualifier.is_none() {
            for schema in &catalog.schemas {
                if starts_with(schema) {
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: schema.clone(),
                            kind: Some(CompletionItemKind::MODULE),
                            detail: Some("schema".to_string()),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        if qualified_ref.is_none() {
            for (schema, table) in &catalog.tables {
                let qualified = format!("{schema}.{table}");
                if starts_with(table) || starts_with(&qualified) {
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: qualified.clone(),
                            kind: Some(CompletionItemKind::CLASS),
                            detail: Some(format!("{schema} table")),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        // Columns are only useful once the user has typed a name, or after a
        // dot. Returning every column on an empty prefix would drown the menu.
        if qualified_ref.is_some()
            || (!query.is_empty() && qualifier.is_none())
            || token.ends_with('.')
        {
            for (schema, table, column) in &catalog.columns {
                if let Some(table_ref) = &qualified_ref {
                    if table_ref
                        .schema
                        .as_ref()
                        .is_some_and(|ref_schema| !schema.eq_ignore_ascii_case(ref_schema))
                        || !table.eq_ignore_ascii_case(&table_ref.table)
                    {
                        continue;
                    }
                }
                if starts_with(column) {
                    let detail = format!("{schema}.{table}");
                    let text_edit = qualified_ref.as_ref().map(|_| {
                        CompletionTextEdit::Edit(TextEdit {
                            range: LspRange::new(
                                position_for_offset(&text, replacement_start),
                                position_for_offset(&text, offset),
                            ),
                            new_text: column.clone(),
                        })
                    });
                    push_item(
                        &mut items,
                        &mut seen,
                        CompletionItem {
                            label: column.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(detail),
                            text_edit,
                            ..Default::default()
                        },
                    );
                }
            }
        }

        items.truncate(300);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableRef {
    schema: Option<String>,
    table: String,
    alias: Option<String>,
}

fn table_ref_label(table_ref: &TableRef) -> String {
    match &table_ref.schema {
        Some(schema) => format!("{schema}.{}", table_ref.table),
        None => table_ref.table.clone(),
    }
}

fn clean_identifier(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim_matches('[')
        .trim_matches(']')
        .to_string()
}

fn tokenize_sql(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric()
            || matches!(character, '_' | '.' | '$' | '"' | '`' | '[' | ']')
        {
            current.push(character);
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if character == ',' {
                tokens.push(",".to_string());
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_table_refs(text: &str) -> Vec<TableRef> {
    let tokens = tokenize_sql(text);
    let mut refs = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let keyword = tokens[index].to_ascii_uppercase();
        if !matches!(keyword.as_str(), "FROM" | "JOIN" | "UPDATE") {
            index += 1;
            continue;
        }
        index += 1;
        while index < tokens.len() {
            if tokens[index] == "," {
                index += 1;
                continue;
            }
            if tokens[index].starts_with('(') || is_clause_keyword(&tokens[index]) {
                break;
            }
            let raw = clean_identifier(&tokens[index]);
            if raw.is_empty() {
                break;
            }
            let (schema, table) = split_table_name(&raw);
            index += 1;
            let alias = if index < tokens.len() && tokens[index].eq_ignore_ascii_case("AS") {
                index += 1;
                if index < tokens.len() {
                    let alias = clean_identifier(&tokens[index]);
                    index += 1;
                    (!alias.is_empty()).then_some(alias)
                } else {
                    None
                }
            } else if index < tokens.len() && is_alias_token(&tokens[index]) {
                let alias = clean_identifier(&tokens[index]);
                index += 1;
                (!alias.is_empty()).then_some(alias)
            } else {
                None
            };
            refs.push(TableRef {
                schema,
                table,
                alias,
            });
            if keyword == "FROM" && index < tokens.len() && tokens[index] == "," {
                index += 1;
                continue;
            }
            break;
        }
    }
    refs
}

fn split_table_name(value: &str) -> (Option<String>, String) {
    match value.rsplit_once('.') {
        Some((schema, table)) => (
            Some(
                schema
                    .rsplit('.')
                    .next()
                    .unwrap_or(schema)
                    .trim_matches(['"', '`', '[', ']'])
                    .to_string(),
            ),
            table.trim_matches(['"', '`', '[', ']']).to_string(),
        ),
        None => (None, value.to_string()),
    }
}

fn is_alias_token(token: &str) -> bool {
    token != ","
        && !token.starts_with('(')
        && !is_clause_keyword(token)
        && token.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, '_' | '$' | '"' | '`' | '[' | ']')
        })
}

fn is_clause_keyword(token: &str) -> bool {
    matches!(
        token.to_ascii_uppercase().as_str(),
        "SELECT"
            | "FROM"
            | "WHERE"
            | "GROUP"
            | "ORDER"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "INNER"
            | "CROSS"
            | "OUTER"
            | "ON"
            | "USING"
            | "UNION"
            | "EXCEPT"
            | "INTERSECT"
            | "SET"
            | "VALUES"
            | "RETURNING"
            | "INSERT"
            | "INTO"
            | "UPDATE"
            | "DELETE"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "WITH"
    )
}

fn resolve_table_ref(refs: &[TableRef], qualifier: &str) -> Option<TableRef> {
    refs.iter()
        .rev()
        .find(|table_ref| {
            table_ref
                .alias
                .as_deref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
                || table_ref.table.eq_ignore_ascii_case(qualifier)
                || table_ref_label(table_ref).eq_ignore_ascii_case(qualifier)
        })
        .cloned()
}

fn same_table(left: &TableRef, right: &TableRef) -> bool {
    left.table.eq_ignore_ascii_case(&right.table)
}

fn default_alias(table: &str) -> String {
    let alias = table
        .split('_')
        .filter_map(|part| part.chars().next())
        .map(|character| character.to_ascii_lowercase())
        .collect::<String>();
    if alias.is_empty() {
        "t".to_string()
    } else {
        alias
    }
}

fn adding_table_alias(refs: &[TableRef]) -> bool {
    let Some(last) = refs.last() else {
        return false;
    };
    let previous = &refs[..refs.len().saturating_sub(1)];
    last.alias.is_none()
        || previous.iter().any(|table_ref| same_table(table_ref, last))
        || last.alias.as_ref().is_some_and(|alias| {
            previous
                .iter()
                .any(|table_ref| table_ref.alias.as_deref() == Some(alias))
        })
}

fn suggest_aliases(last: &TableRef, refs: &[TableRef]) -> Vec<String> {
    let last_index = refs.len().saturating_sub(1);
    let mut used = refs[..last_index]
        .iter()
        .filter_map(|table_ref| table_ref.alias.as_ref())
        .map(|alias| alias.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    if let Some(alias) = &last.alias {
        if !used.contains(&alias.to_ascii_lowercase()) {
            return Vec::new();
        }
        used.insert(alias.to_ascii_lowercase());
    }
    let prior_same = refs[..last_index]
        .iter()
        .filter(|table_ref| same_table(table_ref, last))
        .count();
    let base = last
        .alias
        .clone()
        .unwrap_or_else(|| default_alias(&last.table));
    let start = prior_same + 1;
    let mut aliases = Vec::new();
    for number in start..=start + 8 {
        let candidate = if number == 1 {
            base.clone()
        } else {
            format!("{base}{number}")
        };
        if used.contains(&candidate.to_ascii_lowercase()) {
            continue;
        }
        aliases.push(candidate);
        if aliases.len() == 3 {
            break;
        }
    }
    aliases
}

fn position_for_offset(text: &str, offset: usize) -> Position {
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map(|index| index + 1).unwrap_or(0);
    let character = prefix[line_start..].encode_utf16().count() as u32;
    Position::new(line, character)
}

fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    let mut value_chars = value.chars();
    prefix.chars().all(|prefix_char| {
        value_chars
            .next()
            .is_some_and(|value_char| value_char.eq_ignore_ascii_case(&prefix_char))
    })
}

fn push_item(
    items: &mut Vec<CompletionItem>,
    seen: &mut BTreeSet<(String, String)>,
    item: CompletionItem,
) {
    let key = (item.label.clone(), item.detail.clone().unwrap_or_default());
    if seen.insert(key) {
        items.push(item);
    }
}

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "INNER JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS NULL",
    "IS NOT NULL",
    "LIKE",
    "IN",
    "BETWEEN",
    "EXISTS",
    "DISTINCT",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "UNION",
    "UNION ALL",
    "WITH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CAST",
    "ASC",
    "DESC",
    "EXPLAIN",
    "ANALYZE",
    "PRAGMA",
    "VACUUM",
    "ATTACH",
    "DETACH",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "CREATE",
    "ALTER",
    "DROP",
    "CREATE INDEX",
    "CREATE VIEW",
    "PRIMARY KEY",
    "FOREIGN KEY",
    "REFERENCES",
    "UNIQUE",
    "CHECK",
    "DEFAULT",
    "CASCADE",
    "RESTRICT",
    "RETURNING",
    "CONFLICT",
    "DO NOTHING",
    "DO UPDATE",
    "TOP",
    "FETCH FIRST",
    "NULLS FIRST",
    "NULLS LAST",
    "OVER",
    "PARTITION BY",
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "LAG",
    "LEAD",
    "NOW",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "DATE_TRUNC",
    "EXTRACT",
    "SUBSTRING",
    "TRIM",
    "LOWER",
    "UPPER",
    "LENGTH",
    "REPLACE",
    "CONCAT",
    "MERGE",
    "MATCHED",
    "USING",
    "START WITH",
    "CONNECT BY",
    "DECODE",
    "NVL",
    "ROWNUM",
    "DUAL",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_table_aliases() {
        let refs = parse_table_refs(
            "SELECT u.email FROM public.users AS u JOIN orders o ON o.user_id = u.id",
        );
        assert_eq!(
            refs,
            vec![
                TableRef {
                    schema: Some("public".into()),
                    table: "users".into(),
                    alias: Some("u".into()),
                },
                TableRef {
                    schema: None,
                    table: "orders".into(),
                    alias: Some("o".into()),
                },
            ]
        );
    }

    #[test]
    fn numbers_duplicate_table_aliases() {
        let refs = vec![
            TableRef {
                schema: None,
                table: "users".into(),
                alias: None,
            },
            TableRef {
                schema: None,
                table: "users".into(),
                alias: None,
            },
        ];
        assert_eq!(suggest_aliases(&refs[1], &refs), vec!["u2", "u3", "u4"]);
    }

    #[test]
    fn numbers_a_repeated_explicit_alias() {
        let refs = vec![
            TableRef {
                schema: None,
                table: "users".into(),
                alias: Some("u".into()),
            },
            TableRef {
                schema: None,
                table: "users".into(),
                alias: Some("u".into()),
            },
        ];
        assert_eq!(suggest_aliases(&refs[1], &refs), vec!["u2", "u3", "u4"]);
    }

    #[test]
    fn repeated_table_is_adding_an_alias() {
        let refs = vec![
            TableRef {
                schema: None,
                table: "users".into(),
                alias: Some("u".into()),
            },
            TableRef {
                schema: None,
                table: "users".into(),
                alias: Some("u".into()),
            },
        ];
        assert!(adding_table_alias(&refs));
    }

    #[test]
    fn unique_alias_needs_no_numbered_variant() {
        let refs = vec![TableRef {
            schema: None,
            table: "order_items".into(),
            alias: Some("oi".into()),
        }];
        assert!(suggest_aliases(&refs[0], &refs).is_empty());
        assert_eq!(default_alias("order_items"), "oi");
    }
}
