//! Configurable virtualized data grid for the [GPUI] toolkit.
//!
//! `sqlly-datatable` provides a self-contained `Entity<GridState>` widget that
//! you can drop into a GPUI application. It supports:
//!
//! * Virtualized rendering for large datasets.
//! * Cell, row, column and rectangular drag selection.
//! * Sorting on column headers, with a cycle through ascending, descending,
//!   and unsorted.
//! * Per-column filtering via a rich filter panel (operator predicates,
//!   value checklist, search) opened from the built-in context menu.
//! * Expandable row sections created by grouping on any column from its
//!   built-in context menu or through [`GridState::set_grouped_column`].
//! * Configurable column resizing, mouse-driven scrollbars, and edge-scroll
//!   during drag selection.
//! * Clipboard copy of any selection (with or without headers).
//! * A restrained native motion layer: transient surfaces (context menus,
//!   filter panels, popovers, dialogs, the busy scrim, the pivot drag ghost)
//!   fade in on appear, while the data surface stays instant. On by default;
//!   set [`config::GridConfig::animations`] to `false` to honor a system
//!   reduce-motion preference.
//! * An optional **pivot tab** ([`SqllyDataTableBuilder::pivot`]): a
//!   cross-tabulation view with a drag-and-drop field sidebar (rows /
//!   columns / values / filters), count/sum/avg/min/max aggregation,
//!   expandable row and column groups, subtotals and grand totals, sorting
//!   on labels or values, source-value filters, and CSV export. The pivot
//!   reads a shared snapshot of the grid's rows and never mutates them;
//!   switching tabs preserves both views' state. Configure it
//!   programmatically via [`pivot::PivotConfig`] and read the live layout
//!   back from [`pivot::PivotState::config`]. Pivot row height and column
//!   width can be initialized on the builder, resized in the view, and read
//!   or updated through [`pivot::PivotState`]. Right-clicks surface to a
//!   [`pivot::PivotContextMenuProvider`] with full context (grouping paths,
//!   aggregated value, driving source rows), and double-clicking any value
//!   cell drills through: the flat grid is filtered to exactly the rows
//!   behind that cell and brought to the front.
//!
//! The crate is intentionally GPUI-only on the UI side; the pure formatter in
//! [`mod@format`] is usable in any context (export pipelines, server-side preview,
//! etc.). All formatting is configurable per column by composing the
//! [`config::GridConfig`] defaults with [`config::ColumnOverride`] entries.
//!
//! # Quick start
//!
//! ```no_run
//! use gpui::App;
//! use sqlly_datatable::{
//!     CellValue, Column, ColumnKind, GridConfig, GridData, SqllyDataTable,
//! };
//!
//! // Inside your `gpui::Application::new().run(...)` closure (see the
//! // sample app for a full bootstrap):
//! fn setup(cx: &mut App) {
//!     sqlly_datatable::init(cx);
//!
//!     let data = GridData::new(
//!         vec![Column { name: "id".into(), kind: ColumnKind::Integer, width: 80.0 }],
//!         vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]],
//!     ).expect("rectangular data");
//!     let _view = SqllyDataTable::builder(data)
//!         .config(GridConfig::default())
//!         .build(cx);
//! }
//! ```
//!
//! See `crates/sqlly-datatable-sample` for a runnable demo.
//!
//! [GPUI]: https://github.com/zed-industries/gpui

// `missing_docs` is intentionally not enabled at the crate level. The public
// surfaces documented here are stable; private internals will get docs as the
// `grid::` modules mature. Run clippy with
// `#![warn(missing_docs)]` in scope when cleaning up a module.

pub mod config;
pub mod data;
pub mod filter;
pub mod format;
pub mod grid;
pub mod pivot;

// Re-exported so hosts can call `gpui_component` APIs (theme switching,
// `Root`, other widgets) against the exact version this crate links,
// guaranteeing type identity for globals like `gpui_component::Theme`.
pub use gpui_component;
// Re-exported so hosts can install the lucide icon SVGs this crate's chrome
// renders (chevrons, close buttons, panel toggles, checkmarks):
//
// ```no_run
// gpui::Application::new().with_assets(sqlly_datatable::gpui_component_assets::Assets)
// # ;
// ```
//
// Without an asset source providing `icons/*.svg`, those icons render empty
// (with an error logged per icon).
pub use gpui_component_assets;

pub use config::{
    BooleanFormat, ColumnOverride, DateFormat, GridConfig, KeyBinding, KeyBindings, NullFormat,
    NumberFormat, RelativeDateFormat, RelativeUnit, ReplacementRule, ReplacementTiming,
    ResolvedColumnFormat, StringFormat, TextAlignment, TextCase, TruncationBehavior,
};
pub use data::{
    compare_cells, sample_data, CellValue, Column, ColumnKind, GridData, GridDataError,
};
pub use filter::{ColumnFilter, FilterPredicate, NumberOp, TextOp};
pub use grid::{
    BusyState, ColumnContext, ContextMenu, ContextMenuItem, ContextMenuProvider,
    ContextMenuRequest, ContextMenuSelection, ContextMenuTarget, FilterPanel, GridState, GridTab,
    GridTheme, GridThemePair, HitResult, MenuAction, MenuItem, PivotSidebarPosition, RowGroup,
    RowWindow, ScrollbarAxis, SelectedCellContext, SelectedRowContext, Selection, SortDirection,
    SqllyDataTable, SqllyDataTableBuilder,
};
pub use pivot::{
    AggregationFn, PivotCellContext, PivotConfig, PivotContextMenuProvider,
    PivotContextMenuRequest, PivotFormatDialog, PivotGrid, PivotMenuItem, PivotMenuTarget,
    PivotPathComponent, PivotResult, PivotSaveConfigHandler, PivotSidebar, PivotSortKey,
    PivotState, PivotZone, DEFAULT_PIVOT_COLUMN_WIDTH, DEFAULT_PIVOT_ROW_HEIGHT,
    DEFAULT_PIVOT_SIDEBAR_WIDTH, MIN_PIVOT_COLUMN_WIDTH, MIN_PIVOT_ROW_HEADER_WIDTH,
    MIN_PIVOT_ROW_HEIGHT,
};

/// Initialize the toolkit state this crate's widgets depend on. Call once at
/// application startup, before opening any window that hosts a
/// [`SqllyDataTable`]:
///
/// ```no_run
/// fn setup(cx: &mut gpui::App) {
///     sqlly_datatable::init(cx);
///     // ... open windows ...
/// }
/// ```
///
/// This currently forwards to [`gpui_component::init`], which installs the
/// global [`gpui_component::Theme`] used by the embedded `gpui-component`
/// widgets (for example the pivot sidebar's resizable divider). Skipping it
/// panics on first render of those widgets. Hosts that already call
/// `gpui_component::init` themselves do not need to call this again.
pub fn init(cx: &mut gpui::App) {
    gpui_component::init(cx);
}
