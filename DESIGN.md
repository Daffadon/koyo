# Koyo design direction

This file records the existing Koyo visual signature. It is not a request for a redesign. Change it only when the product owner asks for a direction change.

## Design Read

Reading this as: native database workbench for developers and operators, in a calm, paper-and-ink utility style, dial ENERGY 1 / RHYTHM 1 / MOTION 1.

## Identity

Koyo is a desktop database client. It should feel like a precise working tool, not a marketing site. The interface is quiet, compact, and readable during long sessions. Data gets the visual attention; chrome stays out of the way.

The identity motif is the contrast between warm paper surfaces and dark phthalo green. The KY monogram carries the brand mark. Monospace is reserved for SQL and table data, where fixed character width helps comparison.

## Palette

The phthalo green brand color is `#123524`. The monogram uses cream text `#F6F2E7` on that green.

Dark mode is the default. A developer tool has a legitimate reason for a dark working surface, and the current theme preserves that.

The light theme is a warm paper theme:

- Background: `#F2F0E9`
- Surface and panel: `#F8F6F0`
- Sidebar: `#EAE7DD`
- Main text: `#30342F`
- Secondary text: `#5F665D`
- Accent: `#557A68`
- Accent soft: `#DCE5DE`
- Focus and control boundary: `#4C7463`
- Input boundary: `#7A8177`
- Warning text: `#7A5222`
- Danger text: `#7F4A43`

Warning and danger are semantic status colors only. They are not decorative accents. The active palette stays within a neutral base, one sage accent, and the phthalo brand mark.

## Typography

The UI face is the platform UI sans provided by GPUI. It is chosen for native familiarity and compact labels, not for display personality. The data face is the platform monospace family, chosen because SQL, table cells, and status values need aligned characters and quick scanning.

Do not introduce a display face into the working shell. If the brand later needs one, keep it to the logo or an about surface, not the data workspace.

## Layout and density

The shell is a fixed sidebar plus tabbed main workspace. This is content-driven: the explorer is a persistent index of database objects, and the workspace holds one live table or query per tab.

Spacing is compact and consistent. The work surface favors information density over large empty cards. Tables use a fixed row height so vertical scanning and paging stay predictable.

Radius stays small and functional. Rounded shapes mark controls and the brand mark; data rows and panels stay close to square.

## States

Every data view must show empty, loading, and error states. Loading states name what is happening. Empty states say why the view is empty. Error states say what failed and what to do next.

Keep the current table visible while the next page loads. The loading indicator belongs in the toolbar, not as a full-surface overlay that hides the data.

## Accessibility

Keyboard access, visible focus, readable contrast, and accessible names are part of the signature, not optional polish. The calm palette must not become low-contrast text. Custom rows and cells need a focus path and an activation key.

## Motion

Motion is limited to hover, focus, and state transitions. No decorative loops, parallax, or entrance choreography. A database tool should feel still.

## Reason log

- Phthalo green and cream carry the brand mark and separate it from the sage workspace accent.
- Warm paper light mode keeps long text sessions comfortable; dark mode suits developer tooling.
- Secondary text is darker than the earlier paper palette so it stays readable at small sizes.
- Monospace is limited to data because character alignment helps scanning.
- Compact spacing and fixed row heights support long tables and frequent paging.
- Focus colors are dark enough to remain visible on every light surface.
