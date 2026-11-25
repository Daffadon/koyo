<div align="center">

# Koyo

**A fast, native database client for PostgreSQL, SQLite, MySQL, MariaDB, SQL Server, and Oracle.**

[![Rust](https://img.shields.io/badge/Rust-1.88%2B-000000?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![GPUI Kit](https://img.shields.io/badge/UI-GPUI%20Kit-5A7D6A?style=flat-square)](https://gpui-kit.com)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-4C6EF5?style=flat-square)](#supported-engines)
[![No WebView](https://img.shields.io/badge/runtime-no%20WebView%20%C2%B7%20no%20JavaScript-2F9E44?style=flat-square)](#architecture)

[Features](#features) · [Quick start](#quick-start) · [Engines](#supported-engines) · [Architecture](#architecture) · [Development](#development) · [Limitations](#known-limitations)

</div>

Koyo is a cross-platform desktop database client written in Rust and built with [GPUI Kit](https://gpui-kit.com). It renders natively, talks to databases directly from Rust, and keeps each workspace pane on a stable database session and reuses it for new tabs. There is no WebView, no JavaScript runtime, and no frontend IPC bridge.

Koyo is for everyday work: run SQL with completion and history, inspect schemas, page through tables, filter and sort server-side, and stage row edits before applying them in one transaction.

## Features

### SQL workbench

- SQL editor with syntax highlighting, keyword, schema, table, column, and alias completion. Names match anywhere, not only from the first character. `u.` completion resolves through the table behind the alias. Press **Tab** or **Enter** to apply the selected completion; Enter inserts a newline when the completion menu is closed.
- Format SQL from the editor toolbar or with **Ctrl+Shift+F** / **Cmd+Shift+F**. A selection formats alone; otherwise the whole editor is formatted.
- Run with **Ctrl+Enter** or **Cmd+Enter**. Selected SQL runs alone; with no selection the whole editor runs.
- Line editing: **Ctrl+/** toggles SQL line comments, **Alt+Up/Down** moves lines, **Shift+Alt+Down** duplicates a line, and **Ctrl+Shift+K** deletes a line.
- Multiple result sets per execution, shown as a data grid or as pretty-printed JSON, one object per row.
- Copy the active result set as CSV or JSON from the result options menu, download it as a CSV file, copy individual cells, or copy rows as TSV.
- Drag column edges to resize query-result columns.
- Persistent query history: search by SQL, profile, or result, collapse the panel, and load or re-run an old statement. History keeps the latest 100 distinct SQL/profile pairs.
- Each result set is capped at 1,000 displayed rows.

### Table browser and data editing

- Explore tables, views, functions, triggers, and engine metadata grouped by schema. Tables and views open in data grids; functions and triggers fetch their definitions automatically and load the body into a query tab.
- Press **F5** to refresh the active table.
- Preview rows with server-side filter expressions, one-column sorting, offset paging, total page counts, and cancellable page loads.
- Filter input suggests matching columns and valid operators as you type. Press **Tab** to accept, or click a suggestion. Filter input accepts forms such as `name contains ada`, `age >= 30`, and `status = closed AND amount < 100`.
- Download table rows as CSV from the current page, a page range, or every page. All-page exports use the current filter and sort and stream page by page through the existing table session.
- Opening a table in a pane reuses that pane's tab for the same table, including staged edits and filters. If the table is already open in the other pane, Koyo opens a second tab in the focused pane, like editor groups.
- Foreign-key diagram for the open table.
- Tables with primary keys support staged inserts, updates, and deletes. Apply the batch in one transaction.
- Tick row checkboxes to select rows on the page. **Select all** selects every deletable row; **Delete selected** stages them together.
- Stale-row checks roll back the batch when a row changed after it was loaded.
- Keyless tables and views are read-only.
- Right-click a table cell to copy its value, copy the row, or mark the row for deletion. Click a row number to copy the row.

### Connections and workspace

- Six engines with an engine-first connection flow and per-engine profile forms.
- Create, edit, delete, and search profiles. Profiles contain no passwords.
- Server passwords live in Windows Credential Manager, macOS Keychain, or Linux Secret Service. SQLite profiles use a database file path.
- Tabs for one connection share a live session across panes. Splitting duplicates the active tab on the same session, so a split does not reconnect. Closing the last tab parks its session in an idle connection pool; the next selected tab reuses it. Closing a tab selects its right neighbor, or its left neighbor at the end of the strip. Explicit disconnect and connection loss close every session and tab for that profile, not just the clicked tab.
- Reconnecting once attaches the new session to every disconnected tab with the same profile target in both panes, including the shared explorer. The schema catalog is fetched once and shared. Staged edits survive a same-target reconnect; they are discarded only when the connection target changed.
- Multiple databases can stay live side by side in one tab strip. Shared tabs also share transaction and session state.
- The left explorer and right AI panel belong to the workspace, not individual tabs. Switching tabs keeps the selected connection and panel state.
- Split panes, tab drag between panes, resizable panes and sidebars, and resizable table columns.
- Reorder tabs with **Ctrl+Shift+PageUp/PageDown** or **Cmd+Shift+PageUp/PageDown**, or drag a tab's icon onto another tab. Drag onto another pane to move it there.
- The idle pool defaults to four clean sessions, one per profile, with a five-minute TTL. Settings changes the cap and TTL, can close all idle sessions, and shows the current count. Parked sessions are probed every fifteen seconds; dead entries are dropped before reuse.
- Toast notifications report connect, disconnect, connection lost, query completion, table edits, and CSV exports. Settings toggles each category or silences every toast.
- Three themes: Dark, Light, and Kimi no Nawa. Dark stays black with comet-gradient actions; Kimi no Nawa adds a blurred twilight comet gradient. Settings groups appearance, notifications, and idle connections in a two-pane dialog.
- Theme, sidebar state and width, split layout, active tabs, open tabs, and notification preferences restore on launch. Restored tabs start disconnected and reconnect on demand.
- Destructive actions require confirmation: closing with unsaved SQL or staged edits, deleting a connection, or clearing query history.

### AI assistant

- Collapsible chat panel on the right side. Drag its divider to resize; the width, active thread, and thread transcripts restore on launch.
- Toggle the left sidebar with **Ctrl+B** or **Cmd+B**; toggle the AI panel with **Ctrl+Alt+B** or **Cmd+Alt+B**. Open Settings from the gear in the sidebar header.
- Multiple chat threads with previous, next, new, and delete actions. Thread titles come from the first message.
- Responses stream into the transcript and render Markdown, including fenced code blocks. **Stop** cancels the active stream.
- **Compact context** asks the provider to summarize the current thread and replaces it with a system summary, keeping the next request small. Long threads are split into chunks and merged.
- Configure any OpenAI-compatible chat-completions provider from the panel settings using JSON: `base_url`, `api_key`, `model`, `system_prompt`, `temperature`, `max_tokens`, and optional `headers`.
- Remote AI endpoints must use HTTPS. Plain HTTP is accepted only for loopback hosts such as `localhost` or `127.0.0.1`.
- API keys, provider configuration, and chat threads are stored in encrypted application state, not in plaintext.
- Press Enter to send and Shift+Enter for a newline. Requests run on the application runtime and do not block the UI.

## Supported engines

| Engine | Driver | Default port | Notes |
| --- | --- | --- | --- |
| PostgreSQL | `tokio-postgres` + `rustls` | 5432 | TLS server identity verified by default; cancel supported |
| SQLite | `rusqlite` (bundled) | — | File-based; one statement per execution; cancel supported |
| MySQL | `mysql_async` + `rustls` | 3306 | Server-side filters, paging, staged edits, cancel |
| MariaDB | `mysql_async` + `rustls` | 3306 | Same workflow as MySQL |
| SQL Server | `tiberius` + `rustls` | 1433 | SQL authentication; cancel closes the session, so reconnect afterward |
| Oracle | `oracledb` (beta) | 1521 | No Instant Client; cancellation unavailable in the beta driver |

> [!NOTE]
> Oracle support is beta because it uses the official pre-1.0 pure-Rust `oracledb` driver. The driver, profile surface, and SQL behavior may change.

> [!WARNING]
> **Disable TLS** is intended only for trusted local connections. PostgreSQL verifies the server identity by default; the other network engines request TLS and validate certificates in `verify-full` mode.

## Quick start

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer.
- On Linux, native windowing and font development packages. On Debian/Ubuntu:

  ```sh
  sudo apt-get install -y \
    build-essential \
    libxkbcommon-dev \
    libxkbcommon-x11-dev \
    libx11-xcb-dev \
    libwayland-dev \
    libxcb1-dev
  ```

- On macOS, Xcode Command Line Tools.
- On Windows, the MSVC Rust toolchain plus Visual Studio Build Tools with the Windows SDK.

### Install a release

- **Windows:** run `koyo-windows-x64-setup.exe`. It installs under `%LOCALAPPDATA%\Programs\Koyo`, adds a Start Menu shortcut, and registers an uninstall entry.
- **macOS:** open `koyo-macos-arm64.dmg`, then drag **Koyo** to Applications.
- **Linux:** extract `koyo-linux-x64.tar.gz` and run `./install.sh`. It installs the binary, desktop entry, and icon under `~/.local` so the app appears in the desktop application menu.

Release installers are unsigned; Windows SmartScreen and macOS Gatekeeper may warn.

### Build and run

```sh
git clone https://github.com/Daffadon/koyo.git
cd koyo
cargo run -p koyo
```

The first run creates an empty workspace. Open **Manage connections**, choose an engine, add a profile, and connect. A query tab opens with starter SQL for the engine.

For a release build:

```sh
cargo build --release -p koyo
```

The binary is written to `target/release/koyo`. Linux release builds additionally need the `libxkbcommon`, `libxkbcommon-x11`, X11, Wayland, and fontconfig runtime libraries.

> [!IMPORTANT]
> x86_64 Linux builds link with [mold](https://github.com/rui314/mold) through `.cargo/config.toml`. Install `mold`, or remove the `[target.x86_64-unknown-linux-gnu]` section if you prefer the platform linker. Other targets ignore the setting.

## Architecture

Koyo is a two-crate Rust workspace:

- **`koyo`** — native GPUI desktop application: window, tabs, editor, schema explorer, grids, dialogs, persistence, and shortcuts.
- **`koyo-core`** — database engines, session management, SQL generation, metadata discovery, profile storage, and credential handling. It does not depend on GPUI.

```text
┌──────────────────────────────────────────────┐
│ koyo (GPUI)                                  │
│  views render state · actions update state   │
└──────────────────────┬───────────────────────┘
                       │ async koyo-core APIs
┌──────────────────────▼───────────────────────┐
│ koyo-core                                    │
│  SessionManager · profiles · credentials     │
│  PostgreSQL · SQLite · MySQL · MariaDB       │
│  SQL Server · Oracle                         │
└──────────────────────────────────────────────┘
```

Inside `koyo`:

```text
koyo/src/
  main.rs                  application startup, window, shortcuts
  app.rs                   module wiring and shared re-exports
  app/state.rs             tabs, panes, dialogs, staged edits, app state
  app/settings.rs          persisted theme, AI config, workspace, history, and notifications
  app/theme.rs             palette and light-theme adjustment
  app/helpers.rs           labels, starter SQL, filters, form defaults, CSV
  app/ai.rs                OpenAI-compatible chat-completions client and config
  app/completion.rs        SQL completion catalog and alias parser
  app/actions/             state transitions and async work
    workspace.rs           panes, tabs, splits, persistence, confirmations
    connections.rs         connect, catalog loading, profile dialog/storage
    queries.rs             run, cancel, history, result selection and resize
    tables.rs              paging, filtering, sorting, refresh, apply/discard
    editing.rs             cell editing, inserts, staged changes
    notifications.rs       notification preference gate and toast IDs
    settings.rs            settings dialog actions
    ai.rs                  AI threads, provider settings, streaming, compacting
  app/views/               GPUI rendering only
    root.rs                root frame and overlays
    sidebar.rs             explorer, connection summary, query history
    settings.rs            notification preferences dialog
    ai.rs                  AI chat threads, markdown, stop, compact, provider settings
    workspace.rs           tab bars, panes, status bar
    connection.rs          engine picker and connection dialog
    query/                 editor and result rendering
    table/                 filter, grid, and diagram rendering
    widgets.rs             shared dialogs and input widgets
```

Views read state and emit actions. Actions perform session and database work, then update state and notify GPUI. Database work runs on a Tokio runtime owned by the application; the UI never blocks on a database call.

## Configuration and security

Koyo stores application data in the platform config directory:

| Platform | Path |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/com.koyo.db` or `~/.config/com.koyo.db` |
| macOS | `~/Library/Application Support/com.koyo.db` |
| Windows | `%APPDATA%\com.koyo.db` |

- `profiles.enc` contains encrypted connection profiles without passwords.
- `state.enc` contains the theme, sidebar and AI panel state, AI provider configuration and chat threads, workspace snapshot, and query history, encrypted with XChaCha20-Poly1305.
- The data encryption key and server passwords live in the operating-system credential store.
- AI provider API keys are saved in `state.enc`; the panel settings never write a plaintext provider file.
- Legacy plaintext `state.json` and `profiles.json` files are encrypted and removed on first successful migration.
- If the credential store is unavailable, Koyo reports an error and disables state persistence instead of writing plaintext.
- Password fields are masked and cannot copy or cut their values. Saved passwords are never loaded back into the dialog.
- Linux saved profiles, passwords, and encrypted state require a running Secret Service provider. Without it, profiles cannot be saved and state persistence is disabled.

> [!NOTE]
> Query history, restored SQL, and connection profiles are encrypted at rest. Losing the `state:key` credential makes the files unreadable; Koyo does not silently overwrite them or fall back to plaintext.

## Development

### Checks

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

### Tests

The database integration tests are ignored by default. Point them at disposable databases.

PostgreSQL:

```sh
KOYO_TEST_DATABASE_URL="host=127.0.0.1 user=postgres password=... dbname=koyo_test sslmode=disable" \
  cargo test -p koyo-core -- --ignored
```

MySQL or MariaDB:

```sh
KOYO_TEST_DB_HOST=127.0.0.1 KOYO_TEST_MYSQL_PORT=3306 KOYO_TEST_MYSQL_ENGINE=mysql \
  cargo test -p koyo-core --lib mysql_workflow -- --ignored
```

SQL Server:

```sh
KOYO_TEST_DB_HOST=127.0.0.1 KOYO_TEST_MSSQL_PORT=1433 \
  cargo test -p koyo-core --lib mssql_workflow -- --ignored
```

Oracle:

```sh
KOYO_TEST_ORACLE_HOST=127.0.0.1 KOYO_TEST_ORACLE_PORT=1521 KOYO_TEST_ORACLE_SERVICE=FREEPDB1 \
KOYO_TEST_ORACLE_USER=koyo KOYO_TEST_ORACLE_PASSWORD=koyo \
  cargo test -p koyo-core --lib oracle_workflow -- --ignored
```

The workflow tests expect a `koyo` database/schema with `users`, `orders`, and `events` tables and primary keys. Use disposable data only.

### Git hooks

`prek` runs formatting and build checks. Install `prek` with your preferred package manager, then install the Git shims once per clone:

```sh
prek install --force
```

- **pre-commit**: whitespace, file endings, YAML/JSON/TOML syntax, large files, private keys, case and Windows-name conflicts, `cargo fmt --all --check`.
- **pre-push**: pre-commit checks plus `cargo check --workspace`.

Run every hook manually with `prek run --all-files`.

### Releases

The release workflow builds Linux x64, Windows x64, and macOS Apple Silicon packages on version tags, uploads the Linux archive, Windows installer and portable zip, macOS app archive and DMG, then creates a draft GitHub release. The draft body lists changes since the previous tag, commit hashes, authors, and a contributors section.

Keep the workspace version in `koyo/Cargo.toml` and `koyo-core/Cargo.toml` equal, then push a matching tag:

```sh
git tag v0.3.10
git push origin v0.3.10
```

Builds are unsigned. macOS may require right-click **Open**; Windows SmartScreen may warn.

## Known limitations

- SQLite executes one statement per run.
- Oracle beta cannot cancel a running statement.
- SQL Server cancellation terminates the session; reconnect before continuing.
- Binary and large object values display according to each engine's text contract. Binary editing is not supported.
- Offset paging can skip or repeat rows when data changes concurrently. Views without an explicit sort have no stable page order.
- Multi-column sorting, filter groups, keyset paging, SSH tunnels, client certificates, Windows integrated authentication, and schema editing are not implemented.
- Query results are capped at 1,000 rows per result set; table pages are capped at 100 rows.
- Tables without primary keys are read-only in the data grid.

## Documentation

- [Architecture decision records](docs/adr) — engine boundaries, sessions, GPUI migration, result views, and theme decisions.
- [Glossary](docs/glossary.md) — product terms used by the UI and docs.
