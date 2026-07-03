//! Public context-menu extensibility types.
//!
//! Consumers implement [`ContextMenuProvider`] and register it on
//! [`crate::grid::SqllyDataTableBuilder`] to fully control the right-click
//! menu. When a provider is registered the built-in column-header menu is
//! suppressed; consumers can compose built-in items via
//! [`ContextMenuItem::standard_column_header_items`].
//!
//! The provider receives an owned [`ContextMenuRequest`] snapshot captured
//! at menu-open time. The snapshot survives until the user clicks a menu
//! item, so the provider's [`ContextMenuProvider::on_action`] sees exactly
//! what was selected/right-clicked when the menu opened — even if grid state
//! (sort, filter, selection) changed in the interim.

use std::fmt;
use std::sync::Arc;

use crate::data::{CellValue, ColumnKind};
use crate::grid::menu::MenuAction;
use crate::grid::state::GridState;

/// What was right-clicked. Maps directly from the grid's hit-test result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuTarget {
    /// A data cell.
    Cell {
        display_row_index: usize,
        source_row_index: usize,
        column_index: usize,
    },
    /// The row-number gutter on the left edge.
    RowHeader {
        display_row_index: usize,
        source_row_index: usize,
    },
    /// A column header cell (excluding the sort button area).
    ColumnHeader { column_index: usize },
    /// The sort/indicator button inside a column header.
    SortButton { column_index: usize },
}

impl ContextMenuTarget {
    /// Returns the column index for targets that carry one, or `None` for
    /// row-header targets.
    #[must_use]
    pub fn column_index(&self) -> Option<usize> {
        match self {
            Self::Cell { column_index, .. } => Some(*column_index),
            Self::ColumnHeader { column_index } => Some(*column_index),
            Self::SortButton { column_index } => Some(*column_index),
            Self::RowHeader { .. } => None,
        }
    }

    /// Returns the display row index for targets that carry one, or `None`
    /// for column-header/sort-button targets.
    #[must_use]
    pub fn display_row_index(&self) -> Option<usize> {
        match self {
            Self::Cell {
                display_row_index, ..
            } => Some(*display_row_index),
            Self::RowHeader {
                display_row_index, ..
            } => Some(*display_row_index),
            Self::ColumnHeader { .. } | Self::SortButton { .. } => None,
        }
    }
}

/// Normalized inclusive selection range captured at menu-open time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuSelection {
    pub row_start: usize,
    pub row_end: usize,
    pub column_start: usize,
    pub column_end: usize,
}

/// Metadata and value for a single selected cell.
#[derive(Clone, Debug)]
pub struct SelectedCellContext {
    pub display_row_index: usize,
    pub source_row_index: usize,
    pub column_index: usize,
    pub column_name: String,
    pub value: CellValue,
}

/// Metadata for a column, included in [`SelectedRowContext`].
#[derive(Clone, Debug)]
pub struct ColumnContext {
    pub index: usize,
    pub name: String,
    pub kind: ColumnKind,
}

/// Full selected row: all cell values plus column metadata for name-based
/// lookup helpers.
#[derive(Clone, Debug)]
pub struct SelectedRowContext {
    pub display_row_index: usize,
    pub source_row_index: usize,
    pub values: Vec<CellValue>,
    pub columns: Vec<ColumnContext>,
}

impl SelectedRowContext {
    /// Value at the given ordinal column index.
    #[must_use]
    pub fn value_at(&self, column_index: usize) -> Option<&CellValue> {
        self.values.get(column_index)
    }

    /// Value for the first column whose name matches `column_name` exactly
    /// (case-sensitive). If duplicate names exist, the first match wins.
    #[must_use]
    pub fn value_by_name(&self, column_name: &str) -> Option<&CellValue> {
        self.column_index(column_name)
            .and_then(|i| self.values.get(i))
    }

