use crate::models::{ColumnInfo, FilterKind, FilterOperator, TableSort};
use std::fmt::Write;

pub const QUERY_ROW_LIMIT: usize = 1_000;
pub const TABLE_PAGE_LIMIT: u32 = 200;
pub const MAX_CELL_CHARS: usize = 65_536;
pub const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RESULT_SETS: usize = 64;
pub const MAX_AFFECTED_ROWS: usize = 64;

/// Budget shared by every SQL-result path. It caps each cell and the total
/// retained bytes while rows are copied, so an oversized result cannot grow
/// the client heap before a post-processing pass sees it.
pub struct QueryBudget {
    remaining: usize,
    truncated: bool,
}

impl Default for QueryBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryBudget {
    pub fn new() -> Self {
        Self {
            remaining: MAX_RESULT_BYTES,
            truncated: false,
        }
    }

    pub fn exhausted(&self) -> bool {
        self.remaining == 0
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Charge retained container/slot bytes that are not part of a value's
    /// string length. Returns false when the budget is exhausted.
    pub fn charge_bytes(&mut self, bytes: usize) -> bool {
        if self.remaining < bytes {
            self.remaining = 0;
            self.truncated = true;
            false
        } else {
            self.remaining -= bytes;
            true
        }
    }

    fn charge_slot(&mut self) -> bool {
        self.charge_bytes(std::mem::size_of::<Option<String>>())
    }

    pub fn column(&mut self, mut value: String) -> String {
        if !self.charge_slot() {
            return String::new();
        }
        let cap = self.remaining.min(MAX_CELL_CHARS);
        if value.len() > cap {
            value.truncate(floor_char_boundary(&value, cap));
            if !value.ends_with(" [truncated]") {
                value.push_str(" [truncated]");
            }
            self.truncated = true;
        }
        self.remaining = self.remaining.saturating_sub(value.len());
        value
    }

    pub fn cell(&mut self, value: Option<String>) -> Option<String> {
        self.page_cell_flagged(value).0
    }

    /// Table-page cells are retained for editing. Reports whether this cell was
    /// clipped or replaced so the UI refuses edits that would write partial data.
    pub fn page_cell_flagged(&mut self, value: Option<String>) -> (Option<String>, bool) {
        if !self.charge_slot() {
            return (None, true);
        }
        let Some(mut value) = value else {
            return (None, false);
        };
        if self.remaining == 0 && !value.is_empty() {
            self.truncated = true;
            return (None, true);
        }
        let cap = self.remaining.min(MAX_CELL_CHARS);
        let mut clipped = false;
        if value.len() > cap {
            value.truncate(floor_char_boundary(&value, cap));
            if !value.ends_with(" [truncated]") {
                value.push_str(" [truncated]");
            }
            self.truncated = true;
            clipped = true;
        }
        self.remaining = self.remaining.saturating_sub(value.len());
        (Some(value), clipped)
    }

    pub fn cell_str(&mut self, value: Option<&str>) -> Option<String> {
        if !self.charge_slot() {
            return None;
        }
        let value = value?;
        if self.remaining == 0 && !value.is_empty() {
            self.truncated = true;
            return None;
        }
        let cap = self.remaining.min(MAX_CELL_CHARS);
        let end = floor_char_boundary(value, cap);
        let clipped = end < value.len();
        let mut output = String::with_capacity(end + if clipped { 12 } else { 0 });
        output.push_str(&value[..end]);
        if clipped {
            output.push_str(" [truncated]");
            self.truncated = true;
        }
        self.remaining = self.remaining.saturating_sub(output.len());
        Some(output)
    }
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub struct TableMeta {
    pub columns: Vec<ColumnInfo>,
    pub primary_indices: Vec<usize>,
    pub relation_kind: Option<String>,
}

impl TableMeta {
    pub fn new(columns: Vec<ColumnInfo>, relation_kind: Option<String>) -> Self {
        let primary_indices = columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.primary_key)
            .map(|(index, _)| index)
            .collect();
        Self {
            columns,
            primary_indices,
            relation_kind,
        }
    }

    pub fn primary_columns(&self) -> impl Iterator<Item = &ColumnInfo> {
        self.columns.iter().filter(|column| column.primary_key)
    }

    pub fn editable(&self) -> bool {
        if self.primary_indices.is_empty() {
            return false;
        }
        match self.relation_kind.as_deref() {
            Some(kind) => matches!(kind, "r" | "p"),
            None => true,
        }
    }
}

pub fn find_column<'a>(columns: &'a [ColumnInfo], name: &str) -> Result<&'a ColumnInfo, String> {
    columns
        .iter()
        .find(|column| column.name == name)
        .ok_or_else(|| format!("Column {name} does not exist in this table."))
}

