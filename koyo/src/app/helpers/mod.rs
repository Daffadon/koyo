mod export;
mod filter;
mod sql;
mod ui;

pub(crate) use export::*;
pub(crate) use filter::*;
pub(crate) use sql::*;
pub(crate) use ui::*;

#[cfg(test)]
mod tests {
    use koyo_core::models::{ColumnInfo, FilterKind, FilterOperator, QueryResultSet};

    use super::{
        default_export_path, filter_suggestions, format_sql, insert_filter_suggestion,
        object_definition_sql, parse_filter_input, resolve_export_pages, results_as_csv,
        results_as_json, table_csv_file_name, table_ddl, table_page_as_csv, table_preview_sql,
    };
    use crate::app::ExportScope;
    use koyo_core::models::{Engine, ForeignKeyInfo, SchemaObjectKind};

    fn ddl_column(name: &str, data_type: &str, primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            filter_type: data_type.into(),
            filter_kind: FilterKind::Other,
            sortable: true,
            nullable: !primary_key,
            primary_key,
            generated: false,
            identity: false,
        }
    }

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
            "value\n\"'=HYPERLINK(\"\"https://example.test\"\")\"\n-12.5\n\"'-1+2\"\n\"'@cmd\"\n"
        );
    }

    #[test]
    fn csv_formula_guard_keeps_delimiters_inside_quotes() {
        let result = QueryResultSet {
            columns: vec!["value".into()],
            rows: vec![vec![Some("=1,=cmd|' /C calc'!A0,x".into())]],
        };
        assert_eq!(
            results_as_csv(&result),
            "value\n\"'=1,=cmd|' /C calc'!A0,x\"\n"
        );

        let result = QueryResultSet {
            columns: vec!["value".into()],
            rows: vec![vec![Some("=1\n=cmd|' /C calc'!A0\nx".into())]],
        };
        assert_eq!(
            results_as_csv(&result),
            "value\n\"'=1\n=cmd|' /C calc'!A0\nx\"\n"
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
    fn json_result_keeps_duplicate_column_values() {
        let result = QueryResultSet {
            columns: vec!["id".into(), "id".into()],
            rows: vec![vec![Some("left".into()), Some("right".into())]],
        };
        let json = results_as_json(&result);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed[0]["id"], "left");
        assert_eq!(parsed[0]["id_2"], "right");
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

        assert_eq!(table_page_as_csv(&page, true), "name\n\"'=cmd\"\n");
        assert_eq!(table_page_as_csv(&page, false), "\"'=cmd\"\n");
    }

    #[test]
    fn table_export_file_name_stays_a_file_name() {
        assert_eq!(table_csv_file_name("main", "people"), "main.people.csv");
        assert_eq!(table_csv_file_name("a/b", "c\\d"), "a_b.c_d.csv");
    }

    #[test]
    fn table_ddl_includes_columns_primary_key_and_foreign_key() {
        let columns = vec![
            ddl_column("id", "integer", true),
            ddl_column("owner_id", "integer", false),
            ddl_column("name", "text", false),
        ];
        let foreign_keys = vec![ForeignKeyInfo {
            schema: "public".into(),
            table: "orders".into(),
            column: "owner_id".into(),
            referenced_schema: "public".into(),
            referenced_table: "users".into(),
            referenced_column: "id".into(),
            constraint_name: "orders_owner_fk".into(),
        }];
        let ddl = table_ddl(
            Engine::Postgres,
            "public",
            "orders",
            &columns,
            &foreign_keys,
        );
        assert!(ddl.contains("CREATE TABLE \"public\".\"orders\" ("));
        assert!(ddl.contains("\"id\" integer NOT NULL"));
        assert!(ddl.contains("PRIMARY KEY (\"id\")"));
        assert!(ddl.contains(
            "CONSTRAINT \"orders_owner_fk\" FOREIGN KEY (\"owner_id\") REFERENCES \"public\".\"users\" (\"id\")"
        ));

        let incoming = vec![ForeignKeyInfo {
            schema: "public".into(),
            table: "users".into(),
            column: "id".into(),
            referenced_schema: "public".into(),
            referenced_table: "orders".into(),
            referenced_column: "owner_id".into(),
            constraint_name: "users_orders_fk".into(),
        }];
        let ddl = table_ddl(Engine::Postgres, "public", "orders", &columns, &incoming);
        assert!(!ddl.contains("FOREIGN KEY"));
        assert!(!ddl.contains(",\n);"));
    }

    #[test]
    fn table_ddl_quotes_mysql_identifiers_with_backticks() {
        let columns = vec![ddl_column("id", "bigint", true)];
        let ddl = table_ddl(Engine::Mysql, "app", "items", &columns, &[]);
        assert!(ddl.contains("CREATE TABLE `app`.`items` ("));
        assert!(ddl.contains("`id` bigint NOT NULL"));
    }

    #[test]
    fn table_preview_sql_uses_the_right_limit_syntax() {
        assert_eq!(
            table_preview_sql(Engine::Postgres, "public", "users"),
            "SELECT * FROM \"public\".\"users\" LIMIT 100;"
        );
        assert_eq!(
            table_preview_sql(Engine::Mysql, "app", "users"),
            "SELECT * FROM `app`.`users` LIMIT 100;"
        );
        assert_eq!(
            table_preview_sql(Engine::SqlServer, "dbo", "users"),
            "SELECT TOP 100 * FROM [dbo].[users];"
        );
        assert_eq!(
            table_preview_sql(Engine::Oracle, "APP", "USERS"),
            "SELECT * FROM \"APP\".\"USERS\" FETCH FIRST 100 ROWS ONLY;"
        );
    }

    #[test]
    fn default_export_path_keeps_the_file_name() {
        assert!(default_export_path("main.people.csv").ends_with("main.people.csv"));
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
    fn filter_completion_suggests_columns_operators_and_inserts() {
        let columns = vec![
            filter_column("name", FilterKind::Text),
            filter_column("age", FilterKind::Number),
            filter_column("active", FilterKind::Boolean),
        ];

        let names = filter_suggestions(&columns, "na", 2);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].label, "name");

        let operators = filter_suggestions(&columns, "name co", 7);
        assert_eq!(operators[0].label, "contains");

        let and_names = filter_suggestions(&columns, "age > 1 AND na", 14);
        assert_eq!(and_names[0].label, "name");

        let booleans = filter_suggestions(&columns, "active ", 7);
        assert_eq!(booleans[0].label, "is true");

        let (value, cursor) = insert_filter_suggestion("name co", 7, "contains ");
        assert_eq!(value, "name contains ");
        assert_eq!(cursor, value.len());
    }

    #[test]
    fn formats_sql_with_uppercase_and_two_space_indent() {
        let formatted = format_sql(Engine::Sqlite, "select id, name from users where id = 1");
        assert_eq!(
            formatted,
            "SELECT\n  id,\n  name\nFROM\n  users\nWHERE\n  id = 1"
        );
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
    fn filter_input_handles_non_ascii_identifiers_and_values() {
        let columns = vec![filter_column("naïve", FilterKind::Text)];
        let filters = parse_filter_input(&columns, "naïve = café").expect("unicode filter");
        assert_eq!(filters[0].column, "naïve");
        assert_eq!(filters[0].value.as_deref(), Some("café"));
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
    #[test]
    fn object_definition_sql_quotes_literals_and_identifiers() {
        let postgres = object_definition_sql(
            Engine::Postgres,
            SchemaObjectKind::Function,
            "app",
            "get'user",
        );
        assert!(postgres.contains("n.nspname = 'app'"));
        assert!(postgres.contains("p.proname = 'get''user'"));
        assert!(postgres.contains("pg_get_functiondef"));

        let mysql = object_definition_sql(
            Engine::Mysql,
            SchemaObjectKind::Trigger,
            "odd`schema",
            "odd`name",
        );
        assert_eq!(mysql, "SHOW CREATE TRIGGER `odd``schema`.`odd``name`;");

        let sqlserver = object_definition_sql(
            Engine::SqlServer,
            SchemaObjectKind::Function,
            "sales]data",
            "total]",
        );
        assert!(sqlserver.contains("OBJECT_ID(N'[sales]]data].[total]]]')"));

        let oracle =
            object_definition_sql(Engine::Oracle, SchemaObjectKind::Trigger, "app", "audit");
        assert!(oracle.contains("owner = UPPER('app')"));
        assert!(oracle.contains("name = UPPER('audit')"));
        assert!(oracle.contains("type = 'TRIGGER'"));

        let sqlite =
            object_definition_sql(Engine::Sqlite, SchemaObjectKind::Trigger, "main", "audit");
        assert!(sqlite.contains(r#"FROM "main".sqlite_master"#));
        assert!(sqlite.contains("name = 'audit'"));

        assert!(
            object_definition_sql(Engine::Postgres, SchemaObjectKind::Table, "public", "users")
                .is_empty()
        );
    }
}
