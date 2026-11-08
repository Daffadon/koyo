# Koyo

Koyo is a cross-platform desktop client for PostgreSQL, built with Tauri 2, React, TypeScript, and Rust.

## First-release features

- Save PostgreSQL connection profiles. Passwords are kept in Windows Credential Manager, macOS Keychain, or Linux Secret Service; profile files contain no passwords.
- Browse schemas and tables, then preview table rows.
- Edit rows for tables with primary keys. Inserts, updates, and deletes stay staged until **Apply changes**. The batch runs in one transaction and rolls back on stale-row conflicts. If the commit result is uncertain, Koyo clears staged edits and attempts to reload; reconnect and reload before retrying if needed.
- Run SQL with the connected PostgreSQL role's permissions. Results show PostgreSQL text values and each result set is capped at 1,000 displayed rows. A running query can be cancelled.
- Verify TLS server identity by default. **Disable TLS** is available for trusted local connections only.

Tables without primary keys are read-only in the data grid. Linux password storage requires a running Secret Service provider. Koyo does not save passwords in plaintext if the operating-system credential store is unavailable; passwordless connections do not need the credential store at all.

## Development

Install Node.js, npm, Rust, and the platform prerequisites listed in the [Tauri guide](https://v2.tauri.app/start/prerequisites/). Then run:

```sh
npm install
npm run tauri dev
```

Create a frontend production build with `npm run build`. Build a desktop bundle with `npm run tauri build`.

### Git hooks

`prek` is included as a dev dependency. Install the Git shims once per clone:

```sh
npm install
npm run hooks:install
```

- **pre-commit**: whitespace, file endings, YAML/JSON/TOML syntax, large files, private keys, case and Windows-name conflicts, `cargo fmt --check`.
- **pre-push**: pre-commit checks plus `npm run build`.

Run every hook manually with `npm run hooks:run`. `tsconfig*.json` is excluded from the JSON syntax hook because TypeScript config files allow comments. The Rust hook requires `rustfmt`; install it with `rustup component add rustfmt`.

Run the default Rust tests with `cargo test --manifest-path src-tauri/Cargo.toml`. The database integration test is ignored by default; point it at a disposable database:

```sh
KOYO_TEST_DATABASE_URL="host=127.0.0.1 user=postgres password=... dbname=koyo_test sslmode=disable" \
  cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
```

See [ADR 0001](docs/adr/0001-tauri-postgres.md) for architecture decisions and the [glossary](docs/glossary.md) for product terms.

## Releases

The release workflow builds Windows x64, Linux x64, macOS Intel, and macOS Apple Silicon. It runs on tags and manual dispatch, uploads workflow artifacts, and creates a draft GitHub release.

Keep these versions equal, then push a matching tag:

```sh
# package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json
git tag v0.1.0
git push origin v0.1.0
```

The workflow rejects a tag that does not match the configured version. Builds are unsigned; macOS may require right-click **Open**, and Windows SmartScreen may warn.