pub fn ensure_filter_operator(column: &ColumnInfo, operator: FilterOperator) -> Result<(), String> {
    let allowed = match column.filter_kind {
        FilterKind::Text => matches!(
            operator,
            FilterOperator::Contains
                | FilterOperator::Equals
                | FilterOperator::NotEquals
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Number | FilterKind::Temporal => matches!(
            operator,
            FilterOperator::Equals
                | FilterOperator::NotEquals
                | FilterOperator::LessThan
                | FilterOperator::LessThanOrEqual
                | FilterOperator::GreaterThan
                | FilterOperator::GreaterThanOrEqual
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Boolean => matches!(
            operator,
            FilterOperator::IsTrue
                | FilterOperator::IsFalse
                | FilterOperator::IsNull
                | FilterOperator::IsNotNull
        ),
        FilterKind::Other => {
            matches!(operator, FilterOperator::IsNull | FilterOperator::IsNotNull)
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "Filter operator is not valid for column {}.",
            column.name
        ))
    }
}

pub fn comparison_sql(operator: FilterOperator) -> Option<&'static str> {
    match operator {
        FilterOperator::Equals => Some("="),
        FilterOperator::NotEquals => Some("<>"),
        FilterOperator::LessThan => Some("<"),
        FilterOperator::LessThanOrEqual => Some("<="),
        FilterOperator::GreaterThan => Some(">"),
        FilterOperator::GreaterThanOrEqual => Some(">="),
        _ => None,
    }
}

pub fn ensure_writable(column: &ColumnInfo) -> Result<(), String> {
    if !column.generated && !column.identity {
        return Ok(());
    }
    Err(format!("Column {} cannot be edited.", column.name))
}

pub fn ensure_untruncated_primary_keys(
    primary_indices: &[usize],
    truncated: &[bool],
) -> Result<(), String> {
    if primary_indices
        .iter()
        .any(|index| truncated.get(*index).copied().unwrap_or(false))
    {
        return Err("Rows with truncated primary-key values cannot be edited or deleted.".into());
    }
    Ok(())
}

pub fn ensure_binary_hex(value: &Option<String>) -> Result<Option<&str>, String> {
    let Some(text) = value.as_deref() else {
        return Ok(None);
    };
    let hex = text.strip_prefix("\\x").unwrap_or(text);
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Binary values must be even-length hexadecimal text.".into());
    }
    Ok(Some(hex))
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

pub fn ensure_one_row(count: u64) -> Result<(), String> {
    match count {
        1 => Ok(()),
        0 => Err(
            "The row changed or was deleted after this page loaded. Reload before retrying.".into(),
        ),
        _ => Err("The change matched more than one row. Reload before retrying.".into()),
    }
}

pub fn build_keyed_order_by(
    columns: &[ColumnInfo],
    primary_indices: &[usize],
    sort: Option<&TableSort>,
    table: &str,
    quote: impl Fn(&str) -> String,
) -> Result<String, String> {
    let order_column = |name: &str| format!("{}.{}", quote(table), quote(name));
    let sort_column = sort
        .map(|sort| find_column(columns, &sort.column))
        .transpose()?;
    let mut terms = Vec::with_capacity(primary_indices.len() + 1);
    if let (Some(sort), Some(column)) = (sort, sort_column) {
        if !column.sortable {
            return Err(format!("Column {} cannot be sorted.", column.name));
        }
        terms.push(format!(
            "{} {}",
            order_column(&column.name),
            if sort.descending { "DESC" } else { "ASC" }
        ));
    }
    for index in primary_indices {
        let column = &columns[*index];
        if sort_column.is_none_or(|sort| sort.name != column.name) {
            terms.push(format!("{} ASC", order_column(&column.name)));
        }
    }
    if terms.is_empty()
        && let Some(column) = columns.iter().find(|column| column.sortable)
    {
        terms.push(format!("{} ASC", order_column(&column.name)));
    }
    if terms.is_empty() {
        return Err("Table has no sortable columns.".into());
    }
    Ok(terms.join(", "))
}

pub fn filter_kind(type_name: &str) -> FilterKind {
    let normalized = type_name.to_ascii_lowercase();
    if normalized.contains("bool")
        || normalized == "bit"
        || normalized == "bit(1)"
        || normalized.contains("tinyint(1)")
    {
        FilterKind::Boolean
    } else if normalized.contains("date")
        || normalized.contains("time")
        || normalized.contains("year")
        || normalized.contains("interval")
    {
        FilterKind::Temporal
    } else if normalized.contains("char")
        || normalized.contains("text")
        || normalized.contains("clob")
        || normalized.contains("string")
        || normalized.contains("json")
        || normalized.contains("enum")
        || normalized.contains("set")
        || normalized.contains("xml")
        || normalized.contains("uuid")
    {
        FilterKind::Text
    } else if (!normalized.contains("point") && normalized.contains("int"))
        || normalized.contains("decimal")
        || normalized.contains("numeric")
        || normalized.contains("number")
        || normalized.contains("float")
        || normalized.contains("double")
        || normalized.contains("real")
        || normalized.contains("money")
        || normalized.contains("serial")
    {
        FilterKind::Number
    } else {
        FilterKind::Other
    }
}

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub fn quote_mysql_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

