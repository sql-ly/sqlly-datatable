//! Pivot table (cross-tabulation) support.
//!
//! Split like the flat grid: a pure computation layer with no GPUI
//! dependency ([`mod@aggregation`], [`mod@config`], [`mod@engine`]) and a GPUI
//! layer ([`mod@state`], `paint`, [`mod@widget`], [`mod@sidebar`]) that renders it.
//!
//! Enable the pivot tab on a grid via
//! [`crate::SqllyDataTableBuilder::pivot`]; read or write the live layout
//! through [`crate::pivot::state::PivotState::config`].

pub mod aggregation;
pub mod config;
pub mod context_menu;
pub mod engine;
pub(crate) mod paint;
pub mod sidebar;
pub mod state;
pub mod widget;

pub use aggregation::{aggregate, Accumulator, AggregationFn};
pub use config::{PivotConfig, PivotZone};
pub use context_menu::{
    PivotCellContext, PivotContextMenuProvider, PivotContextMenuRequest, PivotMenuItem,
    PivotMenuTarget, PivotPathComponent, PIVOT_ACTION_COPY_CSV, PIVOT_ACTION_COPY_VALUE,
    PIVOT_ACTION_SHOW_SOURCE_ROWS,
};
pub use engine::{compute_pivot, PivotNode, PivotResult, TOTAL_KEY};
pub use sidebar::PivotSidebar;
pub use state::{
    PivotFilterPopover, PivotHitResult, PivotSaveConfigHandler, PivotSortKey, PivotState,
    VisibleCol, VisibleColKind, VisibleRow, VisibleRowKind, DEFAULT_PIVOT_COLUMN_WIDTH,
    DEFAULT_PIVOT_ROW_HEIGHT, DEFAULT_PIVOT_SIDEBAR_WIDTH, MIN_PIVOT_COLUMN_WIDTH,
    MIN_PIVOT_ROW_HEIGHT,
};
pub use widget::PivotGrid;