    /// Iterator over `(column_name, value)` pairs for every column in this
    /// row.
    pub fn named_values(&self) -> impl Iterator<Item = (&str, &CellValue)> {
        self.columns
            .iter()
            .filter_map(move |col| self.values.get(col.index).map(|v| (col.name.as_str(), v)))
    }

    /// Ordinal column index for the first column whose name matches
    /// `column_name` exactly (case-sensitive). First duplicate wins.
    #[must_use]
    pub fn column_index(&self, column_name: &str) -> Option<usize> {
        self.columns
            .iter()
            .find(|c| c.name == column_name)
            .map(|c| c.index)
    }
}

/// Owned snapshot of the right-click context, captured at menu-open time.
///
/// All indices are display indices (post sort/filter) unless prefixed with
/// `source_`. The `selected_cells` and `selected_rows` vectors contain one
/// entry per cell/row in the effective selection; for large selections this
/// clones owned data.
///
/// For column-oriented targets (`ColumnHeader`, `SortButton`, or a
/// `Selection::Column`), `selected_rows` is left empty — a column right-click
/// is column-oriented (`clicked_row()` is `None`), so the column's values are
/// exposed through `selected_cells` and full per-row snapshots are skipped to
/// avoid O(rows x cols) cloning on large datasets.
#[derive(Clone, Debug)]
pub struct ContextMenuRequest {
    pub target: ContextMenuTarget,
    pub selection: Option<ContextMenuSelection>,
    pub selected_cells: Vec<SelectedCellContext>,
    pub selected_rows: Vec<SelectedRowContext>,
}

impl ContextMenuRequest {
    /// The specific cell under the cursor when the menu opened, if the
    /// right-click landed on a data cell.
    #[must_use]
    pub fn clicked_cell(&self) -> Option<&SelectedCellContext> {
        match &self.target {
            ContextMenuTarget::Cell {
                display_row_index,
                column_index,
                ..
            } => self.selected_cells.iter().find(|c| {
                c.display_row_index == *display_row_index && c.column_index == *column_index
            }),
            _ => None,
        }
    }

    /// The row under the cursor when the menu opened, if the right-click
    /// landed on a cell or row header.
    #[must_use]
    pub fn clicked_row(&self) -> Option<&SelectedRowContext> {
        let row = self.target.display_row_index()?;
        self.selected_rows
            .iter()
            .find(|r| r.display_row_index == row)
    }

    /// All selected cells in the effective selection.
    #[must_use]
    pub fn selected_cells(&self) -> &[SelectedCellContext] {
        &self.selected_cells
    }

    /// All selected rows in the effective selection.
    #[must_use]
    pub fn selected_rows(&self) -> &[SelectedRowContext] {
        &self.selected_rows
    }
}

/// Public menu item returned by a [`ContextMenuProvider`]. Distinct from the
/// internal `MenuItem` used by the rendering pipeline.
#[derive(Clone, Debug)]
pub enum ContextMenuItem {
    /// A built-in action (sort, copy, filter, etc.). Allows providers to
    /// compose standard column-header actions alongside custom ones.
    BuiltIn(MenuAction),
    /// A custom action with a consumer-defined `id` and display label.
    Action { id: String, label: String },
    /// A visual separator.
    Separator,
}

impl ContextMenuItem {
    /// Convenience constructor for a custom action.
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

    /// The standard built-in column-header menu items, in the same order
    /// the grid uses when no provider is registered. Providers can prepend
    /// or append custom items around this list.
    #[must_use]
    pub fn standard_column_header_items() -> Vec<Self> {
        vec![
            Self::BuiltIn(MenuAction::SelectColumn),
            Self::BuiltIn(MenuAction::CopyColumn),
            Self::BuiltIn(MenuAction::CopyColumnWithHeaders),
            Self::Separator,
            Self::BuiltIn(MenuAction::SortAscending),
            Self::BuiltIn(MenuAction::SortDescending),
            Self::BuiltIn(MenuAction::ClearSort),
            Self::Separator,
            Self::BuiltIn(MenuAction::FilterPrompt),
            Self::BuiltIn(MenuAction::ClearFilter),
        ]
    }
}