pub fn quote_mssql_identifier(identifier: &str) -> String {
    format!("[{}]", identifier.replace(']', "]]"))
}

pub fn quote_relation(schema: &str, table: &str, quote: impl Fn(&str) -> String) -> String {
    format!("{}.{}", quote(schema), quote(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, primary_key: bool, sortable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: "text".into(),
            filter_type: "text".into(),
            filter_kind: FilterKind::Text,
            sortable,
            nullable: true,
            primary_key,
            generated: false,
            identity: false,
        }
    }

    #[test]
    fn query_budget_caps_cells_and_total() {
        let mut budget = QueryBudget::new();
        let value = "a".repeat(MAX_CELL_CHARS + 10);
        let capped = budget.cell(Some(value)).expect("cell");
        assert!(capped.ends_with(" [truncated]"));
        assert!(capped.len() <= MAX_CELL_CHARS + " [truncated]".len());
        assert!(budget.truncated());
    }

    #[test]
    fn query_budget_drops_cells_after_exhaustion() {
        let mut budget = QueryBudget {
            remaining: 0,
            truncated: false,
        };
        assert_eq!(budget.cell(Some("value".into())), None);
        assert_eq!(budget.column("name".into()), "");
        assert!(budget.truncated());
    }

    #[test]
    fn query_budget_preserves_empty_string_when_slot_fits_exactly() {
        let slot = std::mem::size_of::<Option<String>>();
        let mut budget = QueryBudget {
            remaining: slot,
            truncated: false,
        };
        assert_eq!(
            budget.page_cell_flagged(Some(String::new())),
            (Some(String::new()), false)
        );
        assert!(!budget.truncated());

        let mut str_budget = QueryBudget {
            remaining: slot,
            truncated: false,
        };
        assert_eq!(str_budget.cell_str(Some("")), Some(String::new()));
        assert!(!str_budget.truncated());
    }

    #[test]
    fn query_budget_charges_slots_and_rejects_after_exhaustion() {
        let slot = std::mem::size_of::<Option<String>>();
        let mut budget = QueryBudget {
            remaining: slot * 2 + 2,
            truncated: false,
        };
        assert_eq!(
            budget.page_cell_flagged(Some("ab".into())),
            (Some("ab".into()), false)
        );
        assert_eq!(budget.page_cell_flagged(Some("cd".into())), (None, true));
        assert_eq!(budget.page_cell_flagged(None), (None, true));
        assert!(budget.truncated());
    }

    #[test]
    fn query_budget_stops_after_total_budget() {
        let slot = std::mem::size_of::<Option<String>>();
        let mut budget = QueryBudget {
            remaining: (slot + 4) * 2,
            truncated: false,
        };
        assert_eq!(budget.cell(Some("1234".into())), Some("1234".into()));
        assert_eq!(budget.cell(Some("5678".into())), Some("5678".into()));
        assert_eq!(budget.cell(Some("9".into())), None);
        assert!(budget.truncated());
    }

    #[test]
    fn null_cells_still_consume_structural_budget() {
        let slot = std::mem::size_of::<Option<String>>();
        let mut budget = QueryBudget {
            remaining: slot * 2,
            truncated: false,
        };
        assert_eq!(budget.cell(None), None);
        assert_eq!(budget.cell(None), None);
        assert_eq!(budget.cell(None), None);
        assert!(budget.truncated());
    }

    #[test]
    fn keyed_order_by_skips_unsortable_first_column_on_keyless_tables() {
        let columns = vec![column("body", false, false), column("title", false, true)];
        let order =
            build_keyed_order_by(&columns, &[], None, "docs", quote_identifier).expect("order by");
        assert_eq!(order, "\"docs\".\"title\" ASC");

        let unsortable = vec![column("body", false, false)];
        assert!(build_keyed_order_by(&unsortable, &[], None, "docs", quote_identifier).is_err());
    }

    #[test]
    fn filter_kind_handles_interval_point_bit_and_enum() {
        assert_eq!(filter_kind("INTERVAL DAY TO SECOND"), FilterKind::Temporal);
        assert_eq!(filter_kind("point"), FilterKind::Other);
        assert_eq!(filter_kind("multipoint"), FilterKind::Other);
        assert_eq!(filter_kind("bit(1)"), FilterKind::Boolean);
        assert_eq!(filter_kind("tinyint(1)"), FilterKind::Boolean);
        assert_eq!(filter_kind("enum('a','b')"), FilterKind::Text);
        assert_eq!(filter_kind("bigint"), FilterKind::Number);
    }

    #[test]
    fn quoting_escapes_embedded_delimiters() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_mysql_identifier("a`b"), "`a``b`");
        assert_eq!(quote_mssql_identifier("a]b"), "[a]]b]");
    }
}
