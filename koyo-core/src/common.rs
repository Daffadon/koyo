use crate::models::{ColumnInfo, FilterKind, FilterOperator};

pub const QUERY_ROW_LIMIT: usize = 1_000;
pub const TABLE_PAGE_LIMIT: u32 = 200;

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

pub fn ensure_writable(column: &ColumnInfo) -> Result<(), String> {
    if !column.generated && !column.identity {
        return Ok(());
    }
    Err(format!("Column {} cannot be edited.", column.name))
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

pub fn filter_kind(type_name: &str) -> FilterKind {
    let normalized = type_name.to_ascii_lowercase();
    if normalized.contains("bool") || normalized == "bit" || normalized.contains("tinyint(1)") {
        FilterKind::Boolean
    } else if normalized.contains("int")
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
    } else if normalized.contains("date")
        || normalized.contains("time")
        || normalized.contains("year")
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
