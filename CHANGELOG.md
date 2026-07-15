# Changelog

All notable changes to `sqlly-datatable` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.2.0] - 2026-07-15

### Added
- Optional save-configuration button in the pivot controls sidebar, rendered
  as a small disk icon next to the Layout section. It only appears when a
  host wires up the action via `SqllyDataTableBuilder::pivot_save_config`,
  `SqllyDataTable::set_pivot_save_config`, or `PivotState::on_save_config`,
  and invokes the handler with the live `PivotConfig`.
- Sidebar chips (fields, filters, columns, rows, and values) show a hover
  tooltip with the full name whenever their label is chopped.

### Fixed
- Sidebar chip labels no longer extend past the chip boundary: text is
  ellipsized so the remove button — and, for values chips, the aggregation
  picker — always stay visible, even at the minimum sidebar width.

## [2.1.0] - 2026-07-15

### Added
- Flat-grid rows can be grouped by any column from the column-header context
  menu, with expandable and collapsible section headers. Grouping is also
  configurable through `GridState` and `SqllyDataTableBuilder`.

## [2.0.2] - 2026-07-14

### Added
- Collapsible accordion controls for the pivot view, with configurable left or
  right placement and a resizable `gpui-ui-kit` pane divider.
- Mouse-driven pivot row-height and value-column-width resizing. Initial sizes
  can be supplied through `SqllyDataTableBuilder::pivot_row_height` and
  `SqllyDataTableBuilder::pivot_column_width`, then read or updated through
  `PivotState`.

## [2.0.1] - 2026-07-14

### Added
- `SqllyDataTable::set_pivot_locked` keeps the Pivot tab visible while blocking
  activation and optionally displaying host-provided status text beside its
  title. This lets streaming hosts defer pivot snapshot work until all source
  rows have arrived.

## [2.0.0] - 2026-07-14

### Added
- Optional pivot tab, enabled with `SqllyDataTableBuilder::pivot` or at
  runtime through `SqllyDataTable::enable_pivot`. It provides a cross-tab view
  over a shared grid-data snapshot while preserving each view's state.
- Drag-and-drop pivot field sidebar for row, column, value, and filter zones;
  programmatic layout through `PivotConfig`; and `GridTab` APIs for switching
  views.
- Count, sum, average, minimum, and maximum aggregation; expandable row and
  column groups; subtotals and grand totals; sorting; source-value filters;
  CSV export; and value-cell drill-through to the source grid rows.
- Custom pivot context menus via `PivotContextMenuProvider`, including
  grouping paths, aggregated value, and driving source-row context.
- Public pivot API: `PivotState`, `PivotGrid`, `PivotSidebar`, `PivotResult`,
  `PivotConfig`, `PivotZone`, `PivotSortKey`, `AggregationFn`, and supporting
  context-menu types.
- Pivot-specific theme colors for group headers, totals, drop zones, and field
  chips in the light and dark `GridTheme` palettes.

## [1.8.0] - 2026-07-09

### Added
- Configurable null-value display (`NullFormat`): grid-wide default via
  `GridConfig::default_null`, per-column override via `ColumnOverride::null`.
  Built-in default renders italic `NULL` over a distinctive background; new
  `GridTheme::null_fg` / `null_bg` colors in both light and dark palettes.

### Changed
- Zebra striping, row-selection highlight, and horizontal grid lines now stop
  at the last column's right edge; the area past the columns stays blank.

### Fixed
- Scroll offset is re-clamped when the grid's allocated bounds change, so the
  visible rows and scrollbar geometry update immediately on resize instead of
  waiting for the next scroll event.

## [1.7.0] - 2026-07-09

### Added
- Multi-selection: drag across column headers to select a contiguous column
  range; cmd+click column headers, row headers, or individual cells to toggle
  them in and out of a discontiguous selection. A plain click replaces the
  selection with only the clicked item.
- New `Selection` variants: `Columns(Vec<usize>)`, `Rows(Vec<usize>)`, and
  `Cells(Vec<(usize, usize)>)`.
- Public accessors for host applications: `GridState::selected_rows`,
  `GridState::selected_columns`, and `GridState::selected_cells` return the
  resolved selection clamped to the current data dimensions.
- `GridState::handle_mouse_down_with_modifiers` carrying the cmd (platform)
  modifier; `handle_mouse_down` is unchanged for compatibility.

## [1.6.1] - 2026-07-09

### Added
- `CHANGELOG.md` documenting the full release history from v1.0.0 onward.

## [1.6.0] - 2026-07-08

### Added
- Windowed-row (virtual rows) mode for large datasets — only visible rows are
  painted, keeping frame times flat as row counts grow.

