//! Public right-click extensibility for the pivot view.
//!
//! Mirrors the flat grid's [`crate::grid::ContextMenuProvider`] pattern:
//! consumers implement [`PivotContextMenuProvider`], register it via
//! [`crate::grid::SqllyDataTableBuilder::pivot_context_menu_provider`], and
//! fully control the pivot's right-click menu. The provider receives a
//! [`PivotContextMenuRequest`] snapshot describing exactly what was clicked —
//! the row/column group paths, the aggregated value, and the source rows
//! that drive it.
//!
//! When no provider is registered the pivot shows a small built-in menu
//! ("Show source rows in Grid", "Copy value", "Copy pivot as CSV"). The
//! built-in action ids ([`PIVOT_ACTION_SHOW_SOURCE_ROWS`],
//! [`PIVOT_ACTION_COPY_VALUE`], [`PIVOT_ACTION_COPY_CSV`]) are always
//! handled by the pivot itself, so providers can compose them into their own
//! menus by returning items with those ids.

use std::fmt;
use std::sync::Arc;

use crate::data::CellValue;
use crate::pivot::aggregation::AggregationFn;
use crate::pivot::config::PivotConfig;
use crate::pivot::state::PivotState;

/// Built-in action id: filter the flat grid to this cell's driving source
/// rows and switch to the Grid tab (same as double-clicking the cell).
pub const PIVOT_ACTION_SHOW_SOURCE_ROWS: &str = "pivot.show-source-rows";
/// Built-in action id: copy the clicked cell's formatted value.
pub const PIVOT_ACTION_COPY_VALUE: &str = "pivot.copy-value";
/// Built-in action id: copy the fully expanded pivot as CSV.
pub const PIVOT_ACTION_COPY_CSV: &str = "pivot.copy-csv";

/// One level of a row/column grouping path, outermost first.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotPathComponent {
    /// Source column index of the grouping field.
    pub field_index: usize,
    /// Source column name of the grouping field.
    pub field_name: String,
    /// The formatted group label at this level (or the configured blank
    /// label).
    pub label: String,
    /// The raw grouping value the group was built from.
    pub group_value: CellValue,
    /// `true` when this group is the null/"(blank)" bucket.
    pub is_blank: bool,
}

/// Full context for a right-clicked pivot value cell.
#[derive(Clone, Debug)]
pub struct PivotCellContext {
    /// Row grouping path (empty on the grand-total row, or when no row
    /// fields are assigned).
    pub row_path: Vec<PivotPathComponent>,
    /// Column grouping path (empty on the grand-total column, or when no
    /// column fields are assigned).
    pub col_path: Vec<PivotPathComponent>,
    /// The aggregated value at this intersection.
    pub value: CellValue,
    /// The value as the pivot displays it.
    pub formatted_value: String,
    /// `true` when the cell sits on the grand-total row.
    pub is_row_grand_total: bool,
    /// `true` when the cell sits in the grand-total column.
    pub is_col_grand_total: bool,
    /// `true` when the cell is a row group subtotal (a group-header or
    /// collapsed-group line rather than an innermost row).
    pub is_row_subtotal: bool,
    /// `true` when the cell is a column subtotal ("Total" column or a
    /// collapsed column group).
    pub is_col_subtotal: bool,
}

/// What was right-clicked in the pivot.
#[derive(Clone, Debug)]
pub enum PivotMenuTarget {
    /// A value cell (including subtotal and grand-total cells).
    Cell(PivotCellContext),
    /// A row label. `path` identifies the group; empty on the grand-total
    /// row.
    RowHeader {
        /// Grouping path of the clicked row.
        path: Vec<PivotPathComponent>,
        /// `true` for the grand-total row label.
        is_grand_total: bool,
    },
    /// A column header. `path` identifies the group; empty on the
    /// grand-total column.
    ColHeader {
        /// Grouping path of the clicked column.
        path: Vec<PivotPathComponent>,
        /// `true` for the grand-total column header.
        is_grand_total: bool,
    },
    /// The top-left corner block.
    Corner,
}

impl PivotMenuTarget {
    /// The clicked cell context, when the target is a value cell.
    #[must_use]
    pub fn cell(&self) -> Option<&PivotCellContext> {
        match self {
            Self::Cell(c) => Some(c),
            _ => None,
        }
    }
}

/// Snapshot of the right-click context, captured at menu-open time. Owned
/// data only — a clone can be moved into background work.
#[derive(Clone, Debug)]
pub struct PivotContextMenuRequest {
    /// What was clicked.
    pub target: PivotMenuTarget,
    /// The active aggregation function.
    pub aggregation: AggregationFn,
    /// Source column index of the value field, if assigned.
    pub value_field_index: Option<usize>,
    /// Header caption for the value area, e.g. `"Sum of Amount"`.
    pub value_caption: String,
    /// The full pivot configuration at menu-open time.
    pub config: PivotConfig,
    /// Indices into the grid's source rows that drive the clicked target:
    /// the rows aggregated into the clicked cell (for cells), the rows of
    /// the clicked group (for row/column headers), or every row passing the
    /// pivot's source filters (for the corner).
    pub source_row_indices: Vec<usize>,
}

impl PivotContextMenuRequest {
    /// The clicked cell context, when the right-click landed on a value
    /// cell.
    #[must_use]
    pub fn clicked_cell(&self) -> Option<&PivotCellContext> {
        self.target.cell()
    }

