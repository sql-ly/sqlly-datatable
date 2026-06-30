//! Configurable virtualized data grid for the [GPUI] toolkit.
//!
//! `sqlly-datatable` provides a self-contained `Entity<GridState>` widget that
//! you can drop into a GPUI application. It supports:
//!
//! * Virtualized rendering for large datasets.
//! * Cell, row, column and rectangular drag selection.
//! * Sorting on column headers, with a cycle through ascending, descending,
//!   and unsorted.
//! * Per-column text-based filtering via a built-in context menu and filter
//!   prompt.
//! * Configurable column resizing, mouse-driven scrollbars, and edge-scroll
//!   during drag selection.
//! * Clipboard copy of any selection (with or without headers).
//!
//! The crate is intentionally GPUI-only on the UI side; the pure formatter in
//! [`format`] is usable in any context (export pipelines, server-side preview,
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
pub mod format;
pub mod grid;

pub use config::{
    BooleanFormat, ColumnOverride, DateFormat, GridConfig, KeyBinding, KeyBindings, NumberFormat,
    RelativeDateFormat, RelativeUnit, ReplacementRule, ReplacementTiming, ResolvedColumnFormat,
    StringFormat, TextAlignment, TextCase, TruncationBehavior,
};
pub use data::{
    compare_cells, sample_data, CellValue, Column, ColumnKind, GridData, GridDataError,
};
pub use grid::{
    ContextMenu, ContextMenuItem, ContextMenuProvider, ContextMenuRequest, ContextMenuSelection,
    ContextMenuTarget, GridState, GridTheme, HitResult, MenuAction, MenuItem, ScrollbarAxis,
    SelectedCellContext, SelectedRowContext, Selection, SortDirection, SqllyDataTable,
    SqllyDataTableBuilder,
};
