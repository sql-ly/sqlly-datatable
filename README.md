# sqlly-datatable

## Why

I discovered [GPUI](https://github.com/zed-industries/gpui) while building [sqlly.app](https://sqlly.app) and immediately loved it. It is fast, the immediate-mode canvas model is a joy to work in, and the codebase is a pleasure to read. GPUI is what made me decide to port sqlly from a Mac-only app to a fully cross-platform tool.

What I discovered along the way is that building a real SQL management client on top of GPUI requires a lot of little tweaks that GPUI doesn't ship with out of the box — virtualized data grids, drag selection, column resizing, scrollbars, configurable formatting, context menus, the works. So I built them.

Before you read on: I am not a skilled Rust developer. I have written custom UI controls in several other languages over the years, but I built this project with AI assistance (read as: prompt, check, sigh, repeat). I cannot attest to the quality of the code. I am sharing it freely in the hope that people more skilled than I am will improve it — or that the GPUI maintainers see something here worth pulling upstream.

If even one part of this makes a GPUI contributor think "huh, that's a useful pattern" rather than "wow, what a bunch of AI slop," the project has served its purpose.

**Why not just submit a PR?**
Given that this may or may not be total AI slop and my name is attached to it, doing so would be a bit gauche don't you think?

## What

A configurable data grid component for GPUI, built for the needs of [sqlly.app](https://sqlly.app). The library targets Rust 1.87+ and links against `gpui` 0.2.

## Features

- **Virtualized cell selection** — large datasets render only the visible rows; cell / row / column / rectangular drag selection still works on rows that are currently offscreen.
- **Cell selection** — single cells, row ranges, column ranges, and click-drag rectangle selection. Shift extends the existing selection.
- **Sorting** — click column headers or sort buttons to cycle ascending / descending / off. Uses a deterministic total ordering across `Integer`, `Decimal`, `Text`, `Date`, `Boolean`, and `None`.
- **Filtering** — per-column filter prompt via the right-click context menu. The filter is compared against the formatted value, case-insensitively.
- **Column resizing** — drag column borders to resize; the cell `[r1..r2]` predicate normalizes reversed drag directions.
- **Scrollbars** — horizontal and vertical, with scroll clamping and edge-scroll during drag selection. Auto-scroll is on-demand (only while a drag is active), so it isn't a 60 fps loop.
- **Context menu** — right-click column headers for select / copy / copy-with-headers / sort / clear sort / filter / clear filter. Fully customizable via `ContextMenuProvider` (see below).
- **Keyboard navigation** — arrow keys, page up/down, select all, copy, copy-with-headers, configurable per platform.
- **Status bar** — shows click position, scroll offsets, cell coordinates, and hover info.
- **Theming** — two shipped theme families, each with light and dark variants that follow the OS appearance, and full custom theming (every color is a public field). See below.

## Theming

Two complete theme families ship with the crate. Each is a `GridThemePair` — a light and a dark `GridTheme` — and the widget automatically applies the variant matching the OS window appearance (and swaps live when the system appearance changes):

- **Neutral** (`GridThemePair::neutral()`, the default) — chroma-free gray surfaces with one restrained azure accent. Designed to blend into a host application.
- **Signature** (`GridThemePair::signature()`) — the crate's own look, built around a teal anchor (`oklch(0.47 0.115 195)`): subtly tinted neutrals with a committed accent carrying selection, pivot chips, and the totals hierarchy.

```rust
use sqlly_datatable::{GridThemePair, SqllyDataTable};

// Pick a shipped family at build time (keeps OS light/dark following):
let view = SqllyDataTable::builder(data)
    .theme_family(GridThemePair::signature())
    .build(cx);

// Swap families at runtime (e.g. from a settings menu):
table.set_theme_family(GridThemePair::neutral(), window, cx);
```

All four shipped palettes were designed in OKLCH and are contrast-verified in unit tests: every text role meets WCAG AA (≥ 4.5:1) against every surface it is painted on, in both light and dark.

Beyond the shipped pair, the grid is themable to the bone: every color — including scrollbar thumbs and the modal overlay scrim — is a public field on `GridTheme`, and nothing in the paint code is hardcoded. A host app can construct its own `GridThemePair` (keeping automatic light/dark following) or pass a single fixed `GridTheme` via `.theme(...)` to opt out of appearance following entirely.

The sample app has a theme switcher in its toolbar demonstrating both families.

## Configuration

All formatting and behavior is externally configurable via `GridConfig` with per-column `ColumnOverride`:

- **Numbers** — decimal places, negative red, parentheses, thousands separators, alignment. Integer cells are formatted without a `f64` round-trip so values larger than `2^53` stay exact.
- **Dates** — format string with `%Y %m %d %H %M %S %y %B %b %A %a` tokens; timezone offset; natural language relative dates ("2 days ago", "in 3 weeks") with frozen-clock testing hooks.
- **Booleans** — custom true/false text, alignment.
- **Strings** — case (upper/lower/title/none), max length in **characters**, truncation (ellipsis / cut-off / wrap), alignment. Truncation respects char boundaries so multi-byte input never panics.
- **Replacement rules** — find/replace pairs applied before or after formatting.
- **Key bindings** — `SELECT ALL`, `COPY`, `COPY WITH HEADERS`, `PAGE UP`, `PAGE DOWN`, context-menu modifier. `KeyBinding::matches` is strict about modifier sets (an unrequested modifier disqualifies the binding).
- **Empty state** — `GridConfig::empty_text` paints a centered hint when the grid has zero rows (default "No rows", localizable, empty string to disable).

## Workspace

```
sqlly-datatable/
├── crates/
│   ├── sqlly-datatable/          # Library crate
│   └── sqlly-datatable-sample/   # Sample application
└── Cargo.toml                    # Workspace root
```

## Usage

```rust
use gpui::{App, Bounds, WindowBounds, WindowOptions, px, size};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, SqllyDataTable,
};

let data = GridData::new(
    vec![Column { name: "amount".into(), kind: ColumnKind::Decimal, width: 200.0 }],
    vec![
        vec![CellValue::Decimal(17968.20)],
        vec![CellValue::Decimal(717.84)],
    ],
).expect("rectangular data");

let view = SqllyDataTable::builder(data)
    .config(GridConfig::default())
    .build(cx);
```

### Per-column overrides

```rust
use sqlly_datatable::{ColumnOverride, GridConfig, NumberFormat};

let mut config = GridConfig::default();
config.column_overrides = vec![
    ColumnOverride {
        number: Some(NumberFormat { decimals: 0, ..Default::default() }),
        ..Default::default()
    },
    // ... per column
];
```

### Sort, filter, sort deterministically

`compare_cells` returns a total ordering that handles `NaN`, mixed numeric
kinds, and cross-type comparisons deterministically — useful if you want to
sort outside the widget too.

```rust
use sqlly_datatable::compare_cells;
let mut rows: Vec<&CellValue> = /* ... */;
rows.sort_by(|a, b| compare_cells(a, b).reverse());
```

### Custom right-click context menus

Implement `ContextMenuProvider` and register it on the builder to fully
control the right-click menu for cells, row headers, and column headers.
When a provider is registered, the built-in column-header menu is
suppressed; use `ContextMenuItem::standard_column_header_items()` to
compose built-in actions alongside custom ones.

The provider receives an owned `ContextMenuRequest` snapshot captured at
menu-open time. It includes:

- **`target`** — what was right-clicked (cell, row header, column header,
  sort button) with both display and source row indices.
- **`selected_cells`** — every selected cell with column name and value.
- **`selected_rows`** — every selected row with all cell values and column
  metadata for name-based lookups (`value_by_name`, `named_values`).

Right-click inside an existing selection preserves the selection; right-click
outside collapses to the clicked target. The snapshot survives until the user
clicks a menu item, so the provider's `on_action` sees exactly what was
selected when the menu opened.

```rust
use sqlly_datatable::{
    ContextMenuItem, ContextMenuProvider, ContextMenuRequest, ContextMenuTarget,
    GridState, SqllyDataTable,
};
use gpui::App;

struct MyMenuProvider;

impl ContextMenuProvider for MyMenuProvider {
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
        let mut items = Vec::new();
        if let Some(row) = request.clicked_row() {
            if let Some(value) = row.value_by_name("narrative") {
                items.push(ContextMenuItem::action(
                    "copy-narrative",
                    format!("Copy: {value:?}"),
                ));
            }
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action("inspect", "Inspect row"));
        }
        // Compose built-in sort/copy/filter for column-header right-clicks.
        if matches!(
            request.target,
            ContextMenuTarget::ColumnHeader { .. } | ContextMenuTarget::SortButton { .. }
        ) {
            items.extend(ContextMenuItem::standard_column_header_items());
        }
        items
    }

    fn on_action(
        &self,
        action_id: &str,
        request: &ContextMenuRequest,
        _state: &mut GridState,
        cx: &mut App,
    ) {
        if action_id == "copy-narrative" {
            if let Some(row) = request.clicked_row() {
                if let Some(value) = row.value_by_name("narrative") {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!("{value:?}")));
                }
            }
        }
    }
}

let view = SqllyDataTable::builder(data)
    .context_menu_provider(MyMenuProvider)
    .build(cx);
```

Column-name lookups are case-sensitive; if duplicate names exist, the first
match wins. `GridData::column_index("name")` provides the same lookup outside
the menu context.

## Run the sample

```sh
cargo run -p sqlly-datatable-sample
```

The sample uses `GridData::new` — if you change column count, update the
rows to match.

## Optional: bundle as a macOS `.app`

`bundle.sh` is no longer invoked automatically from `build.rs`. After a
release build (`cargo build -p sqlly-datatable-sample --release`) you can run

```sh
(cd crates/sqlly-datatable-sample && sh bundle.sh)
```

to package the binary as `SqllyDataTableSample.app`.

## License

MIT
