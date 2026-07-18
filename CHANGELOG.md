# Changelog

All notable changes to `sqlly-datatable` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [4.1.0] - 2026-07-17

### Changed
- **Back on crates.io.** The workspace now depends on the registry releases
  of the UI stack — `gpui = "0.2"` (0.2.2) and `gpui-component = "0.5"` /
  `gpui-component-assets = "0.5"` — instead of the zed/longbridge git
  branches the 4.0.x migration pinned. crates.io rejects git dependencies,
  which is why 4.0.x could not be published; with compatible registry
  releases now available, `cargo package` works again and the CI publish job
  (tag-triggered) is restored.
- Binaries bootstrap via plain `gpui::Application::new()` again — the
  registry `gpui` bundles the OS windowing backends, so the zed-main-only
  `gpui_platform` shim dependency is gone.
- Code adjusted for the small API deltas between the git pins and the
  registry releases (gpui 0.2.2 / gpui-component 0.5.1); no behavior or
  public-API changes intended.

### Removed
- The experimental **web (wasm) build of the sample app** introduced in
  4.0.0 (`build-wasm.sh`, the `web::run` entry point, and the CI `web-app`
  artifact). It required gpui's web backend, which exists only on zed's git
  `main` — reinstate from git history when a gpui registry release ships the
  web backend.

## [4.0.1] - 2026-07-17

### Changed
- Version bump only, re-tagging `master` after the gpui-component migration
  merge; no functional changes.

## [4.0.0] - 2026-07-17

### Changed
- **Breaking:** migrated the UI toolkit dependency from `gpui-ui-kit` to
  [`gpui-component`](https://github.com/longbridge/gpui-component), and moved
  `gpui` from the crates.io `0.2` release to Zed's git `main` (which
  `gpui-component` tracks). Both are now consumed as git dependencies; the
  minimum Rust version is 1.96.
- **Breaking:** hosts must call `sqlly_datatable::init(cx)` once at startup
  (or call `gpui_component::init` themselves). It installs the global
  `gpui_component::Theme` read by the embedded toolkit widgets.
- **Breaking:** binaries now bootstrap via `gpui_platform::application()`
  (Zed's `main` removed `gpui::Application::new()`); see the sample app.
- The pivot sidebar split is now a `gpui-component` resizable panel group.
  Drag-to-resize is handled by the library's resize handle (the old
  hand-rolled drag tracking is gone); the sidebar collapses via a dedicated
  click strip on its inner edge (previously double-click on the divider), and
  the collapsed state keeps the labelled expand rail. `set_pivot_sidebar_*`
  APIs are unchanged.

### Added
- `gpui-component` theme bridge: `GridTheme::from_component_theme` /
  `GridTheme::from_component_colors` derive a `GridTheme` from a
  `gpui_component::Theme`, and `GridThemePair::component()` is a ready-made
  light/dark family built from the toolkit's default palettes. The sample
  app's switcher gained a third "Component" choice demonstrating it.
- `pub use gpui_component;` re-export, so hosts can target the exact toolkit
  version this crate links against.
- **WebAssembly build of the sample app.** `./build-wasm.sh` compiles the
  sample to `wasm32-unknown-unknown` (nightly; gpui's web backend requires
  it), runs `wasm-bindgen`, and packages a self-contained runnable site
  (index.html + JS glue + wasm) into `dist/sqlly-datatable-web-v<version>.zip`.
  CI builds and uploads the zip as a `web-app` artifact. The sample crate is
  now a library (`init_and_open`) with two entry points: the native binary
  and a `wasm-bindgen` `web::run` export using gpui's `run_embedded`.
- New sample dataset: six months (2026-01-01 → 2026-06-30) of the general
  ledger of a fictitious US burger chain — 60,000 records across 3 regions ×
  10 locations, 500 staff, cost centers, menu/supply items, and payment
  methods. Sales rows carry receipt number
  (`{region:4}{location:4}{yyyyMMdd}{seq:08}`, sequential per location per
  day), tax / sub-total / coupon / total, and a customer id; purchase rows
  carry vendor / invoice / purchase order; payroll rows only an amount.
  After the ledger, six literal marker columns (`extra`, `columns`, `to`,
  `show`, `horizontal`, `virturalizion` — header repeated as every value)
  precede 100 randomized columns cycling datetime / text / decimal /
  integer / boolean, exercising horizontal virtualization at 124 columns.
- **Pivot row-label resize.** The right edge of the pivot's row-label area
  drags to resize it (new `PivotHitResult::RowHeaderBorder`,
  `PivotState::set_row_header_width` / `row_header_area_width`,
  `MIN_PIVOT_ROW_HEADER_WIDTH`).
