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
