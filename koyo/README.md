# koyo

Native GPUI desktop application for Koyo. It contains the window, tabs, SQL editor, connection dialog, schema explorer, table grid, and staged-edit workflow.

```sh
cargo run -p koyo
```

Database work is provided by the `koyo-core` crate. Profiles are read from the platform config directory (`com.koyo.db/profiles.json`); server passwords live in the operating-system credential store.

The UI uses GPUI Kit components:

- `Button`, `Input`, `Checkbox`, and `Editor` for controls.
- `TabBar` and `Tab` for workspace and result-set tabs.
- `StatusBar` for the connection and row status line.
- Themed confirmation overlays for destructive actions.
- `Icon` for built-in monochrome icons.
- Collapsible explorer, per-pane tab lists, draggable split panes, resizable and collapsible result panes.
- Metadata explorer for tables, views, functions, and triggers, plus a foreign-key relation view per table.

Run the full workspace checks from the repository root:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```
