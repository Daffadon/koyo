# ADR 0004: Engine boundary and SQLite support

- Status: Accepted
- Date: 2026-11-08

## Implementation note

The engine boundary and SQLite landed before ADR 0003 tabs existed. They currently use the single-session UI and a `DbSession` enum. ADR 0003 will replace that state with tab-owned sessions; engine dispatch carries forward.

## Context

Koyo is PostgreSQL-specific: `tokio-postgres`, `pg_catalog` metadata, `$n::type` casts, quoted identifiers, `xmin` optimistic locking, and `ctid` paging.

The requested engine order is SQLite, MySQL/MariaDB, SQL Server, then Oracle. “Full parity” is defined as **workflow parity**, not identical SQL behavior: connect, run SQL, browse relations, filter/sort, page, stage edits, apply with an engine-specific stale check, and cancel. The read-only schema catalog and Structure tab from ADR 0002 remain a separate increment.

This decision supersedes the ADR 0001 clause that other database engines are out of scope. It does not add SSH tunnels or client certificates.

## Decision

### Engine boundary

- Add an `engine` field to connection profiles. Existing profiles without the field load as PostgreSQL.
- Introduce an `EngineSession` enum with `Postgres` and `Sqlite` variants and shared command methods. Use explicit dispatch, not an async trait object, for this increment. Reassess at the third engine.
- Keep shared result and metadata types where behavior matches. Add engine capabilities for features that differ, such as multiple statements or edit support.
- Engine-specific modules own SQL generation, identifiers, metadata queries, paging, and conflict checks.
- Tab sessions from ADR 0003 remain one connection per tab. The engine is a property of the session.

### SQLite profile

- SQLite profiles use a file path instead of host, port, database, username, and TLS.
- SQLite has no password and does not touch the credential store.
- Add a native file picker for SQLite files.
- Allow creating a new database file. Do not add an in-memory mode to the UI; tests may use `:memory:`.
- Do not change journal mode or other persistent database settings automatically. Set a busy timeout only.

### SQLite behavior

- Use `rusqlite` with bundled SQLite. Run blocking calls on a blocking worker and keep one connection per tab session.
- Use `main` as the only schema in this increment. Attached databases are not listed.
- List tables from `sqlite_schema`; read columns and primary keys from pragmas.
- Support one SQL statement per run. SQLite scripts and multiple result sets are a documented workflow gap.
- Support per-column filters and single-column sorting with SQLite identifiers and `?` parameters.
- Require a primary key for staged edits. Keyless tables remain read-only.
- Apply staged edits in one transaction.
- Detect stale rows by matching the primary key and every originally loaded column value with SQLite null-safe `IS` comparisons. If any row matches zero rows, roll back the batch.
- Use SQLite interrupt handling for cancellation.
- Show values as text, matching the existing grid contract. BLOB display and binary editing are deferred.
- Do not add a SQLite Structure tab until ADR 0002 is implemented.

### Order

1. Multi-connection tabs on PostgreSQL (ADR 0003).
2. Engine boundary and SQLite workflow parity.
3. MySQL and MariaDB through one driver.
4. SQL Server.
5. Oracle last, only with a real user requirement; it needs Oracle Instant Client/OCI and distribution review.

Each later engine gets its own ADR for auth, TLS, metadata, and editing semantics.

## Consequences

- The profile schema and connection dialog become engine-aware.
- Command handlers take a session ID and dispatch by engine.
- PostgreSQL behavior must remain unchanged.
- SQLite cannot match PostgreSQL multi-statement or binary-value behavior in this increment.
- SQLite conflict checks compare all original values, which is safe but generates larger predicates.
- Adding later engines is still real work; this ADR only creates the boundary.

## Alternatives considered

- Port the whole app to SQLx: useful for PostgreSQL, MySQL, and SQLite, but a large rewrite and no SQL Server/Oracle support.
- Add an async trait object now: cleaner for many engines, but adds a dependency and complexity before the second engine exists.
- Per-command connections: smaller memory use, but breaks session and transaction continuity.
- Multi-engine in one increment: too large to review or roll back safely.

## References

- [`rusqlite`](https://docs.rs/rusqlite/latest/rusqlite/)
- [SQLite transaction behavior](https://www.sqlite.org/lang_transaction.html)
- [SQLite pragma functions](https://www.sqlite.org/pragma.html)
- [SQLite interrupt](https://www.sqlite.org/c3ref/interrupt.html)