/// Trait implemented by consumers to supply custom right-click menu items
/// and handle clicks on those items.
///
/// The provider is registered on
/// [`crate::grid::SqllyDataTableBuilder::context_menu_provider`]. When
/// registered, the provider fully controls the right-click menu for all
/// targets (cells, row headers, column headers). When no provider is
/// registered, the built-in column-header menu is used unchanged.
///
/// `menu_items` is called only on right-click, so normal render/scroll
/// performance is unaffected. `on_action` is called when the user clicks a
/// custom menu item, with `&mut GridState` and `&mut gpui::App` available
/// for clipboard, selection, or application-level side effects.
pub trait ContextMenuProvider: 'static {
    /// Build the menu items for the given right-click context.
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem>;

    /// Handle a click on a custom action item. `action_id` matches the `id`
    /// supplied in [`ContextMenuItem::Action`]. Built-in items
    /// ([`ContextMenuItem::BuiltIn`]) are handled by the grid and do not
    /// reach this method.
    #[allow(unused_variables)]
    fn on_action(
        &self,
        action_id: &str,
        request: &ContextMenuRequest,
        state: &mut GridState,
        cx: &mut gpui::App,
    ) {
    }
}

/// Type-erased handle wrapping an `Arc<dyn ContextMenuProvider>`. Stored on
/// `GridState` and cloned into event closures.
#[derive(Clone)]
pub(crate) struct ContextMenuProviderHandle(Arc<dyn ContextMenuProvider>);

impl ContextMenuProviderHandle {
    pub(crate) fn new(provider: impl ContextMenuProvider + 'static) -> Self {
        Self(Arc::new(provider))
    }
}

