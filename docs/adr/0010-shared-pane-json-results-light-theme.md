# ADR 0010: Shared connection pane, JSON results, and calm light theme

- Status: Accepted
- Date: 2026-11-14

## Context

Koyo now supports several live databases at once. The workspace must keep a
PostgreSQL table and a MySQL table open side by side without one connection
taking over the other tab. Live tabs already owned independent sessions, but
choosing another profile from the sidebar replaced the active tab whenever
that tab was a query, and live table tabs could not switch connections at all.

Two presentation issues followed the same session work:

- Clicking a table column header briefly flashed the table toolbar. Page loads
  inserted a Cancel button, conditionally added the staged badge, and dimmed
  every sortable header while `loading` was true.
- Query results only had a tabular view. Large rows with many columns are
  easier to inspect as JSON when the caller wants to copy a row or compare
  nested payloads.
- The stock light theme is bright white with saturated accents and is harsh
  beside the existing dark theme.

## Decision

### One shared pane

- All connections use the same primary/secondary tab pane. Each tab remains
  bound to the session it opened.
- Changing profiles from a live tab opens a new query tab for the new profile.
  It never retargets a live table or query tab. Empty and disconnected tabs may
  still be retargeted in place.
- Table tabs stay on their own connection. Opening a table from another
  connection creates another tab that owns that table's session.
- Tab prefixes use the engine's official database mark so tabs from different
  engines are visually distinguishable. No connection-wide pane switching is
  used.

### Stable toolbars

- Controls that depend on transient loading state stay mounted. The table and
  query Cancel buttons use an invisible reserved slot instead of being inserted
  on demand. The pending-edit badge also reserves its slot.
- Header sort affordances depend on staged edits and connectivity, not on the
  current page load. Clicking a header may start a load, but does not change
  header opacity or button geometry.
- Action handlers still guard concurrent loads; visual stability does not
  weaken the state checks.

### JSON result view

- Each query tab gets a `Table`/`JSON` toggle in the results header.
- JSON is an array, one object per result row. Keys are the returned column
  names; values are JSON strings, or `null` for SQL NULL. Values are not
  heuristically converted to numbers or booleans, because the database text
  representation can contain leading zeros or otherwise lose fidelity.
- Keys and values are escaped as JSON strings. The renderer is virtualized one
  line per row and scrolls horizontally for wide rows.
- JSON is a presentation mode only. Editing and copying are not implied by the
  view.

### Calm light theme

- Add a sidebar light/dark toggle. Dark remains the startup default.
- Light mode overrides GPUI Kit's semantic color tokens after switching:
  warm paper backgrounds, sage accents, desaturated borders, and gray-green
  text instead of white and highly saturated blue.
- The override is implemented through `Theme::update` so component tokens,
  base scrollbar colors, and window refresh stay consistent.

## Consequences

- Multi-database work uses one tab strip and one session per tab. It does not
  multiplex a shared connection across databases.
- Switching profiles is additive for live tabs. Users may end up with several
  query tabs for the same profile; there is no automatic tab reuse.
- JSON values are strings. A future typed result protocol can preserve native
  JSON numbers/booleans without changing the view contract.
- The light palette is intentionally session-scoped and is not persisted across
  launches yet.
- Stable toolbar slots consume a small amount of horizontal space even when
  idle, traded for no layout jump during page loads and sorts.

## Alternatives considered

- **Pane per connection:** rejected because it fragments the workspace and
  prevents two live tables from sharing the visible tab strip.
- **Retarget table tabs:** rejected because a table page, filters, and staged
  edits belong to the session that loaded them.
- **Freeze the whole toolbar during loads:** rejected; disabled/hidden controls
  would still be a state change. Reserving slots keeps the layout stable while
  preserving cancellation and edit affordances.
- **Serialize query values with `serde_json` inference:** rejected; it can turn
  text that merely looks numeric into a different value.
- **Pure white light theme:** rejected as too bright; the calm palette keeps
  contrast for data grids while reducing glare.

## References

- [ADR 0003: Multi-connection tabs](0003-multi-connection-tabs.md)
- [ADR 0009: Native GPUI application and mold linker](0009-native-gpui.md)
- [GPUI Kit themes](https://gpui-kit.com)