## [1.5.1] - 2026-07-07

### Changed
- Patch release.

## [1.5.0] - 2026-07-07

### Added
- `GridState::append_rows` streaming fast path. Appends rows in place —
  extends `data.rows`, the paint-path `data_rows` snapshot, and
  `display_indices` in O(new rows) when no sort/filter is active (recompute
  otherwise), preserving selection and scroll. Lets hosts paint query
  results as batches arrive instead of rebuilding the grid entity per batch
  (O(n²) over a stream).

## [1.4.2] - 2026-07-07

### Fixed
- Clipped grid painting to bounds.
- Added scrollbar gutter separators.

## [1.4.1] - 2026-07-04

### Added
- Exposed `ColumnContext` + added `ContextMenuRequest::for_test` constructor.

## [1.4.0] - 2026-07-03

### Changed
- `ContextMenuRequest` is now lazy (Arc-backed): building it on right-click
  is O(1) regardless of selection size, eliminating large-selection lag.
  **BREAKING:** accessors return owned values and fields are private. Adds
  `selected_cell_count` / `selected_row_count` helpers.

### Added
- Background task support: `BusyState { label, progress }` with
  `set_busy` / `set_busy_progress` / `clear_busy` / `is_busy` / `busy`
  helpers; determinate progress bar or animated indeterminate bar; blocks
  input while busy.
- Sample: menu labels use O(1) counts; "Copy selection to CSV" runs off
  thread via `spawn_background` and shows the loading overlay.

## [1.3.1] - 2026-07-03

### Changed
- Filter panel: auto-apply only (removed Auto Apply checkbox + Apply
  button), funnel filter icon, disabled Clear Filter when inactive, sort
  toggle-off, panel opens above column header, search no longer unchecks
  Select All.

### Performance
- Arc-wrap `data_rows` + `display_indices` (O(1) per-frame paint clones).
- Skip per-row snapshots for column-oriented context-menu targets (~11x
  faster right-click on wide/tall grids).

### Added
- Sample: 100k rows.

## [1.3.0] - 2026-07-03

### Added
- Rich per-column filter panel with operator predicates and value checklist.

## [1.2.1] - 2026-07-03

### Added
- Toggleable debug bar (off by default). `debug_bar_enabled` flag on
  `GridState` controls whether the status bar showing click position,
  scroll offset, and hovered cell is painted. Enable at build time via
  `SqllyDataTableBuilder::debug_bar(true)` or at runtime via
  `GridState::set_debug_bar_enabled(true)`.

### Fixed
- Sort-button clicks no longer change selection (only toggle sort).

## [1.2.0] - 2026-07-02

### Added
- Automatic OS light/dark theming: `GridTheme::light()` / `dark()` /
  `for_appearance()` palettes; the widget sets the initial theme from
  `window.appearance()` and observes appearance changes to swap the theme
  live. Builder `.theme()` now applies an override (and opts out of
  OS-following).
- Sort direction buttons now match the column header styling.

## [1.1.6] - 2026-07-02

### Added
- Deferred + anchored context-menu overlay.

## [1.1.5] - 2026-07-02

### Changed
- Style: applied `cargo fmt`.

## [1.1.4] - 2026-07-01

### Added
- Window-scoped context menu positioning: menu can render beyond the grid
  area but stays within the host application window; opens down by
  default, flips up when there is no room below.
- Slower, gentler edge-scroll acceleration: 3 bands spaced 30px (90px
  trigger zone) at 0.25 / 0.5 / 1 px/tick.
- Sample: 40 columns × 2000 rows of deterministic data, expanded
  right-click context menus, `run-sample.sh` helper.

## [1.1.3] - 2026-07-01

### Added
- Themed context-menu colors + visible hover state.

## [1.1.2] - 2026-07-01

### Fixed
- Normalized grid pointer coords to grid-relative frame.

## [1.1.1] - 2026-07-01

### Changed
- Bumped to v1.1.1 + clippy allow in integration tests.

## [1.1.0] - 2026-06-30

### Added
- `ContextMenuProvider` trait for custom right-click menus on cells, row
  headers, and column headers. Provider receives an owned
  `ContextMenuRequest` snapshot with display/source row mapping, selected
  cells, selected rows, and name-based value lookups. Built-in
  column-header menu preserved when no provider is registered; providers
  can compose built-ins via `standard_column_header_items()`.

## [1.0.2] - 2026-06-29

### Fixed
- Cargo package path-dep version.

## [1.0.0] - 2026-06-29

### Added
- Tooling, correctness fixes, modular split, tests, CI publish.