    /// Number of source rows driving the clicked target.
    #[must_use]
    pub fn source_row_count(&self) -> usize {
        self.source_row_indices.len()
    }
}

/// A menu item returned by a [`PivotContextMenuProvider`].
#[derive(Clone, Debug, PartialEq)]
pub enum PivotMenuItem {
    /// An action with a consumer-defined `id` and display label. Items using
    /// the built-in `pivot.*` action ids are handled by the pivot itself.
    Action {
        /// Identifier passed back to [`PivotContextMenuProvider::on_action`].
        id: String,
        /// Display label.
        label: String,
    },
    /// A visual separator.
    Separator,
}

impl PivotMenuItem {
    /// Convenience constructor for an action item.
    #[must_use]
    pub fn action(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::Action {
            id: id.into(),
            label: label.into(),
        }
    }

    /// Convenience constructor for a separator.
    #[must_use]
    pub fn separator() -> Self {
        Self::Separator
    }

    /// The built-in "Show source rows in Grid" item (drill-through; same as
    /// double-clicking the cell).
    #[must_use]
    pub fn show_source_rows() -> Self {
        Self::action(PIVOT_ACTION_SHOW_SOURCE_ROWS, "Show source rows in Grid")
    }

    /// The built-in "Copy value" item.
    #[must_use]
    pub fn copy_value() -> Self {
        Self::action(PIVOT_ACTION_COPY_VALUE, "Copy value")
    }

    /// The built-in "Copy pivot as CSV" item.
    #[must_use]
    pub fn copy_csv() -> Self {
        Self::action(PIVOT_ACTION_COPY_CSV, "Copy pivot as CSV")
    }

    /// The default menu shown when no provider is registered, for the given
    /// target. Providers can reuse this and append their own items.
    #[must_use]
    pub fn standard_items(target: &PivotMenuTarget) -> Vec<Self> {
        let mut items = Vec::new();
        match target {
            PivotMenuTarget::Cell(_) => {
                items.push(Self::show_source_rows());
                items.push(Self::copy_value());
                items.push(Self::separator());
            }
            PivotMenuTarget::RowHeader { .. } | PivotMenuTarget::ColHeader { .. } => {
                items.push(Self::show_source_rows());
                items.push(Self::separator());
            }
            PivotMenuTarget::Corner => {}
        }
        items.push(Self::copy_csv());
        items
    }
}

/// Trait implemented by consumers to supply custom right-click menu items
/// for the pivot and handle clicks on them.
///
/// Register on
/// [`crate::grid::SqllyDataTableBuilder::pivot_context_menu_provider`] (or
/// [`crate::grid::SqllyDataTable::set_pivot_context_menu_provider`]). When
/// registered, the provider fully controls the menu; compose the built-in
/// actions via [`PivotMenuItem::standard_items`] or the individual
/// constructors. Items carrying the built-in `pivot.*` ids are executed by
/// the pivot itself and do not reach [`PivotContextMenuProvider::on_action`].
pub trait PivotContextMenuProvider: 'static {
    /// Build the menu items for the given right-click context. Return an
    /// empty vec to suppress the menu for this target.
    fn menu_items(&self, request: &PivotContextMenuRequest) -> Vec<PivotMenuItem>;

    /// Handle a click on a custom action item. `action_id` matches the `id`
    /// supplied in [`PivotMenuItem::Action`].
    #[allow(unused_variables)]
    fn on_action(
        &self,
        action_id: &str,
        request: &PivotContextMenuRequest,
        state: &mut PivotState,
        cx: &mut gpui::App,
    ) {
    }
}

/// Type-erased handle wrapping an `Arc<dyn PivotContextMenuProvider>`.
#[derive(Clone)]
pub(crate) struct PivotContextMenuProviderHandle(Arc<dyn PivotContextMenuProvider>);

impl PivotContextMenuProviderHandle {
    pub(crate) fn new(provider: impl PivotContextMenuProvider + 'static) -> Self {
        Self(Arc::new(provider))
    }
}

impl fmt::Debug for PivotContextMenuProviderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PivotContextMenuProviderHandle")
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for PivotContextMenuProviderHandle {
    type Target = dyn PivotContextMenuProvider;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_items_for_cell_include_drill_copy_and_csv() {
        let cell = PivotCellContext {
            row_path: vec![],
            col_path: vec![],
            value: CellValue::Integer(1),
            formatted_value: "1".into(),
            is_row_grand_total: false,
            is_col_grand_total: false,
            is_row_subtotal: false,
            is_col_subtotal: false,
        };
        let items = PivotMenuItem::standard_items(&PivotMenuTarget::Cell(cell));
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], PivotMenuItem::show_source_rows());
        assert_eq!(items[1], PivotMenuItem::copy_value());
        assert_eq!(items[2], PivotMenuItem::Separator);
        assert_eq!(items[3], PivotMenuItem::copy_csv());
    }

    #[test]
    fn standard_items_for_corner_only_offer_csv() {
        let items = PivotMenuItem::standard_items(&PivotMenuTarget::Corner);
        assert_eq!(items, vec![PivotMenuItem::copy_csv()]);
    }

    #[test]
    fn standard_items_for_headers_offer_drill() {
        let items = PivotMenuItem::standard_items(&PivotMenuTarget::RowHeader {
            path: vec![],
            is_grand_total: false,
        });
        assert_eq!(items[0], PivotMenuItem::show_source_rows());
    }
}
