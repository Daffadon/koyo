# ADR 0006: MySQL and MariaDB support

- Status: Accepted
- Date: 2026-11-08

## Context

ADR 0004 fixes the engine boundary and asks for one ADR per later engine. MySQL and MariaDB share a wire protocol and most catalog behavior, but users expect both names in the engine picker. Full workflow parity means connect, SQL, catalog, filter/sort/page, staged edits, and cancel.

## Decision

- Use `mysql_async` with rustls for both engines. MariaDB is a separate profile engine that selects the same driver.
- Store host, port, database, username, TLS mode, and password like PostgreSQL. The default port is 3306.
- TLS mode `verify-full` requires TLS and validates the server certificate against the driver's root store. `disable` sends no TLS and stays a local-development option.
- Use `information_schema` for table and column metadata. System schemas are excluded.
- Render cell values as text with `CAST(column AS CHAR)`. BLOB-heavy display remains a documented text-view limitation.
- Filter with `LOCATE` for contains, direct comparisons for ordered values, and `IS NULL`/`IS NOT NULL`.
- Sort by the requested column, then primary-key columns. A keyless table falls back to its first column so paging is deterministic enough to use.
- Require a primary key for editing. Apply all staged changes in one transaction.
- Detect stale rows by matching every originally loaded value through the same text conversion used for display and MySQL's null-safe `<=>`.
- Cancel with a second connection issuing `KILL QUERY <connection-id>`.
- Detect a protected open transaction from tracked `BEGIN`/`START TRANSACTION`/`COMMIT`/`ROLLBACK`, `@@autocommit`, `@@in_transaction` where MariaDB exposes it, and `information_schema.innodb_trx` as a fallback.
- Keep one live session per tab. The five-session cap from ADR 0005 was removed on 2026-11-14.

## Consequences

- MySQL and MariaDB use the same module and edit semantics; only the profile engine label and default catalog assumptions differ.
- `KILL QUERY` needs the connected user to have permission to kill its own query; cancellation reports the server error when it does not.
- Unix sockets, SSH tunnels, custom certificate files, and binary-value editing are deferred.

## Alternatives considered

- SQLx: useful for MySQL, but adds a second query layer without helping SQL Server or Oracle.
- Per-engine placeholder classes: rejected; explicit dispatch remains smaller.
