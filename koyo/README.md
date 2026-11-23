# koyo

Native GPUI desktop application for Koyo. It contains the window, tabs, SQL editor, connection dialog, schema explorer, table grid, and staged-edit workflow.

```sh
cargo run -p koyo
```

Database work is provided by the `koyo-core` crate. Profiles are read from the platform config directory (`com.koyo.db/profiles.enc`); server passwords live in the operating-system credential store.

The UI uses GPUI Kit components:

- `Button`, `Input`, `Checkbox`, and `Editor` for controls.
- `TabBar` and `Tab` for workspace and result-set tabs.
- `StatusBar` for the connection and row status line.
- Themed confirmation overlays for destructive actions.
- `Icon` for built-in monochrome icons.
- Collapsible explorer, per-pane tab lists, draggable split panes, resizable and collapsible result panes.
- Resizable, collapsible right-side AI chat panel with multiple threads, stop, compact-context, streaming Markdown, and OpenAI-compatible JSON provider settings.
- **Ctrl+Shift+F** / **Cmd+Shift+F** formats the active query; a selection formats alone.
- **Ctrl+Shift+PageUp/PageDown** / **Cmd+Shift+PageUp/PageDown** reorders tabs; dragging a tab icon also reorders or moves it to another pane.
- **Ctrl+B** / **Cmd+B** toggles the left sidebar; **Ctrl+Alt+B** / **Cmd+Alt+B** toggles the AI panel.
- Metadata explorer for tables, views, functions, and triggers, plus a foreign-key relation view per table.

Run the full workspace checks from the repository root:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```
