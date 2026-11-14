# ADR 0001: Tauri PostgreSQL client

- Status: Superseded by [ADR 0009](0009-native-gpui.md) for the desktop shell
- Date: 2026-11-08

## Context

The first release targets Windows, macOS, and Linux. It supports PostgreSQL only and combines a SQL workbench with schema and table browsing. Users can stage table-row inserts, updates, and deletes, then apply or discard them. Tables without a primary key remain preview-only. Saved passwords belong in the operating system credential store.

## Decision

- Use Tauri 2 with React and TypeScript for the desktop UI, and Rust for database and credential operations.
- Run PostgreSQL work through asynchronous Tauri commands. Use `tokio-postgres` and its simple-query protocol for arbitrary SQL so result values can be returned as PostgreSQL text without compile-time query schemas.
- Use parameterized database values and quoted identifiers for generated table-edit statements. Apply a staged batch in one transaction; roll back the batch on any failure or stale-row conflict.
- Keep connection profiles in the application config directory without passwords. Store passwords with the Rust `keyring` crate in the platform credential store. If that store is unavailable, do not fall back to plaintext storage.
- Verify TLS server identity by default. Any local no-TLS mode must be an explicit per-profile choice with a visible warning.
- Stage grid edits in memory. Only tables with a primary key support insert, update, and delete; other tables remain read-only.
- Keep a cancellation token for the active connection so the workbench can stop a long-running statement without waiting for its result stream.

## Consequences

- Tauri allows a web UI while keeping PostgreSQL credentials and sockets in the Rust process.
- React and TypeScript fit the editor, schema tree, and data grid; this introduces a frontend build step.
- PostgreSQL values are shown as text in query results. The UI does not promise type-specific formatting.
- Linux password persistence requires an available Secret Service. Missing credential-store support is an error, not a reason to write the password to disk.
- If a transaction commit response is uncertain, clear the staged batch and reload the table before retrying to avoid duplicate inserts.
- GPUI, other database engines, SSH tunnels, visual query builders, and password export are outside this release.

## References

- [Tauri project creation](https://v2.tauri.app/start/create-project/)
- [Calling Rust from the Tauri frontend](https://v2.tauri.app/develop/calling-rust/)
- [Tauri state management](https://v2.tauri.app/develop/state-management/)
- [tokio-postgres client API](https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html)
- [rustls-tokio-postgres verified TLS guidance](https://docs.rs/rustls-tokio-postgres/latest/rustls_tokio_postgres/)
- [keyring v1 API](https://docs.rs/keyring/latest/keyring/v1/)
