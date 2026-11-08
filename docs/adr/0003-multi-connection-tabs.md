# ADR 0003: Multi-connection tabs and tab-owned sessions

- Status: Accepted
- Date: 2026-11-08

## Context

Koyo currently has one active PostgreSQL session, one cancel token, one query view, and one table view. The next increment adds query tabs, table tabs, and multiple open connections.

An interactive database client needs session continuity. A transaction opened in one command must stay on the same connection for later commands. Temporary tables, session settings, prepared statements, and cancellation all depend on a stable session.

A normal connection pool checks out a connection per command. That breaks all of the above. Reusing a connection across tabs also leaks session state between unrelated work.

## Decision

### Sessions

- Each workspace tab owns one dedicated database session for its lifetime.
- A new tab opened from a profile connects a new session, even when another tab already uses the same profile.
- Closing a tab disconnects its session. There is no connection reuse across tabs.
- Do not add a connection pool. A pool could only cap tab-owned leases, not reuse them safely; the plain session map is smaller.
- Keep sessions in a Rust map keyed by session ID. Database commands take the session ID.
- Keep query-active and cancel state per session. Cancelling one tab must not cancel another tab's work.
- Multiple sessions may target the same profile. The profile is a template, not a live connection.

### Tabs

- Support query tabs and table tabs.
- A query tab owns: SQL text, running state, result sets, error, duration, and its session ID.
- A table tab owns: relation, page, filters, sort, staged changes, load sequence, and its session ID.
- Keep one active tab. The explorer follows the active tab's session.
- Opening a tab from a profile connects and activates it.
- No tab restore across app restarts.
- Closing a tab with staged edits requires confirmation, then discards the edits and disconnects.
- Closing a connection profile closes every tab whose session uses it.

### Failure handling

- A failed connect leaves the new tab disconnected and shows the error.
- A failed query or table load affects only its tab.
- A stale page response cannot overwrite a newer response in the same tab.
- A tab that loses its session shows a disconnected state and offers reconnect.

## Consequences

- Many tabs mean many database connections. The user controls this by closing tabs; a cap can be added later if needed.
- Each tab has independent transaction and cancellation behavior.
- The Rust state and command signatures change from one session to a session map.
- The frontend state changes from one workspace view to tab arrays.
- No pool dependency is added.

## Alternatives considered

- Share one active connection across all tabs: simpler, but switching connections affects every tab and transactions interfere.
- Pool checked out per command: fast, but breaks transactions, temp state, and cancellation.
- Bounded lease pool: equivalent to tab-owned sessions but adds a dependency and a failure mode.
- Restore tabs on restart: useful, but needs persisted tab state and reconnect policy; deferred.

## References

- [PostgreSQL session state](https://www.postgresql.org/docs/current/runtime-config-client.html)
- [`tokio-postgres` cancellation](https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.CancelToken.html)
