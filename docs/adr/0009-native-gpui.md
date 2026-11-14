# ADR 0009: Native GPUI application and mold linker

- Status: Accepted
- Date: 2026-11-14
- Supersedes: [ADR 0001](0001-tauri-postgres.md) for the desktop shell

## Context

ADR 0001 chose Tauri 2 with a React and TypeScript frontend. That shell required Node.js, a web view runtime, a separate frontend build, and an IPC bridge for every database action. The database engine work already lives in Rust (`koyo-core`), so the web layer mainly translated state between two runtimes.

The project now targets a native desktop application with the same workflows: connection profiles, per-tab database sessions, a SQL editor with cancellable statements, schema browsing, paged table views, server-side filters and sort, staged row edits, and credential-store-backed passwords.

## Decision

- Replace the Tauri/React shell with GPUI through [GPUI Kit](https://gpui-kit.com). `koyo` is a native binary and shares `koyo-core` directly; there is no web view and no IPC bridge.
- Build the interface from GPUI Kit components: `Button`, `Input`, `Editor`, `TabBar`, `Checkbox`, `StatusBar`, dialogs, and icons.
- Keep UI state in a single GPUI view. Each workspace tab owns its session, SQL text, results, explorer state, filters, sort, and staged changes.
- Run database work on a Tokio runtime owned by the application. Completed work returns to the GPUI view through an async channel; the UI never blocks on a database call.
- Link x86_64 Linux builds with mold via `.cargo/config.toml` (`-C link-arg=-fuse-ld=mold`). Other targets use the platform linker.
- Set `RUST_FONTCONFIG_DLOPEN=1` so GPUI loads fontconfig at runtime instead of requiring build-time pkg-config metadata for it.

## Consequences

- One language and one build system: `cargo build --release -p koyo`.
- No Node.js, npm, Tauri CLI, WebView2, or frontend asset packaging in development or release automation.
- GPUI requires native windowing and font libraries at build/link time on Linux; release builds document the runtime packages.
- mold is required on x86_64 Linux. Removing the target section of `.cargo/config.toml` or installing mold restores a working build.
- UI behavior is tested by running the native binary. Database behavior remains covered by `koyo-core` tests.

## References

- [GPUI Kit](https://gpui-kit.com)
- [mold linker](https://github.com/rui314/mold)
- [ADRs 0003–0008](.) for tab, engine, and session decisions that still apply
