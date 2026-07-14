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
//! * Configurable column resizing, mouse-driven scrollbars, and edge-scroll
//!   during drag selection.
//! * Clipboard copy of any selection (with or without headers).
//! * An optional **pivot tab** ([`SqllyDataTableBuilder::pivot`]): a
//!   cross-tabulation view with a drag-and-drop field sidebar (rows /
//!   columns / values / filters), count/sum/avg/min/max aggregation,
//!   expandable row and column groups, subtotals and grand totals, sorting
//!   on labels or values, source-value filters, and CSV export. The pivot
//!   reads a shared snapshot of the grid's rows and never mutates them;
//!   switching tabs preserves both views' state. Configure it
//!   programmatically via [`pivot::PivotConfig`] and read the live layout
//!   back from [`pivot::PivotState::config`]. Right-clicks surface to a
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
//! let data = GridData::new(
//!     vec![Column { name: "id".into(), kind: ColumnKind::Integer, width: 80.0 }],
//!     vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]],
//! ).expect("rectangular data");
//! let app = gpui::Application::new();
//! app.run(|cx: &mut App| {
//!     let _view = SqllyDataTable::builder(data)
//!         .config(GridConfig::default())
//!         .build(cx);
//! });
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
    GridTheme, HitResult, MenuAction, MenuItem, RowWindow, ScrollbarAxis, SelectedCellContext,
    SelectedRowContext, Selection, SortDirection, SqllyDataTable, SqllyDataTableBuilder,
};
pub use pivot::{
    AggregationFn, PivotCellContext, PivotConfig, PivotContextMenuProvider,
    PivotContextMenuRequest, PivotGrid, PivotMenuItem, PivotMenuTarget, PivotPathComponent,
    PivotResult, PivotSidebar, PivotSortKey, PivotState, PivotZone,
};