- **Drill-through banner.** When a pivot drill-through filters the flat
  grid, the Grid tab shows a banner naming the applied per-column filters
  with a one-click "Clear filter" (`SqllyDataTable::drill_filter` /
  `clear_drill_filter`).
- **Lucide icons.** All element-based chrome glyphs (sidebar accordion
  carets, chip remove buttons, the aggregation picker chevron, checkbox
  checkmarks, and the sidebar collapse rail/strip) now render
  `gpui-component`'s lucide SVG icons instead of font glyphs, and the
  canvas-painted pivot group carets and the header filter marker are drawn
  as vector paths. Rationale: the web build's embedded fonts have no
  coverage for ▶ ▼ ✕ ✓ 🔽, which rendered as boxes. Hosts must provide the
  icon SVGs via `Application::with_assets` — re-exported as
  `sqlly_datatable::gpui_component_assets` (embedded natively; fetched from
  `icons/` relative to the site root on the web, which `build-wasm.sh` now
  bundles into the zip).

### Changed (sample app)
- The sample now defaults to the **Component** theme family (the
  `gpui-component` bridge palette); Neutral and Signature remain in the
  switcher.

### Fixed
- Two web-only panics: the grid/pivot painters clamped with an inverted
  range when the first frame has zero-sized bounds, and date formatting
  called `std::time::SystemTime::now()`, which is unimplemented on
  `wasm32-unknown-unknown` (now reads the clock via `web-time` there).
- Web icons: the wasm asset fetcher requests `{endpoint}/assets/icons/*.svg`
  and rejects relative endpoints (reqwest requires absolute URLs on wasm),
  so the pivot sidebar's icons never loaded. `build-wasm.sh` now bundles the
  SVGs under `assets/icons/` and the sample derives an absolute endpoint
  from the page's base URI at runtime.

## [3.1.3] - 2026-07-17

### Added
- A restrained, native **motion layer**, on by default. Transient surfaces —
  context menus, filter panels, popovers, the per-field format dialog, the
  busy scrim, the pivot drag ghost, and sidebar accordion bodies — fade in on
  appear (opacity only, ~110 ms, ease-out) instead of snapping into existence.
  The data surface itself (cells, selection, hover, sort) stays instant by
  design. One shared vocabulary drives every surface, so the whole crate moves
  the same way.
- `GridConfig::animations` (default `true`) to opt out. GPUI exposes no OS
  reduce-motion signal, so this flag is the accessibility control: set it
  `false` and every surface appears instantly. The pivot mirrors the flag from
  the host grid at build time.

### Changed
- **Breaking (niche):** `GridConfig` gains an `animations: bool` field, which
  breaks full struct-literal construction; use `..GridConfig::default()`.
- Design consistency pass across the pivot and filter chrome: a single shared
  checkbox affordance (previously hand-rolled at three sizes), a shared
  cell-text inset between the flat grid and the pivot, hover states and
  rounding on filter-checklist rows, source-list field chips that recede to a
  quiet surface (the accent stays reserved for placed fields, totals, and
  selection), and tightened sidebar spacing.

## [3.1.2] - 2026-07-17

### Added
- Pivot **flat (tabular) row layout**. With multiple row fields, the
  nested/indented hierarchy can be switched to a flat table: one row per
  innermost combination, each row field in its own row-header column, with no
  group-header rows, indentation, or per-level subtotals. Off by default —
  toggle "Flat rows (no hierarchy)" in the sidebar's Layout section, or set
  `PivotConfig::flat_rows`.

### Changed
- **Breaking (niche):** `PivotConfig` gains a `flat_rows: bool` field, which
  breaks full struct-literal construction; use `..PivotConfig::default()`.

## [3.1.1] - 2026-07-17

### Fixed
- Pivot sidebar: scrolling the field list (or the filter popover's value
  list) no longer drags the entire control panel with it. GPUI's
  overflow-scroll handler doesn't stop wheel-event propagation, so the outer
  panel — also under the cursor — scrolled in lockstep with the inner list;
  the inner lists now claim the wheel within their own bounds (a `BlockMouse`
  hitbox that keeps the sidebar out of the scroll hit-test).

## [3.1.0] - 2026-07-17

### Added
- Full keyboard navigation for the flat grid: **Home**/**End** jump to the
  first/last column of the active row, **PageUp**/**PageDown** move by one
  viewport of rows, each with **Shift** to extend the selection. Every
  keyboard move now scrolls the active cell into view, so arrowing (or
  paging) past the fold no longer strands the selection off-screen.
