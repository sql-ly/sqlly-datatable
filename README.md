# sqlly-datatable

## Why

I discovered [GPUI](https://github.com/zed-industries/gpui) while building [sqlly.app](https://sqlly.app) and immediately loved it. It is fast, the immediate-mode canvas model is a joy to work in, and the codebase is a pleasure to read. GPUI is what made me decide to port sqlly from a Mac-only app to a fully cross-platform tool.

What I discovered along the way is that building a real SQL management client on top of GPUI requires a lot of little tweaks that GPUI doesn't ship with out of the box — virtualized data grids, drag selection, column resizing, scrollbars, configurable formatting, context menus, the works. So I built them.

I am not a skilled Rust developer. I have written custom UI controls in several other languages over the years, but I built this project with AI assistance. I cannot attest to the quality of the code. I am sharing it freely in the hope that people more skilled than I am will improve it — or that the GPUI maintainers see something here worth pulling upstream.

If even one part of this makes a GPUI contributor think "huh, that's a useful pattern" rather than "wow, what a bunch of AI slop," the project has served its purpose.

## What

A configurable data grid component for GPUI, built for the needs of [sqlly.app](https://sqlly.app).

## Features

- **Virtualized rendering** — only visible rows/columns are painted, handling thousands of rows efficiently
- **Cell selection** — single cells, row ranges, column ranges, and click-drag rectangle selection
- **Sorting** — click column headers or sort buttons to cycle ascending/descending/off
- **Filtering** — per-column filter prompt via right-click context menu
- **Column resizing** — drag column borders to resize
- **Scrollbars** — horizontal and vertical, with scroll clamping and edge-scroll during drag
- **Context menu** — right-click column headers for sort, copy, filter actions
- **Keyboard navigation** — arrow keys, page up/down, select all, copy/copy-with-headers
- **Status bar** — shows click position, scroll offsets, cell coordinates, and hover info

## Configuration

All formatting and behavior is externally configurable via `GridConfig` with per-column overrides:

- **Numbers** — decimal places, negative red, parentheses, thousands separators, alignment
- **Dates** — format string (`%Y-%m-%d`), timezone offset, natural language relative dates ("2 days ago", "in 3 weeks") with configurable precision
- **Booleans** — custom true/false text, alignment
- **Strings** — case (upper/lower/title), max length, truncation (ellipsis/cutoff/wrap), alignment
- **Replacement rules** — find/replace pairs applied before or after formatting
- **Key bindings** — select all, copy, copy with headers, page up/down, context menu modifier

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
use sqlly_datatable::{SqllyDataTable, GridConfig, GridData, ColumnKind};

let data = GridData::new(/* columns, rows */);
let config = GridConfig::default();

let view = SqllyDataTable::builder(data)
    .config(config)
    .build(cx);

// Use in a GPUI window
cx.open_window(options, move |_window, cx| {
    cx.new(|_cx| SqllyDataTable::new(view.state.clone()))
});
```

### Per-column overrides

```rust
use sqlly_datatable::{GridConfig, ColumnOverride, NumberFormat};

let mut config = GridConfig::default();
config.column_overrides = vec![
    ColumnOverride {
        number: Some(NumberFormat { decimals: 0, ..Default::default() }),
        ..Default::default()
    },
    // ... per column
];
```

## Run the sample

```sh
cargo run -p sqlly-datatable-sample
```

## License

MIT
