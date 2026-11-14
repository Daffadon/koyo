# Koyo

Koyo is a cross-platform native database client for PostgreSQL, SQLite, MySQL, MariaDB, SQL Server, and Oracle. It is built with Rust and [GPUI Kit](https://gpui-kit.com), with no WebView and no JavaScript runtime. Oracle support is marked beta because it uses the official pre-1.0 pure-Rust driver.

## Features

- Save PostgreSQL, SQLite, MySQL, MariaDB, SQL Server, and Oracle connection profiles. Server passwords are kept in Windows Credential Manager, macOS Keychain, or Linux Secret Service; SQLite uses a database file path. Profile files contain no passwords. Oracle uses the official pure-Rust `oracledb` beta driver and needs no Instant Client.
- Choose the database engine before a profile, then open query and table tabs that each own a dedicated session. All connections share one tab pane: a live PostgreSQL table tab and a live MySQL table tab can stay open side by side, each pinned to its own session. There is no fixed limit on live sessions.
- Browse tables grouped by schema, then preview table rows. Filter columns server-side, sort by one column, page through results, and cancel a slow page load.
- Edit rows for tables with primary keys. Inserts, updates, and deletes stay staged until **Apply changes**. The batch runs in one transaction and rolls back on stale-row conflicts. If the commit result is uncertain, Koyo clears staged edits and reloads the page; verify data before retrying.
- Run SQL with the connected session's permissions. The editor suggests SQL keywords, schemas, tables, and columns from the connected database. Results are shown as a table or as a JSON array with one object per row (SQL NULL becomes `null`); each result set is capped at 1,000 displayed rows. A running query can be cancelled. SQLite runs one statement per execution.
- Switch between the dark theme and a warm, low-contrast light theme from the sidebar.
- Verify PostgreSQL TLS server identity by default. **Disable TLS** is available for trusted local connections only.

Tables without primary keys are read-only in the data grid. Linux PostgreSQL password storage requires a running Secret Service provider. Koyo does not save passwords in plaintext if the operating-system credential store is unavailable; passwordless connections do not need the credential store at all.

## Architecture

The workspace has two crates:

- `koyo` — native GPUI desktop application. The UI uses GPUI Kit components: buttons, inputs, code editor, tab bars, dialogs, status bar, and icons.
- `koyo-core` — database engines, session management, profile storage, and credential handling shared by every engine.

The application never calls a web bridge: UI actions call `koyo-core` async APIs directly and publish results back to the GPUI view.

## Development

Install Rust 1.88 or newer. On Linux, GPUI needs the native windowing and font development packages:

```sh
sudo apt-get install -y \
  build-essential \
  libxkbcommon-dev \
  libxkbcommon-x11-dev \
  libx11-xcb-dev \
  libwayland-dev \
  libxcb1-dev
```

The workspace is configured to link with [mold](https://github.com/rui314/mold) on `x86_64-unknown-linux-gnu` for faster builds:

```sh
sudo apt-get install -y mold
cargo run -p koyo
```

`.cargo/config.toml` passes `-fuse-ld=mold`. On macOS, Windows, and non-x86_64 Linux the setting is ignored and the platform linker is used. If mold is not installed on an x86_64 Linux host, remove that target section or install mold before building.

Build a release binary with:

```sh
cargo build --release -p koyo
```

The binary is written to `target/release/koyo`. Linux builds additionally need `libxkbcommon`, `libxkbcommon-x11`, X11, Wayland, and fontconfig runtime libraries, which are standard on desktop distributions.

### Tests

```sh
cargo test --workspace
```

The database integration tests are ignored by default; point them at disposable databases:

```sh
KOYO_TEST_DATABASE_URL="host=127.0.0.1 user=postgres password=... dbname=koyo_test sslmode=disable" \
  cargo test -p koyo-core -- --ignored
```

The MySQL, MariaDB, SQL Server, and Oracle workflow tests are also ignored by default:

```sh
KOYO_TEST_DB_HOST=127.0.0.1 KOYO_TEST_MYSQL_PORT=3306 KOYO_TEST_MYSQL_ENGINE=mysql \
  cargo test -p koyo-core --lib mysql_workflow -- --ignored
KOYO_TEST_DB_HOST=127.0.0.1 KOYO_TEST_MSSQL_PORT=1433 \
  cargo test -p koyo-core --lib mssql_workflow -- --ignored
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

Run every hook manually with `prek run --all-files`. The hooks assume the native development packages from above are installed.

See the [ADRs](docs/adr) for architecture decisions, including [ADR 0009](docs/adr/0009-native-gpui.md) for the GPUI migration and mold linker, and the [glossary](docs/glossary.md) for product terms.

## Releases

The release workflow builds Linux x64, Windows x64, and macOS Apple Silicon binaries. It runs on tags, uploads workflow artifacts, and creates a draft GitHub release.

Keep the workspace version in `koyo/Cargo.toml` and `koyo-core/Cargo.toml` equal, then push a matching tag:

```sh
git tag v0.2.0
git push origin v0.2.0
```

Builds are unsigned; macOS may require right-click **Open**, and Windows SmartScreen may warn.
