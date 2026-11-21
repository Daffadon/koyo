# ADR 0003: Multi-connection tabs and tab-owned sessions

- Status: Accepted
- Date: 2026-11-08
- Amended: 2026-11-16

## Amendment: pane-shared sessions

Split panes now duplicate the active tab onto the same live session. New SQL tabs and table tabs opened in a focused pane reuse that pane's live session instead of connecting again. Sessions are reference-counted by tabs; the last tab that holds a session closes it.

This intentionally trades transaction isolation between tabs for continuity when splitting. A future transaction UI must account for shared session state.

## Amendment: parked sessions

Closing the last tab that holds a clean session no longer disconnects it. The session is parked in a bounded idle connection pool and reused by the next workspace tab; tab lifetime and connection lifetime are independent. The pool keeps at most four idle sessions and at most one per profile, so a later tab cannot inherit another profile's session state. Explicit disconnect, profile deletion, or app exit closes a parked session. Busy sessions are still disconnected when their last tab closes so a closed query cannot continue unattended.

## Context

Koyo currently has one active PostgreSQL session, one cancel token, one query view, and one table view. The next increment adds query tabs, table tabs, and multiple open connections.

An interactive database client needs session continuity. A transaction opened in one command must stay on the same connection for later commands. Temporary tables, session settings, prepared statements, and cancellation all depend on a stable session.

A normal connection pool checks out a connection per command. That breaks all of the above. Reusing a connection across tabs also leaks session state between unrelated work.

## Decision

### Sessions

- Each workspace tab owns one dedicated database session for its lifetime.
- A new tab opened from a profile connects a new session, even when another tab already uses the same profile.
- Closing a tab disconnects its session. There is no connection reuse across tabs.
- Do not add a per-command connection pool. Each live tab keeps one dedicated database connection so transactions, temporary tables, session settings, and cancellation stay on one session. Idle connections are pooled after the last holding tab closes.
- Keep sessions in a Rust map keyed by session ID. Database commands take the session ID.
- The engine-first connection flow is defined in ADR 0005; its session cap was removed on 2026-11-14.
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

- Any number of sessions may be live; disconnected tabs can remain and preserve their SQL.
- Each tab has independent transaction and cancellation behavior.
- The Rust state and command signatures change from one session to a session map.
- The UI state changes from one workspace view to tab arrays.
- No pool dependency is added.

## Alternatives considered

- Share one active connection across all tabs: simpler, but switching connections affects every tab and transactions interfere.
- Pool checked out per command: fast, but breaks transactions, temp state, and cancellation.
- Bounded lease pool: equivalent to tab-owned sessions but adds a dependency and a failure mode.
- Restore tabs on restart: useful, but needs persisted tab state and reconnect policy; deferred.

## References

- [PostgreSQL session state](https://www.postgresql.org/docs/current/runtime-config-client.html)
- [`tokio-postgres` cancellation](https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.CancelToken.html)
