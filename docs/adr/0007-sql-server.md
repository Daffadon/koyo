# ADR 0007: SQL Server support

- Status: Accepted
- Date: 2026-11-08

## Context

SQL Server uses TDS rather than the PostgreSQL or MySQL protocols. The existing session and tab rules still apply; the session cap was removed on 2026-11-14.

## Decision

- Use `tiberius` with its rustls backend. SQL authentication is the first auth mode; Windows integrated auth is deferred.
- Reuse profile host, port, database, username, TLS mode, and password. The default port is 1433.
- `verify-full` requests required encryption and validates the server certificate against the platform trust store. `disable` disables encryption for trusted local instances.
- Discover relations from `sys.tables`, `sys.schemas`, `sys.columns`, `sys.types`, and `sys.indexes`.
- Render values as text with `CONVERT(nvarchar(max), column)`. Binary columns use hexadecimal conversion.
- Filter with `CHARINDEX` for contains and direct comparisons for ordered values.
- Sort by the requested column, then primary-key columns; use `OFFSET ... FETCH NEXT` for paging.
- Require a primary key for editing. Apply all changes in one TDS transaction.
- Detect stale rows by matching every originally loaded value with null-safe `(column = @P OR (column IS NULL AND @P IS NULL))` predicates.
- Cancel by opening a second connection and issuing `KILL <spid>`.
- Protect open transactions with `@@TRANCOUNT`.

## Consequences

- SQL Server certificate trust follows the OS root store through rustls-native-certs.
- SQL authentication is complete; Kerberos/Windows authentication needs a separate increment and packaging review.
- Cancellation issues `KILL <spid>`, which terminates the session rather than only the statement; the tab needs reconnect before further work. It requires permission to kill the session's own SPID.
- LOB and XML values follow the text-display contract; binary editing remains deferred.

## Alternatives considered

- ODBC: depends on installed drivers and loses protocol-level type control.
- SQLx MSSQL: wraps tiberius and would not remove the need for TDS-specific text conversion.