- Focus-visible ring: a 1px accent frame (themable via
  `GridTheme::sort_indicator`) is painted around the grid while it holds
  keyboard focus, so a keyboard user can always see where input goes
  (WCAG 2.4.7).
- `tests/keyboard_nav.rs`: exercises the public `handle_key` path and
  asserts the active cell is scrolled into view on arrow / PageUp/Down /
  Home / End, plus Shift-extend to the row extremes.
- Color-blind-safety invariant test: a negative number always carries a
  non-color sign channel in its text (leading `-` or `( … )`) regardless of
  the decorative `show_negative_red` flag (WCAG 1.4.1).

### Fixed
- Zebra striping was effectively invisible in all four shipped palettes
  (~1.06 contrast against the base row). The alternating band is deepened to
  a genuinely perceptible ~1.16 so a row can be tracked across a wide
  horizontal scroll; body text stays ≥ 12.8:1 on the band. The WCAG test's
  zebra floor is raised (1.02 → 1.12) to lock the band in.
- Row numbers right-align against the frozen gutter (the Excel / Numbers
  convention) instead of floating flush-left.
- Column headers echo their column's data alignment — a numeric column's
  label now sits right-aligned over its right-aligned values — while
  reserving the hover/sort button gutter (and the filter funnel when the
  column is filtered).
- Right- and center-aligned cells, headers, and the empty-state hint are
  positioned by the text's true shaped width, so double-width glyphs (CJK,
  emoji) align correctly instead of drifting by the monospace estimate.
- Appearance-following reconciles the resolved theme against the window
  appearance on every render, self-healing a stale first-frame appearance
  read that could otherwise strand the grid in the light variant.

### Changed
- **Breaking (niche):** `GridState::resolved_formats` is now
  `Arc<Vec<ResolvedColumnFormat>>` (was `Vec<ResolvedColumnFormat>`) so the
  once-per-frame paint snapshot clones a pointer instead of deep-copying
  every column's format. Indexing (`state.resolved_formats[i]`), `.len()`,
  and `.iter()` are unchanged via `Deref`; only code that reassigned the
  field or iterated `&state.resolved_formats` directly needs adjusting
  (`Arc::new(…)` / `.iter()`).
- Documented the negative-number accessibility contract on `NumberFormat`:
  `show_negative_red` is decorative, and the sign glyph (leading minus, or
  parentheses via `negative_parentheses`) is the color-blind-safe channel.

## [3.0.2] - 2026-07-16

