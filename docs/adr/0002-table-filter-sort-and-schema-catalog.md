# ADR 0002: Table filters, sorting, and read-only schema catalog

- Status: Accepted
- Date: 2026-11-08

## Delivery

Ship server-side filtering and single-column sorting first. Add the read-only schema catalog and Structure tab in a separate increment.

## Context

The table tab currently loads one page at a time with an unfiltered `LIMIT/OFFSET` query. It can only order by the primary key, or by `ctid` for keyless tables. The explorer only lists regular and partitioned tables.

The next increment adds two connected capabilities:

- Find and order rows in a table or view.
- Browse views, materialized views, indexes, and constraints without changing the schema.

The design constraints are:

- Filters must apply to the whole relation, not only the current page.
- Generated SQL must use known columns and parameterized values.
- Existing staged-edit safety must not regress.
- The schema catalog remains read-only.

## Decision

### Filtering

- Put a filter row under the column headers. Each column has at most one filter.
- Apply filters and sort to rows in the open relation. The explorer object list is not searchable in this increment.
- Combine active filters with `AND`. Do not add `OR` groups in this increment.
- Provide an explicit **Apply filters** action; pressing Enter in a filter input also applies. Do not query on every keystroke.
- Reset the page offset to zero when filters or sort change.
- Classify columns into `text`, `number`, `temporal`, `boolean`, and `other` from PostgreSQL catalog types.
- Operators:
  - Text: contains, equals, not equals.
  - Number and temporal: equals, not equals, less than, less than or equal, greater than, greater than or equal.
  - Boolean: true, false.
  - All types: is null, is not null.
  - `other` types support only is null and is not null; use the SQL workbench for other predicates.
- Contains is a case-insensitive literal substring match. Equals and not equals use PostgreSQL equality.
- Send filter values as query parameters and cast them to the column type in SQL. Validate filter columns against loaded metadata. Never interpolate filter values into SQL.
- Treat an empty filter input as inactive. Use the SQL workbench to match empty strings explicitly.
- If PostgreSQL rejects a filter value during type casting, show the error and keep the current page and filter inputs. Do not clear the grid.
- Do not show a total matching-row count. Continue to page with `LIMIT/OFFSET` and `hasMore`.

### Sorting

- Support one sort column at a time.
- Clicking a column header cycles ascending, descending, then no explicit sort.
- Append primary-key columns as tiebreakers. For keyless tables, append `ctid`. Views have no stable tiebreaker.
- Reset the page offset to zero when the sort changes.
- Filters and sort are in-memory UI state for the open relation. They reset when the relation closes or the connection changes.

### Staged edits

- While staged table changes exist, disable filter and sort controls. Require the user to apply or discard the batch first.
- Do not hide staged rows behind a filter and do not silently discard edits.
- Views and materialized views are always read-only and never have staged edits.

### Schema catalog

- Include regular tables, partitioned tables, views, and materialized views in the explorer.
- Do not include sequences, functions, types, triggers, or foreign tables in this increment.
- Do not add a search box for the explorer object list in this increment.
- Opening a view or materialized view shows a read-only data grid with filtering, sorting, and paging.
- Add a **Structure** tab for the selected relation. It shows:
  - Columns: name, type, nullability, default, generated/identity, primary key.
  - Indexes: name, uniqueness, primary-key flag, definition.
  - Constraints: name, type, definition.
  - View or materialized-view SQL.
- Use PostgreSQL-generated definitions (`pg_get_viewdef`, `pg_get_indexdef`, `pg_get_constraintdef`) and allow copying them.
- Do not reconstruct a full `CREATE TABLE` statement. Do not execute DDL from the structure view.

### Backend

- Extend relation metadata with relation kind and a filter kind per column.
- Add a relation-list command that returns tables, partitioned tables, views, and materialized views.
- Add a relation-page command that accepts an optional filter list and sort state.
- Use the extended query protocol for filtered pages so values remain parameters. Select column values as text to preserve the existing grid contract.
- Do not select `xmin` for views or materialized views; they are read-only and need no row version.
- Add a structure command that returns columns, indexes, constraints, and object definitions for one relation.

## Consequences

- Filters and sort are server-side and work across pages.
- The SQL builder needs a type-aware filter model; arbitrary predicates remain available through the SQL workbench.
- Contains filtering scans the relation and cannot use a normal B-tree index.
- Offset paging can skip or repeat rows when data changes concurrently. This limitation already exists and is not solved here.
- Views have no stable default order, so paging through a view without a sort is best-effort.
- No total row count avoids expensive `COUNT(*)` queries.
- Invalid filter values produce a PostgreSQL cast error without replacing the current page.
- The structure view is descriptive, not a schema editor.
- Filtering and sorting ship before the schema catalog; the catalog decisions remain accepted but deferred.
- Full `CREATE TABLE` reconstruction and schema mutation remain out of scope.

## Alternatives considered

- Client-side filtering and sorting: only works on the loaded page; rejected.
- Text-casting every column: simple, but numeric and temporal ordering become wrong.
- A raw SQL `WHERE` box: powerful, but bypasses validation and duplicates the workbench.
- Filter groups with `OR`: more UI and a more complex SQL builder; deferred.
- Multi-column sorting and keyset paging: better for large, changing tables; deferred.
- Schema editing: destructive and out of scope.
- A full object explorer: larger catalog and permission surface; deferred.

## References

- [PostgreSQL catalogs](https://www.postgresql.org/docs/current/catalogs.html)
- [`pg_get_viewdef`](https://www.postgresql.org/docs/current/functions-info.html#FUNCTIONS-INFO-CATALOG-TABLE)
- [`pg_get_indexdef`](https://www.postgresql.org/docs/current/functions-info.html#FUNCTIONS-INFO-CATALOG-TABLE)
- [`pg_get_constraintdef`](https://www.postgresql.org/docs/current/functions-info.html#FUNCTIONS-INFO-CATALOG-TABLE)
- [`tokio-postgres` query API](https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html#method.query)