impl fmt::Debug for ContextMenuProviderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenuProviderHandle")
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for ContextMenuProviderHandle {
    type Target = dyn ContextMenuProvider;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// Deferred custom context-menu action, flushed at the top of `render`.
#[derive(Clone, Debug)]
pub(crate) struct PendingCustomContextMenuAction {
    pub id: String,
    pub request: ContextMenuRequest,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn row(name: &str, values: &[CellValue]) -> SelectedRowContext {
        let columns = vec![
            ColumnContext {
                index: 0,
                name: "id".into(),
                kind: ColumnKind::Integer,
            },
            ColumnContext {
                index: 1,
                name: name.into(),
                kind: ColumnKind::Text,
            },
        ];
        SelectedRowContext {
            display_row_index: 0,
            source_row_index: 0,
            values: values.to_vec(),
            columns,
        }
    }

    #[test]
    fn value_at_returns_by_ordinal() {
        let r = row(
            "name",
            &[CellValue::Integer(7), CellValue::Text("hi".into())],
        );
        assert_eq!(r.value_at(0), Some(&CellValue::Integer(7)));
        assert_eq!(r.value_at(1), Some(&CellValue::Text("hi".into())));
        assert_eq!(r.value_at(2), None);
    }

    #[test]
    fn value_by_name_exact_case_sensitive() {
        let r = row(
            "Name",
            &[CellValue::Integer(7), CellValue::Text("hi".into())],
        );
        assert_eq!(r.value_by_name("Name"), Some(&CellValue::Text("hi".into())));
        assert_eq!(r.value_by_name("name"), None);
        assert_eq!(r.value_by_name("NAME"), None);
    }

    #[test]
    fn value_by_name_first_duplicate_wins() {
        let columns = vec![
            ColumnContext {
                index: 0,
                name: "dup".into(),
                kind: ColumnKind::Integer,
            },
            ColumnContext {
                index: 1,
                name: "dup".into(),
                kind: ColumnKind::Integer,
            },
        ];
        let r = SelectedRowContext {
            display_row_index: 0,
            source_row_index: 0,
            values: vec![CellValue::Integer(1), CellValue::Integer(2)],
            columns,
        };
        assert_eq!(r.value_by_name("dup"), Some(&CellValue::Integer(1)));
        assert_eq!(r.column_index("dup"), Some(0));
    }

    #[test]
    fn named_values_iterates_all_columns() {
        let r = row(
            "name",
            &[CellValue::Integer(7), CellValue::Text("hi".into())],
        );
        let pairs: Vec<_> = r.named_values().collect();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "id");
        assert_eq!(pairs[0].1, &CellValue::Integer(7));
        assert_eq!(pairs[1].0, "name");
        assert_eq!(pairs[1].1, &CellValue::Text("hi".into()));
    }

    #[test]
    fn context_menu_target_column_index() {
        assert_eq!(
            ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 0,
                column_index: 3
            }
            .column_index(),
            Some(3)
        );
        assert_eq!(
            ContextMenuTarget::RowHeader {
                display_row_index: 0,
                source_row_index: 0
            }
            .column_index(),
            None
        );
    }

    #[test]
    fn context_menu_target_display_row_index() {
        assert_eq!(
            ContextMenuTarget::Cell {
                display_row_index: 5,
                source_row_index: 2,
                column_index: 0
            }
            .display_row_index(),
            Some(5)
        );
        assert_eq!(
            ContextMenuTarget::ColumnHeader { column_index: 1 }.display_row_index(),
            None
        );
    }

    #[test]
    fn standard_column_header_items_match_builtin_order() {
        let items = ContextMenuItem::standard_column_header_items();
        assert_eq!(items.len(), 10);
        assert!(matches!(
            items[0],
            ContextMenuItem::BuiltIn(MenuAction::SelectColumn)
        ));
        assert!(matches!(items[3], ContextMenuItem::Separator));
        assert!(matches!(
            items[9],
            ContextMenuItem::BuiltIn(MenuAction::ClearFilter)
        ));
    }

    #[test]
    fn clicked_cell_finds_target_cell() {
        let request = ContextMenuRequest {
            target: ContextMenuTarget::Cell {
                display_row_index: 1,
                source_row_index: 2,
                column_index: 0,
            },
            selection: None,
            selected_cells: vec![
                SelectedCellContext {
                    display_row_index: 0,
                    source_row_index: 0,
                    column_index: 0,
                    column_name: "a".into(),
                    value: CellValue::Integer(1),
                },
                SelectedCellContext {
                    display_row_index: 1,
                    source_row_index: 2,
                    column_index: 0,
                    column_name: "a".into(),
                    value: CellValue::Integer(3),
                },
            ],
            selected_rows: vec![],
        };
        let clicked = request.clicked_cell().unwrap();
        assert_eq!(clicked.source_row_index, 2);
        assert_eq!(clicked.value, CellValue::Integer(3));
    }

    #[test]
    fn clicked_cell_none_for_column_header_target() {
        let request = ContextMenuRequest {
            target: ContextMenuTarget::ColumnHeader { column_index: 0 },
            selection: None,
            selected_cells: vec![],
            selected_rows: vec![],
        };
        assert!(request.clicked_cell().is_none());
    }

    #[test]
    fn clicked_row_finds_target_for_row_header() {
        let request = ContextMenuRequest {
            target: ContextMenuTarget::RowHeader {
                display_row_index: 1,
                source_row_index: 2,
            },
            selection: None,
            selected_cells: vec![],
            selected_rows: vec![
                SelectedRowContext {
                    display_row_index: 0,
                    source_row_index: 0,
                    values: vec![],
                    columns: vec![],
                },
                SelectedRowContext {
                    display_row_index: 1,
                    source_row_index: 2,
                    values: vec![],
                    columns: vec![],
                },
            ],
        };
        assert_eq!(request.clicked_row().unwrap().source_row_index, 2);
    }

    #[test]
    fn clicked_row_none_for_column_header() {
        let request = ContextMenuRequest {
            target: ContextMenuTarget::ColumnHeader { column_index: 0 },
            selection: None,
            selected_cells: vec![],
            selected_rows: vec![],
        };
        assert!(request.clicked_row().is_none());
    }
}
