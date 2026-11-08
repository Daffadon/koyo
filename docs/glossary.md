# Glossary

- **Connection profile**: Saved PostgreSQL host, port, database, username, and TLS settings. Password is stored separately in the operating system credential store.
- **Active connection**: The currently selected profile with an open PostgreSQL session.
- **SQL workbench**: SQL editor that runs statements with the connected PostgreSQL role's permissions and displays returned rows or command status.
- **Schema browser**: Navigation through PostgreSQL schemas and regular tables.
- **Table preview**: A paged view of table rows. Rows are editable only when the table has a primary key.
- **Staged change**: An insert, update, or delete held in the UI until the user applies or discards it.
- **Apply**: Execute all staged table changes in one PostgreSQL transaction. A failure rolls back the batch; an uncertain commit requires reloading before retry.
- **Primary key**: Database-declared columns that identify one row. Required for safe grid updates and deletes.
- **Keyless table**: Table without a primary key. Its preview is read-only.
- **Credential store**: Platform facility used for saved passwords: Windows Credential Manager, macOS Keychain, or Linux Secret Service.
- **TLS verification**: Validation that a TLS connection reaches the intended PostgreSQL server, not only that traffic is encrypted.
- **Relation**: A table, partitioned table, view, or materialized view shown in the explorer.
- **Read-only relation**: A view, materialized view, or keyless table. Its data grid has no staged edits.
- **Filter kind**: Column classification that determines which filter operators are available: text, number, temporal, boolean, or other.
- **Table filter**: A per-column condition applied server-side before paging. Active filters combine with AND.
- **Sort state**: One sort column and direction. Primary-key columns, or `ctid` for keyless tables, break ties.
- **Structure view**: Read-only metadata for a relation: columns, indexes, constraints, and object definitions.
- **Object definition**: PostgreSQL-generated SQL for a view, index, or constraint. It is not a reconstructed full `CREATE TABLE` statement.
