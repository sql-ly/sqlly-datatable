//! `GridState` plus the GPUI widget, paint functions, and helpers that swing
//! between them. The flat public re-exports below keep existing consumers of
//! the 0.1.x API working; new code should prefer importing from the canonical
//! `crate::grid::*` paths.

pub mod context_menu;
pub mod menu;
pub mod paint;
pub mod selection;
pub mod state;
pub mod theme;
pub mod widget;

// Flat re-exports so external code can write `use sqlly_datatable::GridState`
// without mapping the internal split.
pub use context_menu::{
    ColumnContext, ContextMenuItem, ContextMenuProvider, ContextMenuRequest, ContextMenuSelection,
    ContextMenuTarget, SelectedCellContext, SelectedRowContext,
};
pub use menu::{ContextMenu, MenuAction, MenuItem};
pub use selection::{HitResult, ScrollbarAxis, Selection, SortDirection};
pub use state::{BusyState, FilterInput, FilterPanel, FilterValueRow, GridState};
pub use theme::GridTheme;
pub use widget::{SqllyDataTable, SqllyDataTableBuilder};

// Inline a couple of constants that callers used to read from the `grid` mod.
pub use state::SCROLLBAR_SIZE;