### Changed
- The active-filter indicator is the 🔽 emoji in both views (the grid's
  hand-drawn funnel next to the sort button, and the `●` marker on the
  pivot sidebar's Filters-zone chips), rendered via the system color-emoji
  fallback.
- The pivot sidebar's Display and export section is easier to read and hit:
  16px checkboxes with an accent-filled ✓ checked state (knockout check in
  the theme's background color), 13px row labels with hover feedback and a
  larger click target, 12px buttons with real padding and hover states, and
  12px section labels.

## [3.0.1] - 2026-07-16

### Added
- `GridConfig::empty_text`: a hint painted centered in the data area when
  the grid has zero rows (default "No rows"). Host-supplied so it can be
  localized; set to an empty string to paint nothing. Note: adding this
  field is breaking for full struct-literal construction of `GridConfig`
  (use `..GridConfig::default()`).
- Hardening test suite (`tests/hardening.rs`): hostile text (emoji ZWJ
  sequences, CJK, RTL, 500-char values), `NaN`/`±inf` decimals, extreme
  integers, empty result sets, and zero-column frames all survive sort,
  filter, select-all, copy, and pointer sweeps without panicking.
- The sample app grows deliberately hostile narrative values (emoji, CJK,
  Arabic, an over-long label) and a `SQLLY_SAMPLE_ROWS` env override
  (`SQLLY_SAMPLE_ROWS=0` shows the empty-result state).

### Fixed
- Cell-text truncation slices are clamped to UTF-8 char boundaries in both
  painters; a mid-character byte index from the shaper would previously
  have panicked the paint pass on multi-byte input.
- The flat grid's quad painter skips zero/negative-size quads (parity with
  the pivot painter) instead of handing degenerate geometry to the renderer.

## [3.0.0] - 2026-07-16

### Added
- Two shipped theme families, each a light/dark `GridThemePair` that follows
  the OS window appearance: **Neutral** (chroma-free surfaces, restrained
  azure accent — the default) and **Signature** (teal-anchored tinted
  neutrals with a committed accent). All four palettes are designed in OKLCH
  and contrast-verified in unit tests: every text role meets WCAG AA
  (≥ 4.5:1) against every surface it is painted on.
- `GridThemePair` (exported at the crate root): a light/dark pair with
  `neutral()`, `signature()`, and `for_appearance(..)`. Hosts can supply a
  custom pair and keep automatic light/dark following.
- `SqllyDataTableBuilder::theme_family(..)` to pick the family at build time
  and `SqllyDataTable::set_theme_family(..)` to swap it at runtime; the
  matching variant is applied immediately and future OS appearance changes
  resolve against the new family. `GridState::theme_family` holds the pair.
- New `GridTheme` fields: `scrollbar_thumb` (was a hardcoded constant in the
  grid and pivot painters) and `overlay_scrim` (was a hardcoded translucent
  black behind the pivot format dialog). No color painted by the widget is
  hardcoded anymore.
- The sample app toolbar (replacing the placeholder 500px panel) with a
  Neutral/Signature theme switcher; the sample defaults to Signature.

### Changed
- `GridTheme::default()`/`light()`/`dark()` now alias the Neutral family
  palettes. Selection fills are opaque in all shipped palettes (previously
  50% alpha), so text contrast on selections is deterministic.
- Grid column-header labels and flat-grid group-header rows paint bold,
  matching the pivot's total styling, so structure leads the hierarchy.
- Sort buttons are quiet at rest: the outlined button and `-` cycle hint
  appear only while the column header is hovered, and a sorted column shows
  a bold accent `↑`/`↓` glyph instead. Hit targets are unchanged.
- The pivot's sort targets (innermost column headers, subtotal and
  grand-total columns, and the corner's row-label sort) now use the same
  affordance as the flat grid: a `-` hint on hover and a bold accent
  `↑`/`↓` glyph when sorted, replacing the plain `^`/`v` appended to the
  label text. The column-label sort (from the context menu) shows its glyph
  next to the column field caption.
- Sort arrows, hover hints, and the filter funnel paint one-third larger
  than cell text so state reads at a glance.
- Pivot sidebar field chips have a fixed 24px height and never flex-shrink;
  previously the scrollable list compressed each chip unevenly, so the
  nominal 3px gaps rendered as anything from 2px to 4px. Chip gaps are now
  a uniform 4px, with an 8px rhythm between sidebar sections.
- The drag-selection marquee paints a 1px accent outline (previously a fully
  transparent quad, i.e. invisible).
- Sample app: semibold toolbar title, wider default integer/boolean columns
  so header labels fit the bold face, and a corrected selection-summary
  menu line (it previously printed the cell count as the column count).

### Fixed
- Canvas-painted text (cells, headers, pivot, status bar) now requests a
  real monospace family per platform — Menlo on macOS, Consolas on Windows,
  DejaVu Sans Mono elsewhere, each with cross-platform fallbacks — instead
  of the generic `"monospace"`, which resolved to a single-face family on
  macOS and silently dropped the bold and italic variants. The pivot's bold
  grand totals (added in 2.3.0) and italic null placeholders now actually
  render, and the default `char_width` approximations derive from the real
  font's advance metrics.

### Deprecated
- `grid::menu::background()` — menu chrome is themed; use
  `GridTheme::menu_bg`.

## [2.3.0] - 2026-07-15

### Added
- Double-clicking any Rows / Columns / Filters / Values chip in the pivot
  sidebar opens a per-field format dialog: negative numbers in red, thousands
  separator, minus sign vs. parentheses for negatives, decimal count, and
  left/center/right alignment. Edits apply live; Reset reverts the field to
  its resolved default.
- Format edits are stored on `PivotConfig` (`field_formats` for label fields,
  the existing `value_format` for value cells), so hosts that persist the
  config — for example via the sidebar's save button — get the formats back
  when they pass the config into a fresh widget or a new data load. Axis
  group labels honor the configured alignment and paint negative numeric
  labels red; `PivotState::label_format` exposes the effective per-field
  format.

### Changed
- Pivot value cells are always right-aligned and always paint negative
  numbers red, regardless of the source column's kind or format. An explicit
  `PivotConfig::value_format` override can still choose otherwise.
- The pivot's grand-total row and grand-total column (cells, row label, and
  column header) are painted bold.

## [2.2.1] - 2026-07-15

### Changed
- The pivot sidebar's save-configuration button moved into the Layout section
  header, directly next to its title. The sidebar's collapsible sections are
  now rendered in-crate (visually unchanged) because the `gpui-ui-kit`
  accordion header cannot host extra controls.

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
