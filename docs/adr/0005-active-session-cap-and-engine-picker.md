# ADR 0005: Active session cap and engine-first connection flow

- Status: Accepted
- Date: 2026-11-08

## Context

ADR 0003 gives every workspace tab its own live session. Unbounded tabs would mean unbounded database connections. The product needs a predictable resource cap and an explicit database choice before connecting.

The requested policy is five active connections maximum and termination of the earliest connection. That conflicts with running work, staged edits, and open transactions, so eviction must not silently destroy state.

## Decision

### Engine-first connection flow

- A new connection starts with an engine choice: PostgreSQL or SQLite now; MySQL/MariaDB, SQL Server, and Oracle later.
- After the engine is chosen, show or create profiles for that engine only.
- Add the engine to the profile. Engine-specific fields are shown by the profile form.
- Do not auto-select the last engine. The user chooses it each time.

### Active session cap

- The cap is **five live sessions globally**, across all engines and profiles.
- Disconnected tabs do not count.
- The Rust session map enforces the cap as a backstop.
- The frontend owns tab and staged-edit state and selects the eviction candidate.

### Eviction policy

- Order active sessions by connection time, oldest first. Ties use tab creation order.
- Protected sessions cannot be evicted:
  - a query is running;
  - the table tab has staged edits;
  - the session has an open transaction.
- When a sixth connection is requested:
  1. If fewer than five sessions are live, connect normally.
  2. Otherwise find the oldest unprotected session.
  3. If every session is protected, refuse the new connection and explain why.
  4. Disconnect the candidate, then connect the new session, keeping the cap strict.
- Eviction disconnects the session but keeps the tab, SQL text, results, layout, and filter state. Mark the tab disconnected and disable database actions until reconnect.
- Show a notice naming the evicted tab.
- Reconnecting a disconnected tab is subject to the same cap and may evict another session.
- Protecting an open transaction is engine-specific: PostgreSQL checks its own session state; SQLite checks autocommit mode.

### Failure behavior

- If the new connection fails after eviction, the new tab shows the error.
- The evicted tab remains disconnected with its SQL and results preserved.
- The user can reconnect the evicted tab later.

## Consequences

- The cap is strict at five sessions; there is no transient sixth connection.
- A failed connect can leave an old session disconnected. Its tab state is preserved.
- Five busy sessions block new connections until the user applies, discards, or finishes work.
- The backend needs session connection timestamps and a status check for running queries and open transactions.
- The frontend needs a staged-edit flag per table tab.
- The engine picker adds one step before the profile list.

## Alternatives considered

- Close the oldest tab: frees more but loses SQL, results, and layout.
- Refuse every new connection at the cap: safest, but defeats automatic eviction.
- Connect first, then evict: avoids losing the old session on a failed connect, but briefly exceeds the cap.
- Per-profile or per-engine cap: allows the total resource use to exceed the target.
- No protection: may roll back transactions or discard staged edits.
