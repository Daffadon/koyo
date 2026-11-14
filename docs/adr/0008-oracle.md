# ADR 0008: Oracle support (beta)

- Status: Accepted
- Date: 2026-11-08

## Context

Oracle is the last engine in the ADR 0004 order. Oracle support is marked beta in the UI because it uses the official `oracledb` pure-Rust thin driver, which is still pre-1.0.

## Decision

- Use Oracle's maintained `oracledb` crate. It needs no Oracle Instant Client and no C toolchain.
- Mark Oracle as beta in the engine picker and profile list.
- Use Easy Connect strings. `verify-full` uses `tcps://host:port/service`; `disable` uses plain TCP. The default port is 1521.
- Discover relations from `ALL_TABLES`, `ALL_TAB_COLUMNS`, `ALL_CONSTRAINTS`, and `ALL_CONS_COLUMNS`; exclude Oracle-maintained schemas.
- Render common scalar values as text. CLOB, BLOB, BFILE, JSON, XMLTYPE, and OBJECT cells show a bracketed type placeholder until LOB streaming is implemented.
- Filter with `INSTR`/`LOWER` for contains and direct comparisons for ordered values.
- Sort by the requested column, then primary-key columns; page with `OFFSET ... FETCH NEXT`.
- Require a primary key for editing. Apply all changes in one transaction and roll back on failure.
- Detect stale rows by matching every originally loaded value with null-safe predicates.
- Detect open transactions with `DBMS_TRANSACTION.LOCAL_TRANSACTION_ID`.
- Query execution uses `query` for result sets and `execute` for DML/PLSQL, with affected-row counts from the driver.
- Cancellation is not available in the beta driver. The UI reports the limitation instead of pretending to cancel.

## Consequences

- Oracle ships without a native client dependency.
- Cancel is a documented beta gap; running Oracle statements must finish or lose the tab/session.
- LOB and object display is deferred; the text contract stays consistent with other engines.
- The driver, profile, and SQL surface may change while `oracledb` remains pre-1.0.

## Alternatives considered

- OCI `oracle` crate: mature cancellation and LOB support, but requires Oracle Instant Client on every machine.
- ODBC: still requires an Oracle driver and adds native packaging.
