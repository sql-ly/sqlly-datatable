//! `PivotState` — runtime state for the pivot view: the computed result,
//! expand/collapse, sorting, source filters, selection, scrolling, and
//! hit-testing.
//!
//! The state holds an `Arc` snapshot of the source rows shared with the flat
//! grid. **The source is never mutated**; every interaction either re-runs
//! the pure engine (config/filter changes) or just re-flattens the visible
//! row/column lists (expand/collapse/sort — no engine recompute).

use crate::config::{KeyBindings, NumberFormat, ResolvedColumnFormat, TextAlignment};
use crate::data::{compare_cells, CellValue, Column, ColumnKind};
use crate::format::format_cell;
use crate::grid::selection::{ScrollbarAxis, SortDirection};
use crate::grid::state::{FilterValueRow, SCROLLBAR_SIZE};
use crate::grid::theme::GridTheme;
use crate::pivot::aggregation::AggregationFn;
use crate::pivot::config::{PivotConfig, PivotZone};
use crate::pivot::context_menu::{
    PivotCellContext, PivotContextMenuProviderHandle, PivotContextMenuRequest, PivotMenuItem,
    PivotMenuTarget, PivotPathComponent, PIVOT_ACTION_COPY_CSV, PIVOT_ACTION_COPY_VALUE,
    PIVOT_ACTION_SHOW_SOURCE_ROWS,
};
use crate::pivot::engine::{compute_pivot, PivotNode, PivotResult, TOTAL_KEY};

use gpui::{px, App, Bounds, FocusHandle, Pixels, Point, ScrollHandle, Size};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// Callback invoked when the user clicks the sidebar's save-configuration
/// button. Receives the live [`PivotConfig`] to persist.
pub type PivotSaveConfigHandler = Rc<dyn Fn(&PivotConfig, &mut App)>;

/// What kind of line a visible pivot row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleRowKind {
    /// An innermost group — a plain data row.
    Leaf,
    /// A group header. `expanded == false` means the group is collapsed and
    /// this row shows the group's subtotals.
    GroupHeader {
        /// Whether the group's children are currently shown.
        expanded: bool,
    },
    /// The grand-total row at the bottom.
    GrandTotal,
}

/// One row of the flattened, display-ready pivot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleRow {
    /// Row node id, or [`TOTAL_KEY`] for the grand-total row (and for the
    /// single value row when no row fields are assigned).
    pub key: usize,
    /// Indent level (0 = outermost).
    pub depth: usize,
    /// What this line is.
    pub kind: VisibleRowKind,
    /// Alternating-shade flag for leaf rows; resets at every group header
    /// so striping restarts within each group.
    pub zebra: bool,
}

/// What kind of column a visible pivot column is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleColKind {
    /// An innermost column group — a plain value column.
    Leaf,
    /// A collapsed column group shown as a single subtotal column.
    Collapsed,
    /// The "Total" column appended after an expanded group
    /// (when [`PivotConfig::show_column_subtotals`] is on).
    Subtotal,
    /// The grand-total column at the right.
    GrandTotal,
}

/// One column of the flattened, display-ready pivot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleCol {
    /// Column node id, or [`TOTAL_KEY`] for the grand-total column (and for
    /// the single value column when no column fields are assigned).
    pub key: usize,
    /// Depth of the node this column represents.
    pub depth: usize,
    /// What this column is.
    pub kind: VisibleColKind,
}

/// What the row axis (and, for `ColLabel`, the column axis) is ordered by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PivotSortKey {
    /// Order row groups by their grouping value.
    RowLabel,
    /// Order row groups by their aggregated value in the given visible
    /// column key ([`TOTAL_KEY`] = the grand-total column, i.e. "by
    /// subtotal").
    RowsByColumn(usize),
    /// Order column groups by their grouping value.
    ColLabel,
}

/// What a pointer position over the pivot grid resolved to. Row/column
/// coordinates are indices into the visible row/column lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PivotHitResult {
    /// Nothing interactive.
    None,
    /// The top-left corner block.
    Corner,
    /// A column header cell (any header level) over visible column `col`.
    ColHeader {
        /// Header level (0 = caption row).
        level: usize,
        /// Visible column index.
        col: usize,
    },
    /// A row label cell.
    RowHeader {
        /// Visible row index.
        row: usize,
    },
    /// The lower edge of a visible row in the row-header area.
    RowBorder {
        /// Visible row index whose lower edge was hit.
        row: usize,
    },
    /// The right edge of a visible value column in the header area.
    ColBorder {
        /// Visible column index whose right edge was hit.
        col: usize,
    },
    /// The expand/collapse chevron on a row group header.
    RowChevron {
        /// Visible row index.
        row: usize,
    },
    /// The expand/collapse chevron on a column group header.
    ColChevron {
        /// Visible column index the chevron was painted at (run start).
        col: usize,
        /// Depth of the group the chevron belongs to.
        depth: usize,
    },
    /// A data cell.
    Cell {
        /// Visible row index.
        row: usize,
        /// Visible column index.
        col: usize,
    },
    /// The vertical scrollbar strip.
    VScrollbar,
    /// The horizontal scrollbar strip.
    HScrollbar,
}

/// Working state of the checklist popover opened from a Filters-zone chip.
#[derive(Clone, Debug)]
pub struct PivotFilterPopover {
    /// The source column being filtered.
    pub field: usize,
    /// Distinct formatted values with their checked state.
    pub rows: Vec<FilterValueRow>,
    /// Window-absolute anchor where the popover opens (the click position).
    pub anchor: Point<Pixels>,
}

impl PivotFilterPopover {
    /// `true` when every value is checked (filter is inert).
    #[must_use]
    pub fn all_checked(&self) -> bool {
        !self.rows.is_empty() && self.rows.iter().all(|r| r.checked)
    }
}

/// Working state of the per-field format dialog opened by double-clicking a
/// sidebar zone chip.
#[derive(Clone, Copy, Debug)]
pub struct PivotFormatDialog {
    /// The source column being configured.
    pub field: usize,
    /// The zone whose chip was double-clicked. [`PivotZone::Values`] edits
    /// [`PivotConfig::value_format`] (value cells); every other zone edits
    /// [`PivotConfig::field_formats`] (that field's labels).
    pub zone: PivotZone,
    /// Window-absolute anchor where the dialog opens (the click position).
    pub anchor: Point<Pixels>,
}

/// An open pivot right-click menu: what to show, where, and the context it
/// was opened for.
#[derive(Clone, Debug)]
pub(crate) struct PivotMenu {
    /// Grid-relative anchor (the right-click position).
    pub(crate) anchor: Point<Pixels>,
    /// Items in display order.
    pub(crate) items: Vec<PivotMenuItem>,
    /// Index (into action items only) currently hovered.
    pub(crate) hovered: Option<usize>,
    /// The context snapshot handed to the provider.
    pub(crate) request: PivotContextMenuRequest,
    /// Precomputed drill-through filters for the built-in "Show source
    /// rows" action.
    pub(crate) drill: Vec<(usize, HashSet<String>)>,
}

/// Fixed indent per row-group depth level, in pixels.
pub(crate) const ROW_INDENT: f32 = 16.0;
/// Square hit/paint size of an expand/collapse chevron.
pub(crate) const CHEVRON_SIZE: f32 = 14.0;
const RESIZE_HIT_SLOP: f32 = 3.0;
/// Default height of pivot data rows, in logical pixels.
pub const DEFAULT_PIVOT_ROW_HEIGHT: f32 = 24.0;
/// Default width of pivot value columns, in logical pixels.
pub const DEFAULT_PIVOT_COLUMN_WIDTH: f32 = 140.0;
/// Smallest supported pivot data-row height.
pub const MIN_PIVOT_ROW_HEIGHT: f32 = 18.0;
/// Smallest supported pivot value-column width.
pub const MIN_PIVOT_COLUMN_WIDTH: f32 = 40.0;
/// Default width of the pivot controls sidebar, in logical pixels.
pub const DEFAULT_PIVOT_SIDEBAR_WIDTH: f32 = 260.0;

#[derive(Clone, Copy, Debug)]
enum PivotResizeDrag {
    Row {
        boundary: usize,
        start_y: f32,
        start_height: f32,
    },
    Column {
        boundary: usize,
        start_x: f32,
        start_width: f32,
    },
}

/// Complete pivot-view state owned by a GPUI `Entity<PivotState>`.
pub struct PivotState {
    /// Live pivot layout. Mutate it (or use the helper methods) and call
    /// [`PivotState::recompute`]; read it back for persistence — this struct
    /// *is* the "current configuration" API.
    pub config: PivotConfig,
    /// The computed pivot, Arc-wrapped so paint snapshots clone in O(1).
    pub result: Arc<PivotResult>,
    /// Shared, immutable source rows (same Arc as the flat grid's snapshot).
    pub(crate) source_rows: Arc<Vec<Vec<CellValue>>>,
    /// Source column metadata.
    pub(crate) source_columns: Vec<Column>,
    /// Resolved per-source-column formats; drive group labels and filters.
    pub(crate) resolved_formats: Vec<ResolvedColumnFormat>,
    /// [`Self::resolved_formats`] with [`PivotConfig::field_formats`]
    /// overrides applied. Rebuilt on every recompute; everything that
    /// formats group labels or filter values reads these.
    pub(crate) label_formats: Vec<ResolvedColumnFormat>,
    /// Format used for value cells (kind-adjusted for the aggregation).
    pub(crate) value_fmt: ResolvedColumnFormat,

    /// Flattened display rows, rebuilt on recompute/collapse/sort.
    /// Arc-wrapped so paint snapshots clone in O(1).
    pub(crate) visible_rows: Arc<Vec<VisibleRow>>,
    /// Flattened display columns.
    pub(crate) visible_cols: Arc<Vec<VisibleCol>>,
    /// Collapsed row groups, keyed by label path so collapse state survives
    /// recomputes (node ids do not).
    pub(crate) collapsed_row_paths: HashSet<Vec<String>>,
    /// Collapsed column groups, keyed by label path.
    pub(crate) collapsed_col_paths: HashSet<Vec<String>>,
    /// Active ordering. `None` = canonical (ascending by grouping value).
    pub sort: Option<(PivotSortKey, SortDirection)>,
    /// Per filter-field allow-list of formatted labels. Absent entry = all
    /// values pass.
    pub(crate) filter_values: HashMap<usize, HashSet<String>>,
    /// Open Filters-zone checklist popover, if any.
    pub(crate) filter_popover: Option<PivotFilterPopover>,
    /// Open per-field format dialog (sidebar chip double-click), if any.
    pub(crate) format_dialog: Option<PivotFormatDialog>,
    /// Whether the sidebar's aggregation picker is expanded.
    pub agg_menu_open: bool,
    /// Registered save-configuration action. The sidebar's save button only
    /// renders while this is `Some`.
    pub(crate) save_config_handler: Option<PivotSaveConfigHandler>,
    /// Current width of the pivot controls sidebar, kept in sync by the host
    /// widget. The sidebar uses it to decide when chip labels are truncated.
    pub(crate) sidebar_width: f32,
    /// Registered right-click provider; `None` shows the built-in menu.
    pub(crate) context_menu_provider: Option<PivotContextMenuProviderHandle>,
    /// Open right-click menu, if any.
    pub(crate) menu: Option<PivotMenu>,
    /// Drill-through request produced by a double-click or the built-in
    /// "Show source rows" action: per-source-column allowed formatted
    /// values. Drained by `SqllyDataTable::render`, which applies them as
    /// grid filters and switches to the Grid tab.
    pub(crate) pending_drill_down: Option<Vec<(usize, HashSet<String>)>>,

    /// Selected rectangle in visible coordinates `(r1, c1, r2, c2)`,
    /// normalized.
    pub selection: Option<(usize, usize, usize, usize)>,
    pub(crate) select_anchor: Option<(usize, usize)>,
    pub(crate) is_selecting: bool,
    /// Last hit under the pointer (drives hover affordances).
    pub hover_hit: Option<PivotHitResult>,
    pub(crate) scrollbar_drag: Option<ScrollbarAxis>,
    resize_drag: Option<PivotResizeDrag>,

    /// Theme shared with the host grid.
    pub theme: GridTheme,
    /// Mirrors [`crate::GridConfig::animations`]: whether the pivot's transient
    /// surfaces (menu, filter popover, format dialog, drag ghost, accordion
    /// bodies) fade in on appear. Set from the host grid's config at build time.
    pub animations: bool,
    pub(crate) key_bindings: KeyBindings,
    /// Scroll offset of the data region.
    pub scroll_handle: ScrollHandle,
    /// Focus handle for keyboard input.
    pub focus_handle: FocusHandle,
    /// Painted bounds, updated each layout pass.
    pub bounds: Bounds<Pixels>,
    pub(crate) window_viewport: Size<Pixels>,
    /// Height of one data row.
    pub row_height: f32,
    /// Height of one column-header level.
    pub header_row_height: f32,
    /// Width of the row-label column.
    pub row_header_width: f32,
    /// Uniform width of every value column.
    pub value_col_width: f32,
    /// Font size for all pivot text.
    pub font_size: f32,
    /// Approximate monospace character width used for text measurement.
    pub char_width: f32,
}

impl PivotState {
    /// Build a pivot state over a shared source snapshot and compute the
    /// initial result.
    #[must_use]
    pub fn new(
        source_columns: Vec<Column>,
        source_rows: Arc<Vec<Vec<CellValue>>>,
        resolved_formats: Vec<ResolvedColumnFormat>,
        config: PivotConfig,
        key_bindings: KeyBindings,
        focus_handle: FocusHandle,
    ) -> Self {
        let mut state = Self {
            config,
            result: Arc::new(PivotResult::default()),
            source_rows,
            source_columns,
            resolved_formats,
            label_formats: Vec::new(),
            value_fmt: default_value_format(),
            visible_rows: Arc::new(Vec::new()),
            visible_cols: Arc::new(Vec::new()),
            collapsed_row_paths: HashSet::new(),
            collapsed_col_paths: HashSet::new(),
            sort: None,
            filter_values: HashMap::new(),
            filter_popover: None,
            format_dialog: None,
            agg_menu_open: false,
            save_config_handler: None,
            sidebar_width: DEFAULT_PIVOT_SIDEBAR_WIDTH,
            context_menu_provider: None,
            menu: None,
            pending_drill_down: None,
            selection: None,
            select_anchor: None,
            is_selecting: false,
            hover_hit: None,
            scrollbar_drag: None,
            theme: GridTheme::default(),
            animations: true,
            key_bindings,
            scroll_handle: ScrollHandle::new(),
            focus_handle,
            bounds: Bounds::default(),
            window_viewport: Size::default(),
            row_height: DEFAULT_PIVOT_ROW_HEIGHT,
            header_row_height: 26.0,
            row_header_width: 220.0,
            value_col_width: DEFAULT_PIVOT_COLUMN_WIDTH,
            // Match the flat grid's cell font (grid/state.rs) so cell text
            // doesn't change size when the user toggles Grid ↔ Pivot.
            font_size: 14.0,
            char_width: crate::grid::paint::default_char_width(14.0),
            resize_drag: None,
        };
        state.recompute();
        state
    }

    /// Replace the source snapshot (e.g. after the flat grid appended rows)
    /// and recompute. O(1) extra memory when `rows` shares the grid's Arc.
    pub fn set_source(&mut self, columns: Vec<Column>, rows: Arc<Vec<Vec<CellValue>>>) {
        self.source_columns = columns;
        self.source_rows = rows;
        self.recompute();
    }

    /// `true` when `rows` is a different snapshot than the current source.
    #[must_use]
    pub fn source_differs(&self, rows: &Arc<Vec<Vec<CellValue>>>) -> bool {
        !Arc::ptr_eq(&self.source_rows, rows)
    }

    /// Source column metadata (for building field lists).
    #[must_use]
    pub fn source_columns(&self) -> &[Column] {
        &self.source_columns
    }

    /// Re-run the pivot engine against the current config/filters, then
    /// rebuild the flattened display lists. Never mutates the source rows.
    pub fn recompute(&mut self) {
        self.config.clamp_to_columns(self.source_columns.len());
        let filter_fields = self.config.filter_fields.clone();
        self.filter_values
            .retain(|field, _| filter_fields.contains(field));
        self.label_formats = self.build_label_formats();

        if self.config.is_ready() {
            let included = self.filtered_source_rows();
            self.result = Arc::new(compute_pivot(
                &self.source_columns,
                &self.source_rows,
                &included,
                &self.config,
                &self.label_formats,
            ));
        } else {
            self.result = Arc::new(PivotResult::default());
        }
        self.value_fmt = self.build_value_format();
        self.resort();
    }

    /// [`Self::resolved_formats`] with [`PivotConfig::field_formats`]
    /// applied: the override replaces the number format and its alignment
    /// carries over to every kind's alignment.
    fn build_label_formats(&self) -> Vec<ResolvedColumnFormat> {
        self.resolved_formats
            .iter()
            .enumerate()
            .map(|(i, base)| {
                let mut fmt = base.clone();
                if let Some(over) = self.config.field_formats.get(&i) {
                    fmt.number = *over;
                    fmt.string.alignment = over.alignment;
                    fmt.date.alignment = over.alignment;
                    fmt.boolean.alignment = over.alignment;
                }
                fmt
            })
            .collect()
    }

    /// The effective display format for one field's group labels and filter
    /// values (resolved column format plus any
    /// [`PivotConfig::field_formats`] override).
    #[must_use]
    pub fn label_format(&self, field: usize) -> Option<&ResolvedColumnFormat> {
        self.label_formats.get(field)
    }

    /// Source row indices that pass every active Filters-zone filter.
    fn filtered_source_rows(&self) -> Vec<usize> {
        let active: Vec<(usize, &HashSet<String>)> = self
            .config
            .filter_fields
            .iter()
            .filter_map(|&f| self.filter_values.get(&f).map(|set| (f, set)))
            .collect();
        if active.is_empty() {
            return (0..self.source_rows.len()).collect();
        }
        (0..self.source_rows.len())
            .filter(|&r| {
                active.iter().all(|(field, allowed)| {
                    let cell = self.source_rows[r].get(*field).unwrap_or(&CellValue::None);
                    let label = format_cell(cell, &self.label_formats[*field]).0;
                    allowed.contains(&label)
                })
            })
            .collect()
    }

    /// Resolved display format for value cells: the value column's format
    /// with its kind adjusted to what the aggregation actually produces
    /// (`Count` → Integer, `Avg` → Decimal), plus the optional
    /// [`PivotConfig::value_format`] number override. Value cells are always
    /// right-aligned and paint negative numbers red regardless of the source
    /// column's format (an explicit `value_format` override may still choose
    /// otherwise).
    fn build_value_format(&self) -> ResolvedColumnFormat {
        let mut fmt = self
            .config
            .value_field
            .and_then(|f| self.resolved_formats.get(f).cloned())
            .unwrap_or_else(default_value_format);
        match self.config.aggregation {
            AggregationFn::Count => {
                fmt.kind = ColumnKind::Integer;
                fmt.number = NumberFormat {
                    decimals: 0,
                    ..fmt.number
                };
            }
            AggregationFn::Avg => fmt.kind = ColumnKind::Decimal,
            AggregationFn::Sum | AggregationFn::Min | AggregationFn::Max => {}
        }
        fmt.number.alignment = TextAlignment::Right;
        fmt.string.alignment = TextAlignment::Right;
        fmt.date.alignment = TextAlignment::Right;
        fmt.boolean.alignment = TextAlignment::Right;
        fmt.number.show_negative_red = true;
        if let Some(over) = self.config.value_format {
            fmt.number = over;
            fmt.string.alignment = over.alignment;
            fmt.date.alignment = over.alignment;
            fmt.boolean.alignment = over.alignment;
        }
        fmt
    }

    /// The format used for value cells (public for export code).
    #[must_use]
    pub fn value_format(&self) -> &ResolvedColumnFormat {
        &self.value_fmt
    }

    // ------------------------------------------------------------------
    // Flattening / expand-collapse
    // ------------------------------------------------------------------

    /// Rebuild [`Self::visible_rows`] / [`Self::visible_cols`] from the
    /// result and the collapse sets. Cheap: no engine work.
    pub(crate) fn rebuild_visible(&mut self) {
        self.visible_rows = Arc::new(flatten_rows(
            &self.result,
            &self.collapsed_row_paths,
            &self.config,
        ));
        self.visible_cols = Arc::new(flatten_cols(
            &self.result,
            &self.collapsed_col_paths,
            &self.config,
        ));
        self.clamp_selection();
        self.clamp_scroll_to_bounds();
    }

    /// The flattened display rows.
    #[must_use]
    pub fn visible_rows(&self) -> &[VisibleRow] {
        &self.visible_rows
    }

    /// The flattened display columns.
    #[must_use]
    pub fn visible_cols(&self) -> &[VisibleCol] {
        &self.visible_cols
    }

    /// Toggle a row group open/closed. `node` is a row node id. Instant —
    /// only re-flattens, no engine recompute.
    pub fn toggle_row_group(&mut self, node: usize) {
        if node >= self.result.row_nodes.len() {
            return;
        }
        let path = node_path(&self.result.row_nodes, node);
        if !self.collapsed_row_paths.remove(&path) {
            self.collapsed_row_paths.insert(path);
        }
        self.rebuild_visible();
    }

    /// Toggle a column group open/closed. `node` is a column node id.
    pub fn toggle_col_group(&mut self, node: usize) {
        if node >= self.result.col_nodes.len() {
            return;
        }
        let path = node_path(&self.result.col_nodes, node);
        if !self.collapsed_col_paths.remove(&path) {
            self.collapsed_col_paths.insert(path);
        }
        self.rebuild_visible();
    }

    /// Collapse every row group at every level.
    pub fn collapse_all_rows(&mut self) {
        self.collapsed_row_paths = all_group_paths(&self.result.row_nodes);
        self.rebuild_visible();
    }

    /// Expand every row group.
    pub fn expand_all_rows(&mut self) {
        self.collapsed_row_paths.clear();
        self.rebuild_visible();
    }

    /// Collapse every column group at every level.
    pub fn collapse_all_cols(&mut self) {
        self.collapsed_col_paths = all_group_paths(&self.result.col_nodes);
        self.rebuild_visible();
    }

    /// Expand every column group.
    pub fn expand_all_cols(&mut self) {
        self.collapsed_col_paths.clear();
        self.rebuild_visible();
    }

    // ------------------------------------------------------------------
    // Sorting
    // ------------------------------------------------------------------

    /// Cycle row ordering by a visible column key (asc → desc → canonical).
    /// Pass [`TOTAL_KEY`] to sort by the row subtotals.
    pub fn cycle_sort_by_column(&mut self, col_key: usize) {
        self.sort = match self.sort {
            Some((PivotSortKey::RowsByColumn(k), SortDirection::Ascending)) if k == col_key => {
                Some((
                    PivotSortKey::RowsByColumn(col_key),
                    SortDirection::Descending,
                ))
            }
            Some((PivotSortKey::RowsByColumn(k), SortDirection::Descending)) if k == col_key => {
                None
            }
            _ => Some((
                PivotSortKey::RowsByColumn(col_key),
                SortDirection::Ascending,
            )),
        };
        self.resort();
    }

    /// Cycle row ordering by the row labels. The canonical order is itself
    /// ascending, so this cycles asc → desc → canonical.
    pub fn cycle_label_sort(&mut self) {
        self.sort = match self.sort {
            Some((PivotSortKey::RowLabel, SortDirection::Ascending)) => {
                Some((PivotSortKey::RowLabel, SortDirection::Descending))
            }
            Some((PivotSortKey::RowLabel, SortDirection::Descending)) => None,
            _ => Some((PivotSortKey::RowLabel, SortDirection::Ascending)),
        };
        self.resort();
    }

    /// Cycle column ordering by the column labels.
    pub fn cycle_col_label_sort(&mut self) {
        self.sort = match self.sort {
            Some((PivotSortKey::ColLabel, SortDirection::Ascending)) => {
                Some((PivotSortKey::ColLabel, SortDirection::Descending))
            }
            Some((PivotSortKey::ColLabel, SortDirection::Descending)) => None,
            _ => Some((PivotSortKey::ColLabel, SortDirection::Ascending)),
        };
        self.resort();
    }

    /// Re-apply the active sort to the result trees, then re-flatten.
    /// Collapse only hides children — sorting is computed over all groups,
    /// so re-expanding shows children in sorted order.
    pub(crate) fn resort(&mut self) {
        let result = Arc::make_mut(&mut self.result);
        // Always restore the canonical (ascending grouping value) order
        // first so the active sort applies over a deterministic base.
        sort_axis_by_key(&mut result.row_nodes, &mut result.row_roots, false);
        sort_axis_by_key(&mut result.col_nodes, &mut result.col_roots, false);
        match self.sort {
            None => {}
            Some((PivotSortKey::RowLabel, dir)) => {
                sort_axis_by_key(
                    &mut result.row_nodes,
                    &mut result.row_roots,
                    dir == SortDirection::Descending,
                );
            }
            Some((PivotSortKey::ColLabel, dir)) => {
                sort_axis_by_key(
                    &mut result.col_nodes,
                    &mut result.col_roots,
                    dir == SortDirection::Descending,
                );
            }
            Some((PivotSortKey::RowsByColumn(col_key), dir)) => {
                let keys: Vec<CellValue> = (0..result.row_nodes.len())
                    .map(|id| {
                        result
                            .values
                            .get(&(id, col_key))
                            .cloned()
                            .unwrap_or(CellValue::None)
                    })
                    .collect();
                sort_axis_by(
                    &mut result.row_nodes,
                    &mut result.row_roots,
                    &keys,
                    dir == SortDirection::Descending,
                );
            }
        }
        self.rebuild_visible();
    }

    // ------------------------------------------------------------------
    // Filters-zone popover
    // ------------------------------------------------------------------

    /// Open (or re-open) the value checklist for a Filters-zone field.
    /// `anchor` is the window-absolute position the popover opens at.
    pub fn open_filter_popover(&mut self, field: usize, anchor: Point<Pixels>) {
        if field >= self.source_columns.len() {
            return;
        }
        let fmt = &self.label_formats[field];
        let allowed = self.filter_values.get(&field);
        let mut seen: HashSet<String> = HashSet::new();
        let mut pairs: Vec<(String, CellValue)> = Vec::new();
        for row in self.source_rows.iter() {
            let cell = row.get(field).unwrap_or(&CellValue::None);
            let label = format_cell(cell, fmt).0;
            if seen.insert(label.clone()) {
                pairs.push((label, cell.clone()));
            }
        }
        pairs.sort_by(|(_, a), (_, b)| compare_cells(a, b));
        let rows = pairs
            .into_iter()
            .map(|(label, _)| {
                let checked = allowed.is_none_or(|set| set.contains(&label));
                FilterValueRow { label, checked }
            })
            .collect();
        self.filter_popover = Some(PivotFilterPopover {
            field,
            rows,
            anchor,
        });
    }

    /// The open Filters-zone popover, if any.
    #[must_use]
    pub fn filter_popover(&self) -> Option<&PivotFilterPopover> {
        self.filter_popover.as_ref()
    }

    /// Toggle one checklist row and re-apply immediately.
    pub fn toggle_filter_popover_value(&mut self, index: usize) {
        if let Some(p) = &mut self.filter_popover {
            if let Some(row) = p.rows.get_mut(index) {
                row.checked = !row.checked;
            }
        }
        self.apply_filter_popover();
    }

    /// Toggle all checklist rows at once and re-apply.
    pub fn toggle_filter_popover_select_all(&mut self) {
        if let Some(p) = &mut self.filter_popover {
            let target = !p.all_checked();
            for row in &mut p.rows {
                row.checked = target;
            }
        }
        self.apply_filter_popover();
    }

    /// Commit the popover's checked set to the active filters and recompute.
    /// All-checked stores no entry (inert filter).
    pub fn apply_filter_popover(&mut self) {
        let Some(p) = &self.filter_popover else {
            return;
        };
        let field = p.field;
        if p.all_checked() {
            self.filter_values.remove(&field);
        } else {
            self.filter_values.insert(
                field,
                p.rows
                    .iter()
                    .filter(|r| r.checked)
                    .map(|r| r.label.clone())
                    .collect(),
            );
        }
        self.recompute();
    }

    /// Close the popover (its edits are already applied — auto-apply).
    pub fn close_filter_popover(&mut self) {
        self.filter_popover = None;
    }

    /// Clear the filter on one Filters-zone field.
    pub fn clear_filter(&mut self, field: usize) {
        if self.filter_values.remove(&field).is_some() {
            self.recompute();
        }
        if let Some(p) = &mut self.filter_popover {
            if p.field == field {
                for row in &mut p.rows {
                    row.checked = true;
                }
            }
        }
    }

    /// `true` when the given Filters-zone field has an active (non-inert)
    /// filter.
    #[must_use]
    pub fn filter_active(&self, field: usize) -> bool {
        self.filter_values.contains_key(&field)
    }

    // ------------------------------------------------------------------
    // Per-field format dialog
    // ------------------------------------------------------------------

    /// The open format dialog, if any.
    #[must_use]
    pub fn format_dialog(&self) -> Option<&PivotFormatDialog> {
        self.format_dialog.as_ref()
    }

    /// Open the format dialog for `field` as it appears in `zone`. `anchor`
    /// is the window-absolute position the dialog opens at.
    pub fn open_format_dialog(&mut self, field: usize, zone: PivotZone, anchor: Point<Pixels>) {
        if field >= self.source_columns.len() {
            return;
        }
        self.format_dialog = Some(PivotFormatDialog {
            field,
            zone,
            anchor,
        });
    }

    /// Close the format dialog (its edits are already applied — auto-apply).
    pub fn close_format_dialog(&mut self) {
        self.format_dialog = None;
    }

    /// The number format the open dialog is editing: the effective value-cell
    /// format for a Values chip, or the field's effective label format
    /// otherwise.
    #[must_use]
    pub fn format_dialog_format(&self) -> Option<NumberFormat> {
        let dialog = self.format_dialog.as_ref()?;
        let fmt = match dialog.zone {
            PivotZone::Values => &self.value_fmt,
            _ => self.label_formats.get(dialog.field)?,
        };
        // Report the kind-effective alignment (a Text field's labels follow
        // its string alignment, not the number format's).
        let mut number = fmt.number;
        number.alignment = fmt.alignment();
        Some(number)
    }

    /// Apply `mutate` to the open dialog's target format (stored as a config
    /// override) and recompute.
    pub fn update_format_dialog(&mut self, mutate: impl FnOnce(&mut NumberFormat)) {
        let Some(dialog) = self.format_dialog else {
            return;
        };
        let Some(mut fmt) = self.format_dialog_format() else {
            return;
        };
        mutate(&mut fmt);
        match dialog.zone {
            PivotZone::Values => self.config.value_format = Some(fmt),
            _ => {
                self.config.field_formats.insert(dialog.field, fmt);
            }
        }
        self.recompute();
    }

    /// Drop the open dialog's override, reverting the field to its resolved
    /// default format.
    pub fn reset_format_dialog(&mut self) {
        let Some(dialog) = self.format_dialog else {
            return;
        };
        match dialog.zone {
            PivotZone::Values => self.config.value_format = None,
            _ => {
                self.config.field_formats.remove(&dialog.field);
            }
        }
        self.recompute();
    }

    // ------------------------------------------------------------------
    // Right-click menu / drill-through
    // ------------------------------------------------------------------

    /// Register (or replace) the right-click menu provider. When set, the
    /// provider fully controls the pivot's context menu; built-in `pivot.*`
    /// action ids remain handled by the pivot itself.
    pub fn set_context_menu_provider(
        &mut self,
        provider: impl crate::pivot::context_menu::PivotContextMenuProvider + 'static,
    ) {
        self.context_menu_provider = Some(PivotContextMenuProviderHandle::new(provider));
    }

    /// Register (or replace) the save-configuration action. While registered,
    /// the sidebar renders a save button next to the Layout section that
    /// invokes the handler with the live [`PivotConfig`].
    pub fn on_save_config(&mut self, handler: impl Fn(&PivotConfig, &mut App) + 'static) {
        self.save_config_handler = Some(Rc::new(handler));
    }

    /// Remove the save-configuration action; the sidebar's save button
    /// disappears.
    pub fn clear_save_config_handler(&mut self) {
        self.save_config_handler = None;
    }

    /// Whether a save-configuration action is currently registered.
    #[must_use]
    pub fn has_save_config_handler(&self) -> bool {
        self.save_config_handler.is_some()
    }

    /// The registered save-configuration action, if any.
    #[must_use]
    pub fn save_config_handler(&self) -> Option<PivotSaveConfigHandler> {
        self.save_config_handler.clone()
    }

    /// The grouping path for a row-axis key ([`TOTAL_KEY`] → empty).
    #[must_use]
    pub fn row_path_components(&self, row_key: usize) -> Vec<PivotPathComponent> {
        path_components(
            &self.result.row_nodes,
            &self.config.row_fields,
            &self.source_columns,
            &self.config.blank_label,
            row_key,
        )
    }

    /// The grouping path for a column-axis key ([`TOTAL_KEY`] → empty).
    #[must_use]
    pub fn col_path_components(&self, col_key: usize) -> Vec<PivotPathComponent> {
        path_components(
            &self.result.col_nodes,
            &self.config.column_fields,
            &self.source_columns,
            &self.config.blank_label,
            col_key,
        )
    }

    /// Indices into the source rows that drive the `(row_key, col_key)`
    /// intersection: rows passing the pivot's source filters whose grouping
    /// labels match both paths. [`TOTAL_KEY`] on either axis means "no
    /// constraint on that axis", so totals and subtotals resolve naturally.
    #[must_use]
    pub fn source_rows_for(&self, row_key: usize, col_key: usize) -> Vec<usize> {
        let constraints: Vec<(usize, String)> = self
            .row_path_components(row_key)
            .into_iter()
            .chain(self.col_path_components(col_key))
            .map(|c| (c.field_index, c.label))
            .collect();
        rows_matching_path(
            &self.source_rows,
            &self.label_formats,
            &self.config.blank_label,
            &self.filtered_source_rows(),
            &constraints,
        )
    }

    /// Per-source-column allowed formatted values that reproduce the
    /// `(row_key, col_key)` cell's driving rows as flat-grid value filters:
    /// the pivot's active source filters plus one allow-set per grouping
    /// path component. Blank groups map to the empty formatted value the
    /// grid's filter pipeline produces for null cells.
    #[must_use]
    pub fn drill_down_filters(
        &self,
        row_key: usize,
        col_key: usize,
    ) -> Vec<(usize, HashSet<String>)> {
        let mut out: Vec<(usize, HashSet<String>)> = self
            .config
            .filter_fields
            .iter()
            .filter_map(|&f| self.filter_values.get(&f).map(|set| (f, set.clone())))
            .collect();
        for comp in self
            .row_path_components(row_key)
            .into_iter()
            .chain(self.col_path_components(col_key))
        {
            let set = drill_filter_set(&comp.label, &self.config.blank_label);
            if let Some(entry) = out.iter_mut().find(|(f, _)| *f == comp.field_index) {
                entry.1 = set;
            } else {
                out.push((comp.field_index, set));
            }
        }
        out
    }

    /// Queue a drill-through for the visible cell at `(row, col)`: the host
    /// widget applies the resulting filters to the flat grid and switches
    /// to the Grid tab. Triggered by double-click and by the built-in
    /// "Show source rows in Grid" menu action.
    pub fn request_drill_down(&mut self, row: usize, col: usize) {
        let (Some(vr), Some(vc)) = (
            self.visible_rows.get(row).copied(),
            self.visible_cols.get(col).copied(),
        ) else {
            return;
        };
        self.pending_drill_down = Some(self.drill_down_filters(vr.key, vc.key));
    }

    /// Take the queued drill-through, if any. Called by the host widget.
    pub(crate) fn take_pending_drill_down(&mut self) -> Option<Vec<(usize, HashSet<String>)>> {
        self.pending_drill_down.take()
    }

    /// Build the [`PivotCellContext`] for a visible cell.
    fn cell_context(&self, vr: &VisibleRow, vc: &VisibleCol) -> PivotCellContext {
        let value = self.result.value(vr.key, vc.key).clone();
        let formatted_value = if matches!(value, CellValue::None) {
            String::new()
        } else {
            format_cell(&value, &self.value_fmt).0
        };
        PivotCellContext {
            row_path: self.row_path_components(vr.key),
            col_path: self.col_path_components(vc.key),
            value,
            formatted_value,
            is_row_grand_total: vr.kind == VisibleRowKind::GrandTotal,
            is_col_grand_total: vc.kind == VisibleColKind::GrandTotal,
            is_row_subtotal: matches!(vr.kind, VisibleRowKind::GroupHeader { .. }),
            is_col_subtotal: matches!(
                vc.kind,
                VisibleColKind::Subtotal | VisibleColKind::Collapsed
            ),
        }
    }

    /// Open the right-click menu for a hit-test result at the given
    /// grid-relative anchor. Builds the [`PivotContextMenuRequest`] and asks
    /// the provider (or the built-in default) for items. Non-interactive
    /// hits close any open menu.
    pub fn open_context_menu(&mut self, hit: PivotHitResult, anchor: Point<Pixels>) {
        self.filter_popover = None;
        let resolved: Option<(PivotMenuTarget, usize, usize)> = match hit {
            PivotHitResult::Cell { row, col } => {
                match (self.visible_rows.get(row), self.visible_cols.get(col)) {
                    (Some(vr), Some(vc)) => {
                        let ctx = self.cell_context(vr, vc);
                        Some((PivotMenuTarget::Cell(ctx), vr.key, vc.key))
                    }
                    _ => None,
                }
            }
            PivotHitResult::RowHeader { row } | PivotHitResult::RowChevron { row } => {
                self.visible_rows.get(row).map(|vr| {
                    (
                        PivotMenuTarget::RowHeader {
                            path: self.row_path_components(vr.key),
                            is_grand_total: vr.kind == VisibleRowKind::GrandTotal,
                        },
                        vr.key,
                        TOTAL_KEY,
                    )
                })
            }
            PivotHitResult::ColHeader { level: 0, .. } | PivotHitResult::Corner => {
                Some((PivotMenuTarget::Corner, TOTAL_KEY, TOTAL_KEY))
            }
            PivotHitResult::ColHeader { level, col } => {
                let depth = level - 1;
                let key = self
                    .col_ancestor_at(col, depth)
                    .or_else(|| self.visible_cols.get(col).map(|vc| vc.key));
                key.map(|key| {
                    let is_grand = key == TOTAL_KEY
                        && self
                            .visible_cols
                            .get(col)
                            .is_some_and(|vc| vc.kind == VisibleColKind::GrandTotal);
                    (
                        PivotMenuTarget::ColHeader {
                            path: self.col_path_components(key),
                            is_grand_total: is_grand,
                        },
                        TOTAL_KEY,
                        key,
                    )
                })
            }
            PivotHitResult::ColChevron { col, depth } => {
                self.col_group_run_at(col, depth).map(|(node, _)| {
                    (
                        PivotMenuTarget::ColHeader {
                            path: self.col_path_components(node),
                            is_grand_total: false,
                        },
                        TOTAL_KEY,
                        node,
                    )
                })
            }
            PivotHitResult::None
            | PivotHitResult::RowBorder { .. }
            | PivotHitResult::ColBorder { .. }
            | PivotHitResult::VScrollbar
            | PivotHitResult::HScrollbar => None,
        };
        let Some((target, row_key, col_key)) = resolved else {
            self.menu = None;
            return;
        };

        let request = PivotContextMenuRequest {
            source_row_indices: self.source_rows_for(row_key, col_key),
            target,
            aggregation: self.config.aggregation,
            value_field_index: self.config.value_field,
            value_caption: self.result.value_caption.clone(),
            config: self.config.clone(),
        };
        let items = match &self.context_menu_provider {
            Some(provider) => provider.menu_items(&request),
            None => PivotMenuItem::standard_items(&request.target),
        };
        if items.is_empty() {
            self.menu = None;
            return;
        }
        let drill = self.drill_down_filters(row_key, col_key);
        self.menu = Some(PivotMenu {
            anchor,
            items,
            hovered: None,
            request,
            drill,
        });
    }

    /// Execute a clicked menu item. Built-in `pivot.*` ids are handled here;
    /// everything else is dispatched to the registered provider.
    pub(crate) fn execute_menu_action(&mut self, id: &str, menu: PivotMenu, cx: &mut App) {
        match id {
            PIVOT_ACTION_SHOW_SOURCE_ROWS => {
                self.pending_drill_down = Some(menu.drill);
            }
            PIVOT_ACTION_COPY_VALUE => {
                if let Some(cell) = menu.request.clicked_cell() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        cell.formatted_value.clone(),
                    ));
                }
            }
            PIVOT_ACTION_COPY_CSV => {
                let csv = self.to_csv();
                if !csv.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(csv));
                }
            }
            _ => {
                if let Some(provider) = self.context_menu_provider.clone() {
                    provider.on_action(id, &menu.request, self, cx);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Labels
    // ------------------------------------------------------------------

    /// Display label for a visible row.
    #[must_use]
    pub fn row_label(&self, row: &VisibleRow) -> String {
        match row.kind {
            VisibleRowKind::GrandTotal => "Grand Total".into(),
            _ if row.key == TOTAL_KEY => "Total".into(),
            _ => self
                .result
                .row_nodes
                .get(row.key)
                .map(|n| n.label.clone())
                .unwrap_or_default(),
        }
    }

    /// Display label for a visible column (as shown at the innermost header
    /// level).
    #[must_use]
    pub fn col_label(&self, col: &VisibleCol) -> String {
        match col.kind {
            VisibleColKind::GrandTotal => "Grand Total".into(),
            VisibleColKind::Subtotal | VisibleColKind::Collapsed => self
                .result
                .col_nodes
                .get(col.key)
                .map(|n| format!("{} Total", n.label))
                .unwrap_or_else(|| "Total".into()),
            _ if col.key == TOTAL_KEY => self.result.value_caption.clone(),
            _ => self
                .result
                .col_nodes
                .get(col.key)
                .map(|n| n.label.clone())
                .unwrap_or_default(),
        }
    }

    /// The value cell for a visible (row, col) pair.
    #[must_use]
    pub fn cell_value(&self, row: &VisibleRow, col: &VisibleCol) -> &CellValue {
        self.result.value(row.key, col.key)
    }

    // ------------------------------------------------------------------
    // Geometry / hit testing
    // ------------------------------------------------------------------

    /// Current height of every pivot data row, in logical pixels.
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.row_height
    }

    /// Set the height of every pivot data row.
    ///
    /// Non-finite values are ignored and finite values are clamped to the
    /// minimum supported height.
    pub fn set_row_height(&mut self, height: f32) {
        if height.is_finite() {
            self.row_height = height.max(MIN_PIVOT_ROW_HEIGHT);
            self.clamp_scroll_to_bounds();
        }
    }

    /// Current width of every pivot value column, in logical pixels.
    #[must_use]
    pub fn column_width(&self) -> f32 {
        self.value_col_width
    }

    /// Set the width of every pivot value column.
    ///
    /// Non-finite values are ignored and finite values are clamped to the
    /// minimum supported width.
    pub fn set_column_width(&mut self, width: f32) {
        if width.is_finite() {
            self.value_col_width = width.max(MIN_PIVOT_COLUMN_WIDTH);
            self.clamp_scroll_to_bounds();
        }
    }

    /// Number of stacked header rows: one caption row plus one row per
    /// column-field level (minimum one).
    #[must_use]
    pub fn header_levels(&self) -> usize {
        1 + self.result.col_depth.max(1)
    }

    /// Total header block height in pixels.
    #[must_use]
    pub fn header_height(&self) -> f32 {
        self.header_levels() as f32 * self.header_row_height
    }

    /// Content size of the data region (all rows × all columns), in pixels.
    #[must_use]
    pub fn content_size(&self) -> (f32, f32) {
        (
            self.visible_cols.len() as f32 * self.value_col_width,
            self.visible_rows.len() as f32 * self.row_height,
        )
    }

    pub(crate) fn scrollbar_reserved(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let vw = f32::from(self.bounds.size.width) - self.row_header_width;
        let vh = f32::from(self.bounds.size.height) - self.header_height();
        let reserved_w = if ch > vh { SCROLLBAR_SIZE } else { 0.0 };
        let reserved_h = if cw > vw { SCROLLBAR_SIZE } else { 0.0 };
        (reserved_w, reserved_h)
    }

    pub(crate) fn max_scroll(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let (rw, rh) = self.scrollbar_reserved();
        let vw = f32::from(self.bounds.size.width) - self.row_header_width - rw;
        let vh = f32::from(self.bounds.size.height) - self.header_height() - rh;
        ((cw - vw).max(0.0), (ch - vh).max(0.0))
    }

    /// Clamp the scroll offset into the valid range after layout changes.
    pub(crate) fn clamp_scroll_to_bounds(&mut self) {
        let (mx, my) = self.max_scroll();
        let s = self.scroll_handle.offset();
        let nx = f32::from(s.x).clamp(0.0, mx);
        let ny = f32::from(s.y).clamp(0.0, my);
        if nx != f32::from(s.x) || ny != f32::from(s.y) {
            self.scroll_handle.set_offset(Point {
                x: px(nx),
                y: px(ny),
            });
        }
    }

    fn clamp_selection(&mut self) {
        let (nr, nc) = (self.visible_rows.len(), self.visible_cols.len());
        if let Some((r1, c1, r2, c2)) = self.selection {
            if nr == 0 || nc == 0 || r1 >= nr || c1 >= nc {
                self.selection = None;
                self.select_anchor = None;
            } else {
                self.selection = Some((r1, c1, r2.min(nr - 1), c2.min(nc - 1)));
            }
        }
    }

    /// Resolve a grid-relative pointer position.
    #[must_use]
    pub fn hit_test(&self, pos: Point<Pixels>) -> PivotHitResult {
        let x = f32::from(pos.x);
        let y = f32::from(pos.y);
        let bw = f32::from(self.bounds.size.width);
        let bh = f32::from(self.bounds.size.height);
        if x < 0.0 || y < 0.0 || x > bw || y > bh {
            return PivotHitResult::None;
        }
        let hdr_h = self.header_height();
        let (mx, my) = self.max_scroll();
        if my > 0.0 && x >= bw - SCROLLBAR_SIZE && y >= hdr_h {
            return PivotHitResult::VScrollbar;
        }
        if mx > 0.0 && y >= bh - SCROLLBAR_SIZE && x >= self.row_header_width {
            return PivotHitResult::HScrollbar;
        }
        let sx = f32::from(self.scroll_handle.offset().x);
        let sy = f32::from(self.scroll_handle.offset().y);

        if y < hdr_h {
            if x < self.row_header_width {
                return PivotHitResult::Corner;
            }
            let level = ((y / self.header_row_height) as usize).min(self.header_levels() - 1);
            let cx = x - self.row_header_width + sx;
            if cx < 0.0 {
                return PivotHitResult::None;
            }
            let boundary = (cx / self.value_col_width).round() as usize;
            if boundary > 0
                && boundary <= self.visible_cols.len()
                && (cx - boundary as f32 * self.value_col_width).abs() <= RESIZE_HIT_SLOP
            {
                return PivotHitResult::ColBorder { col: boundary - 1 };
            }
            let col = (cx / self.value_col_width) as usize;
            if col >= self.visible_cols.len() {
                return PivotHitResult::None;
            }
            // Chevron: painted at the start of a group's run, at header
            // level `depth + 1`.
            if level >= 1 {
                let depth = level - 1;
                if let Some((_, run_start)) = self.col_group_run_at(col, depth) {
                    let run_x0 = run_start as f32 * self.value_col_width;
                    let within = cx - run_x0;
                    let is_group = self
                        .col_ancestor_at(col, depth)
                        .map(|n| !self.result.col_nodes[n].is_leaf())
                        .unwrap_or(false);
                    if is_group && (2.0..=2.0 + CHEVRON_SIZE).contains(&within) {
                        return PivotHitResult::ColChevron {
                            col: run_start,
                            depth,
                        };
                    }
                }
            }
            return PivotHitResult::ColHeader { level, col };
        }

        if x < self.row_header_width {
            let ry = y - hdr_h + sy;
            if ry < 0.0 {
                return PivotHitResult::None;
            }
            let boundary = (ry / self.row_height).round() as usize;
            if boundary > 0
                && boundary <= self.visible_rows.len()
                && (ry - boundary as f32 * self.row_height).abs() <= RESIZE_HIT_SLOP
            {
                return PivotHitResult::RowBorder { row: boundary - 1 };
            }
            let row = (ry / self.row_height) as usize;
            if row >= self.visible_rows.len() {
                return PivotHitResult::None;
            }
            let vr = self.visible_rows[row];
            if matches!(vr.kind, VisibleRowKind::GroupHeader { .. }) {
                let indent = vr.depth as f32 * ROW_INDENT + 4.0;
                if x >= indent && x <= indent + CHEVRON_SIZE {
                    return PivotHitResult::RowChevron { row };
                }
            }
            return PivotHitResult::RowHeader { row };
        }

        let cx = x - self.row_header_width + sx;
        let ry = y - hdr_h + sy;
        if cx < 0.0 || ry < 0.0 {
            return PivotHitResult::None;
        }
        let col = (cx / self.value_col_width) as usize;
        let row = (ry / self.row_height) as usize;
        if row < self.visible_rows.len() && col < self.visible_cols.len() {
            PivotHitResult::Cell { row, col }
        } else {
            PivotHitResult::None
        }
    }

    /// For a visible column and a header depth: the group node shown there
    /// plus the first visible column of its contiguous run. `None` when the
    /// column has no ancestor at that depth (e.g. the grand-total column).
    pub(crate) fn col_group_run_at(&self, col: usize, depth: usize) -> Option<(usize, usize)> {
        let node = self.col_ancestor_at(col, depth)?;
        let mut start = col;
        while start > 0 && self.col_ancestor_at(start - 1, depth) == Some(node) {
            start -= 1;
        }
        Some((node, start))
    }

    /// The column-axis node displayed at `depth` for visible column `col`.
    pub(crate) fn col_ancestor_at(&self, col: usize, depth: usize) -> Option<usize> {
        let vc = self.visible_cols.get(col)?;
        if vc.key == TOTAL_KEY {
            return None;
        }
        let mut id = vc.key;
        let mut d = self.result.col_nodes.get(id)?.depth;
        if depth > d {
            return None;
        }
        while d > depth {
            id = self.result.col_nodes[id].parent?;
            d -= 1;
        }
        Some(id)
    }

    // ------------------------------------------------------------------
    // Mouse / keyboard behavior (called by the widget)
    // ------------------------------------------------------------------

    /// Handle a left mouse-down at a grid-relative position.
    pub fn handle_mouse_down(&mut self, pos: Point<Pixels>, shift: bool) {
        let hit = self.hit_test(pos);
        match hit {
            PivotHitResult::VScrollbar => {
                self.scrollbar_drag = Some(ScrollbarAxis::Vertical);
                self.scroll_to_vbar(f32::from(pos.y));
            }
            PivotHitResult::HScrollbar => {
                self.scrollbar_drag = Some(ScrollbarAxis::Horizontal);
                self.scroll_to_hbar(f32::from(pos.x));
            }
            PivotHitResult::RowBorder { row } => {
                self.resize_drag = Some(PivotResizeDrag::Row {
                    boundary: row + 1,
                    start_y: f32::from(pos.y),
                    start_height: self.row_height,
                });
            }
            PivotHitResult::ColBorder { col } => {
                self.resize_drag = Some(PivotResizeDrag::Column {
                    boundary: col + 1,
                    start_x: f32::from(pos.x),
                    start_width: self.value_col_width,
                });
            }
            PivotHitResult::RowChevron { row } => {
                if let Some(vr) = self.visible_rows.get(row).copied() {
                    self.toggle_row_group(vr.key);
                }
            }
            PivotHitResult::ColChevron { col, depth } => {
                if let Some((node, _)) = self.col_group_run_at(col, depth) {
                    self.toggle_col_group(node);
                } else if let Some(vc) = self.visible_cols.get(col) {
                    if vc.key != TOTAL_KEY {
                        self.toggle_col_group(vc.key);
                    }
                }
            }
            PivotHitResult::Corner => self.cycle_label_sort(),
            PivotHitResult::ColHeader { level, col } => {
                // Innermost header level sorts rows by that column; higher
                // group levels toggle the group under the pointer; the
                // caption row is inert.
                if level == 0 {
                    return;
                }
                if level == self.header_levels() - 1 {
                    if let Some(vc) = self.visible_cols.get(col).copied() {
                        self.cycle_sort_by_column(vc.key);
                    }
                } else {
                    let depth = level - 1;
                    if let Some((node, _)) = self.col_group_run_at(col, depth) {
                        if !self.result.col_nodes[node].is_leaf() {
                            self.toggle_col_group(node);
                        }
                    }
                }
            }
            PivotHitResult::RowHeader { row } => {
                if self.visible_cols.is_empty() {
                    return;
                }
                let last = self.visible_cols.len() - 1;
                self.selection = Some((row, 0, row, last));
                self.select_anchor = Some((row, 0));
            }
            PivotHitResult::Cell { row, col } => {
                if shift {
                    let (ar, ac) = self.select_anchor.unwrap_or((row, col));
                    self.selection = Some(norm_rect(ar, ac, row, col));
                } else {
                    self.selection = Some((row, col, row, col));
                    self.select_anchor = Some((row, col));
                    self.is_selecting = true;
                }
            }
            PivotHitResult::None => {
                self.selection = None;
                self.select_anchor = None;
            }
        }
    }

    /// Handle pointer movement (hover, drag-selection, scrollbar drags).
    pub fn handle_mouse_move(&mut self, pos: Point<Pixels>, left_down: bool) {
        if let Some(drag) = self.resize_drag {
            if !left_down {
                self.resize_drag = None;
            } else {
                match drag {
                    PivotResizeDrag::Row {
                        boundary,
                        start_y,
                        start_height,
                    } => {
                        let delta = (f32::from(pos.y) - start_y) / boundary as f32;
                        self.set_row_height(start_height + delta);
                        self.hover_hit = Some(PivotHitResult::RowBorder { row: boundary - 1 });
                    }
                    PivotResizeDrag::Column {
                        boundary,
                        start_x,
                        start_width,
                    } => {
                        let delta = (f32::from(pos.x) - start_x) / boundary as f32;
                        self.set_column_width(start_width + delta);
                        self.hover_hit = Some(PivotHitResult::ColBorder { col: boundary - 1 });
                    }
                }
                return;
            }
        }
        if let Some(axis) = self.scrollbar_drag {
            if left_down {
                match axis {
                    ScrollbarAxis::Vertical => self.scroll_to_vbar(f32::from(pos.y)),
                    ScrollbarAxis::Horizontal => self.scroll_to_hbar(f32::from(pos.x)),
                }
                return;
            }
            self.scrollbar_drag = None;
        }
        let hit = self.hit_test(pos);
        self.hover_hit = Some(hit);
        if self.is_selecting && left_down {
            if let PivotHitResult::Cell { row, col } = hit {
                if let Some((ar, ac)) = self.select_anchor {
                    self.selection = Some(norm_rect(ar, ac, row, col));
                }
            }
        } else if self.is_selecting {
            self.is_selecting = false;
        }
    }

    /// Handle mouse-up: end any drag.
    pub fn handle_mouse_up(&mut self) {
        self.is_selecting = false;
        self.scrollbar_drag = None;
        self.resize_drag = None;
    }

    /// Apply a scroll-wheel delta, clamped to the content.
    pub fn apply_scroll_delta(&mut self, dx: f32, dy: f32) {
        let (mx, my) = self.max_scroll();
        let s = self.scroll_handle.offset();
        let nx = (f32::from(s.x) - dx).clamp(0.0, mx);
        let ny = (f32::from(s.y) - dy).clamp(0.0, my);
        self.scroll_handle.set_offset(Point {
            x: px(nx),
            y: px(ny),
        });
    }

    pub(crate) fn scroll_to_vbar(&mut self, mouse_y: f32) {
        let (_, my) = self.max_scroll();
        let hdr = self.header_height();
        let (_, rh) = self.scrollbar_reserved();
        let track_h = f32::from(self.bounds.size.height) - hdr - rh;
        let (_, ch) = self.content_size();
        if track_h <= 0.0 || ch <= 0.0 {
            return;
        }
        let thumb_h = ((track_h * (track_h / ch)).max(20.0)).min(track_h);
        let range = (track_h - thumb_h).max(0.0);
        let rel = (mouse_y - hdr - thumb_h * 0.5).clamp(0.0, range);
        let frac = if range > 0.0 { rel / range } else { 0.0 };
        let x = self.scroll_handle.offset().x;
        self.scroll_handle.set_offset(Point {
            x,
            y: px(frac * my),
        });
    }

    pub(crate) fn scroll_to_hbar(&mut self, mouse_x: f32) {
        let (mx, _) = self.max_scroll();
        let (rw, _) = self.scrollbar_reserved();
        let track_x = self.row_header_width;
        let track_w = f32::from(self.bounds.size.width) - self.row_header_width - rw;
        let (cw, _) = self.content_size();
        if track_w <= 0.0 || cw <= 0.0 {
            return;
        }
        let thumb_w = ((track_w * (track_w / cw)).max(20.0)).min(track_w);
        let range = (track_w - thumb_w).max(0.0);
        let rel = (mouse_x - track_x - thumb_w * 0.5).clamp(0.0, range);
        let frac = if range > 0.0 { rel / range } else { 0.0 };
        let y = self.scroll_handle.offset().y;
        self.scroll_handle.set_offset(Point {
            x: px(frac * mx),
            y,
        });
    }

    /// Handle a keystroke: copy bindings, escape, arrow-key selection moves.
    pub fn handle_key(&mut self, ks: &gpui::Keystroke, cx: &mut App) {
        if self.key_bindings.copy.matches(ks) {
            self.copy_selection(false, cx);
            return;
        }
        if self.key_bindings.copy_with_headers.matches(ks) {
            self.copy_selection(true, cx);
            return;
        }
        match ks.key.as_str() {
            "escape" => {
                self.selection = None;
                self.select_anchor = None;
                self.filter_popover = None;
                self.menu = None;
            }
            "up" => self.move_selection(0, -1),
            "down" => self.move_selection(0, 1),
            "left" => self.move_selection(-1, 0),
            "right" => self.move_selection(1, 0),
            _ => {}
        }
    }

    fn move_selection(&mut self, dx: i32, dy: i32) {
        let nr = self.visible_rows.len() as i32;
        let nc = self.visible_cols.len() as i32;
        if nr == 0 || nc == 0 {
            return;
        }
        let (r, c) = match self.selection {
            Some((r1, c1, ..)) => (r1 as i32, c1 as i32),
            None => (0, 0),
        };
        let r = (r + dy).clamp(0, nr - 1) as usize;
        let c = (c + dx).clamp(0, nc - 1) as usize;
        self.selection = Some((r, c, r, c));
        self.select_anchor = Some((r, c));
    }

    // ------------------------------------------------------------------
    // Copy / export
    // ------------------------------------------------------------------

    /// Copy the selected rectangle (or the whole visible pivot when nothing
    /// is selected) to the clipboard as TSV.
    pub fn copy_selection(&self, with_headers: bool, cx: &mut App) {
        let text = self.selection_text(with_headers, '\t');
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    /// Build the copy text for the current selection (whole pivot if none),
    /// using `sep` between fields.
    #[must_use]
    pub fn selection_text(&self, with_headers: bool, sep: char) -> String {
        use std::fmt::Write as _;
        if self.visible_rows.is_empty() || self.visible_cols.is_empty() {
            return String::new();
        }
        let (r1, c1, r2, c2) = self.selection.unwrap_or((
            0,
            0,
            self.visible_rows.len() - 1,
            self.visible_cols.len() - 1,
        ));
        let mut out = String::new();
        if with_headers {
            for c in c1..=c2 {
                out.push(sep);
                out.push_str(&self.col_label(&self.visible_cols[c]));
            }
            out.push('\n');
        }
        for r in r1..=r2 {
            let vr = &self.visible_rows[r];
            if with_headers {
                let _ = write!(out, "{}{}", "  ".repeat(vr.depth), self.row_label(vr));
                out.push(sep);
            }
            for c in c1..=c2 {
                if c > c1 {
                    out.push(sep);
                }
                let cell = self.cell_value(vr, &self.visible_cols[c]);
                if !matches!(cell, CellValue::None) {
                    out.push_str(&format_cell(cell, &self.value_fmt).0);
                }
            }
            out.push('\n');
        }
        out
    }

    /// Export the **fully expanded** pivot as CSV: one column per row field
    /// (hierarchical labels), one column per leaf column, plus totals as
    /// configured. Ignores the current collapse state.
    #[must_use]
    pub fn to_csv(&self) -> String {
        let res = &self.result;
        let mut out = String::new();
        let col_leaves = res.col_leaves();
        let want_grand_col = self.config.show_column_grand_total && !col_leaves.is_empty();

        let mut header: Vec<String> = if res.row_depth == 0 {
            vec![String::new()]
        } else {
            res.row_field_names.clone()
        };
        if col_leaves.is_empty() {
            header.push(res.value_caption.clone());
        } else {
            for &leaf in &col_leaves {
                header.push(node_path(&res.col_nodes, leaf).join(" / "));
            }
        }
        if want_grand_col {
            header.push("Grand Total".into());
        }
        push_csv_line(&mut out, &header);

        let value_cols: Vec<usize> = if col_leaves.is_empty() {
            vec![TOTAL_KEY]
        } else {
            col_leaves
        };
        let fmt_value = |cell: &CellValue| {
            if matches!(cell, CellValue::None) {
                String::new()
            } else {
                format_cell(cell, &self.value_fmt).0
            }
        };
        let emit_line = |out: &mut String, labels: Vec<String>, row_key: usize| {
            let mut fields = labels;
            for &ck in &value_cols {
                fields.push(fmt_value(res.value(row_key, ck)));
            }
            if want_grand_col {
                fields.push(fmt_value(res.value(row_key, TOTAL_KEY)));
            }
            push_csv_line(out, &fields);
        };

        let row_leaves = res.row_leaves();
        if row_leaves.is_empty() && res.row_depth == 0 && !res.values.is_empty() {
            emit_line(&mut out, vec!["Total".into()], TOTAL_KEY);
        }
        for &leaf in &row_leaves {
            let mut labels = node_path(&res.row_nodes, leaf);
            while labels.len() < res.row_depth {
                labels.push(String::new());
            }
            emit_line(&mut out, labels, leaf);
        }
        if self.config.show_row_grand_total && !row_leaves.is_empty() {
            let mut labels = vec!["Grand Total".to_owned()];
            while labels.len() < res.row_depth.max(1) {
                labels.push(String::new());
            }
            emit_line(&mut out, labels, TOTAL_KEY);
        }
        out
    }
}

fn default_value_format() -> ResolvedColumnFormat {
    crate::config::GridConfig::default().resolve(usize::MAX, ColumnKind::Decimal)
}

fn norm_rect(r1: usize, c1: usize, r2: usize, c2: usize) -> (usize, usize, usize, usize) {
    (r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2))
}

fn push_csv_line(out: &mut String, fields: &[String]) {
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if f.contains(',') || f.contains('"') || f.contains('\n') {
            out.push('"');
            out.push_str(&f.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(f);
        }
    }
    out.push('\n');
}

/// The label path from the root down to `node`, e.g. `["Europe", "Widget"]`.
pub(crate) fn node_path(nodes: &[PivotNode], node: usize) -> Vec<String> {
    let mut path = Vec::new();
    let mut current = Some(node);
    while let Some(id) = current {
        path.push(nodes[id].label.clone());
        current = nodes[id].parent;
    }
    path.reverse();
    path
}

/// The full grouping path (root → `key`) as public path components.
/// [`TOTAL_KEY`] and out-of-range keys yield an empty path.
pub(crate) fn path_components(
    nodes: &[PivotNode],
    fields: &[usize],
    columns: &[Column],
    blank_label: &str,
    key: usize,
) -> Vec<PivotPathComponent> {
    if key == TOTAL_KEY || key >= nodes.len() {
        return Vec::new();
    }
    let mut chain = Vec::new();
    let mut current = Some(key);
    while let Some(id) = current {
        chain.push(id);
        current = nodes[id].parent;
    }
    chain.reverse();
    chain
        .into_iter()
        .map(|id| {
            let node = &nodes[id];
            let field_index = fields.get(node.depth).copied().unwrap_or(usize::MAX);
            PivotPathComponent {
                field_index,
                field_name: columns
                    .get(field_index)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                label: node.label.clone(),
                group_value: node.sort_key.clone(),
                is_blank: node.label == blank_label,
            }
        })
        .collect()
}

/// Filter `base` down to the rows whose grouping labels satisfy every
/// `(field, label)` constraint, using the same labeling rule as the engine
/// (null cells take `blank_label`, everything else its formatted value).
pub(crate) fn rows_matching_path(
    rows: &[Vec<CellValue>],
    formats: &[ResolvedColumnFormat],
    blank_label: &str,
    base: &[usize],
    constraints: &[(usize, String)],
) -> Vec<usize> {
    if constraints.is_empty() {
        return base.to_vec();
    }
    base.iter()
        .copied()
        .filter(|&r| {
            constraints.iter().all(|(field, label)| {
                let cell = rows
                    .get(r)
                    .and_then(|row| row.get(*field))
                    .unwrap_or(&CellValue::None);
                let cell_label = if matches!(cell, CellValue::None) {
                    blank_label.to_owned()
                } else {
                    format_cell(cell, &formats[*field]).0
                };
                cell_label == *label
            })
        })
        .collect()
}

/// The flat-grid filter allow-set that selects a pivot group's rows. Blank
/// groups match the empty formatted value the grid produces for null cells
/// (plus the literal blank label, since the engine merges both into one
/// group).
pub(crate) fn drill_filter_set(label: &str, blank_label: &str) -> HashSet<String> {
    if label == blank_label {
        HashSet::from([String::new(), label.to_owned()])
    } else {
        HashSet::from([label.to_owned()])
    }
}

/// Paths of every non-leaf node (used by collapse-all).
fn all_group_paths(nodes: &[PivotNode]) -> HashSet<Vec<String>> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_leaf())
        .map(|(id, _)| node_path(nodes, id))
        .collect()
}

/// Flatten the row tree into display rows, honoring the collapse set.
pub(crate) fn flatten_rows(
    result: &PivotResult,
    collapsed: &HashSet<Vec<String>>,
    config: &PivotConfig,
) -> Vec<VisibleRow> {
    let mut out = Vec::new();
    if result.row_depth == 0 {
        // No row fields: a single total row (when there is anything to show).
        if !result.values.is_empty() {
            out.push(VisibleRow {
                key: TOTAL_KEY,
                depth: 0,
                kind: VisibleRowKind::Leaf,
                zebra: false,
            });
        }
        return out;
    }
    if config.flat_rows {
        // Flat/tabular layout: one row per innermost leaf combination, in the
        // current sort order (`row_leaves` walks the already-resorted tree),
        // with no group-header rows, indentation, or subtotals. Each field's
        // value is painted in its own row-header column (see `paint.rs`).
        // Zebra alternates across the whole flat list rather than resetting
        // per group.
        let mut zebra = false;
        for leaf in result.row_leaves() {
            out.push(VisibleRow {
                key: leaf,
                depth: result.row_nodes[leaf].depth,
                kind: VisibleRowKind::Leaf,
                zebra,
            });
            zebra = !zebra;
        }
        if config.show_row_grand_total && !out.is_empty() {
            out.push(VisibleRow {
                key: TOTAL_KEY,
                depth: 0,
                kind: VisibleRowKind::GrandTotal,
                zebra: false,
            });
        }
        return out;
    }
    fn walk(
        result: &PivotResult,
        collapsed: &HashSet<Vec<String>>,
        id: usize,
        path: &mut Vec<String>,
        zebra: &mut bool,
        out: &mut Vec<VisibleRow>,
    ) {
        let node = &result.row_nodes[id];
        path.push(node.label.clone());
        if node.is_leaf() {
            out.push(VisibleRow {
                key: id,
                depth: node.depth,
                kind: VisibleRowKind::Leaf,
                zebra: *zebra,
            });
            *zebra = !*zebra;
        } else if collapsed.contains(path) {
            *zebra = false;
            out.push(VisibleRow {
                key: id,
                depth: node.depth,
                kind: VisibleRowKind::GroupHeader { expanded: false },
                zebra: false,
            });
        } else {
            // Striping restarts inside every group.
            *zebra = false;
            out.push(VisibleRow {
                key: id,
                depth: node.depth,
                kind: VisibleRowKind::GroupHeader { expanded: true },
                zebra: false,
            });
            for &child in &node.children {
                walk(result, collapsed, child, path, zebra, out);
            }
            *zebra = false;
        }
        path.pop();
    }
    let mut path = Vec::new();
    let mut zebra = false;
    for &root in &result.row_roots {
        walk(result, collapsed, root, &mut path, &mut zebra, &mut out);
    }
    if config.show_row_grand_total && !out.is_empty() {
        out.push(VisibleRow {
            key: TOTAL_KEY,
            depth: 0,
            kind: VisibleRowKind::GrandTotal,
            zebra: false,
        });
    }
    out
}

/// Flatten the column tree into display columns, honoring the collapse set.
pub(crate) fn flatten_cols(
    result: &PivotResult,
    collapsed: &HashSet<Vec<String>>,
    config: &PivotConfig,
) -> Vec<VisibleCol> {
    let mut out = Vec::new();
    if result.col_depth == 0 {
        if !result.values.is_empty() {
            out.push(VisibleCol {
                key: TOTAL_KEY,
                depth: 0,
                kind: VisibleColKind::Leaf,
            });
        }
        return out;
    }
    fn walk(
        result: &PivotResult,
        collapsed: &HashSet<Vec<String>>,
        config: &PivotConfig,
        id: usize,
        path: &mut Vec<String>,
        out: &mut Vec<VisibleCol>,
    ) {
        let node = &result.col_nodes[id];
        path.push(node.label.clone());
        if node.is_leaf() {
            out.push(VisibleCol {
                key: id,
                depth: node.depth,
                kind: VisibleColKind::Leaf,
            });
        } else if collapsed.contains(path) {
            out.push(VisibleCol {
                key: id,
                depth: node.depth,
                kind: VisibleColKind::Collapsed,
            });
        } else {
            for &child in &node.children {
                walk(result, collapsed, config, child, path, out);
            }
            if config.show_column_subtotals {
                out.push(VisibleCol {
                    key: id,
                    depth: node.depth,
                    kind: VisibleColKind::Subtotal,
                });
            }
        }
        path.pop();
    }
    let mut path = Vec::new();
    for &root in &result.col_roots {
        walk(result, collapsed, config, root, &mut path, &mut out);
    }
    if config.show_column_grand_total && !out.is_empty() {
        out.push(VisibleCol {
            key: TOTAL_KEY,
            depth: 0,
            kind: VisibleColKind::GrandTotal,
        });
    }
    out
}

/// Sort both roots and every sibling list by the nodes' grouping value.
fn sort_axis_by_key(nodes: &mut [PivotNode], roots: &mut [usize], descending: bool) {
    let keys: Vec<CellValue> = nodes.iter().map(|n| n.sort_key.clone()).collect();
    sort_axis_by(nodes, roots, &keys, descending);
}

/// Sort both roots and every sibling list by a per-node key vector.
fn sort_axis_by(
    nodes: &mut [PivotNode],
    roots: &mut [usize],
    keys: &[CellValue],
    descending: bool,
) {
    let cmp = |a: &usize, b: &usize| {
        let ord = compare_cells(&keys[*a], &keys[*b]);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    };
    roots.sort_by(cmp);
    for node in nodes.iter_mut() {
        let mut children = std::mem::take(&mut node.children);
        children.sort_by(cmp);
        node.children = children;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::GridConfig;
    use crate::data::ColumnKind;
    use CellValue::{Decimal, Integer, Text};

    fn fixture_result(config: &PivotConfig) -> PivotResult {
        let columns = vec![
            Column::new("region", ColumnKind::Text, 100.0),
            Column::new("product", ColumnKind::Text, 100.0),
            Column::new("year", ColumnKind::Integer, 80.0),
            Column::new("amount", ColumnKind::Decimal, 100.0),
        ];
        let r = |region: &str, product: &str, year: i64, amount: f64| {
            vec![
                Text(region.into()),
                Text(product.into()),
                Integer(year),
                Decimal(amount),
            ]
        };
        let rows = vec![
            r("Europe", "Widget", 2023, 10.0),
            r("Europe", "Widget", 2024, 20.0),
            r("Europe", "Gadget", 2023, 5.0),
            r("Asia", "Widget", 2023, 7.0),
            r("Asia", "Gadget", 2024, 3.0),
        ];
        let formats = GridConfig::default().resolve_all(&columns);
        let all: Vec<usize> = (0..rows.len()).collect();
        compute_pivot(&columns, &rows, &all, config, &formats)
    }

    fn two_level_config() -> PivotConfig {
        PivotConfig {
            row_fields: vec![0, 1],
            column_fields: vec![2],
            value_field: Some(3),
            ..PivotConfig::default()
        }
    }

    #[test]
    fn flatten_rows_expanded_lists_headers_and_leaves() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let rows = flatten_rows(&result, &HashSet::new(), &cfg);
        // Asia(hdr) Gadget Widget Europe(hdr) Gadget Widget GrandTotal
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].kind, VisibleRowKind::GroupHeader { expanded: true });
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].kind, VisibleRowKind::Leaf);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[6].kind, VisibleRowKind::GrandTotal);
    }

    #[test]
    fn flatten_rows_collapse_hides_children() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let mut collapsed = HashSet::new();
        collapsed.insert(vec!["Asia".to_owned()]);
        let rows = flatten_rows(&result, &collapsed, &cfg);
        // Asia(collapsed) Europe(hdr) Gadget Widget GrandTotal
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[0].kind,
            VisibleRowKind::GroupHeader { expanded: false }
        );
        assert_eq!(rows[1].kind, VisibleRowKind::GroupHeader { expanded: true });
    }

    #[test]
    fn flatten_rows_without_grand_total() {
        let mut cfg = two_level_config();
        cfg.show_row_grand_total = false;
        let result = fixture_result(&cfg);
        let rows = flatten_rows(&result, &HashSet::new(), &cfg);
        assert!(rows.iter().all(|r| r.kind != VisibleRowKind::GrandTotal));
    }

    #[test]
    fn flatten_rows_flat_lists_leaf_combinations_without_hierarchy() {
        let mut cfg = two_level_config();
        cfg.flat_rows = true;
        let result = fixture_result(&cfg);
        let rows = flatten_rows(&result, &HashSet::new(), &cfg);
        // 4 distinct (region, product) leaves + grand total; no group headers.
        assert_eq!(rows.len(), 5);
        assert!(rows[..4].iter().all(|r| r.kind == VisibleRowKind::Leaf));
        assert!(rows
            .iter()
            .all(|r| !matches!(r.kind, VisibleRowKind::GroupHeader { .. })));
        assert_eq!(rows[4].kind, VisibleRowKind::GrandTotal);
        // Each leaf carries the full path (both row fields) for its tabular
        // row-header columns, in depth-first (sorted) order.
        let paths: Vec<Vec<String>> = rows[..4]
            .iter()
            .map(|r| node_path(&result.row_nodes, r.key))
            .collect();
        assert!(paths.iter().all(|p| p.len() == 2));
        assert_eq!(paths[0], vec!["Asia".to_owned(), "Gadget".to_owned()]);
        assert_eq!(paths[3], vec!["Europe".to_owned(), "Widget".to_owned()]);
    }

    #[test]
    fn flatten_rows_flat_respects_grand_total_toggle() {
        let mut cfg = two_level_config();
        cfg.flat_rows = true;
        cfg.show_row_grand_total = false;
        let result = fixture_result(&cfg);
        let rows = flatten_rows(&result, &HashSet::new(), &cfg);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.kind == VisibleRowKind::Leaf));
    }

    #[test]
    fn flatten_cols_leaves_plus_grand_total() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let cols = flatten_cols(&result, &HashSet::new(), &cfg);
        // 2023, 2024, Grand Total
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].kind, VisibleColKind::Leaf);
        assert_eq!(cols[2].kind, VisibleColKind::GrandTotal);
    }

    #[test]
    fn flatten_cols_collapsed_group_is_single_column() {
        let mut cfg = two_level_config();
        // Two column levels: year then product.
        cfg.row_fields = vec![0];
        cfg.column_fields = vec![2, 1];
        let result = fixture_result(&cfg);
        // Group labels follow the resolved column format, so derive the
        // collapse path from the first year group rather than hardcoding it.
        let first_year = result.col_roots[0];
        let mut collapsed = HashSet::new();
        collapsed.insert(vec![result.col_nodes[first_year].label.clone()]);
        let cols = flatten_cols(&result, &collapsed, &cfg);
        // 2023 collapsed → 1 col; 2024 expanded → its product leaves; + grand.
        assert_eq!(cols[0].kind, VisibleColKind::Collapsed);
        assert!(cols.len() >= 3);
    }

    #[test]
    fn flatten_cols_subtotal_columns_follow_expanded_groups() {
        let mut cfg = two_level_config();
        cfg.row_fields = vec![0];
        cfg.column_fields = vec![2, 1];
        cfg.show_column_subtotals = true;
        let result = fixture_result(&cfg);
        let cols = flatten_cols(&result, &HashSet::new(), &cfg);
        let subtotal_count = cols
            .iter()
            .filter(|c| c.kind == VisibleColKind::Subtotal)
            .count();
        assert_eq!(subtotal_count, 2); // one per year group
                                       // Each subtotal column directly follows its group's leaves.
        let first_sub = cols
            .iter()
            .position(|c| c.kind == VisibleColKind::Subtotal)
            .unwrap();
        assert!(first_sub > 0);
        assert_eq!(cols[first_sub - 1].kind, VisibleColKind::Leaf);
    }

    #[test]
    fn flatten_cols_no_column_fields_yields_single_value_column() {
        let mut cfg = two_level_config();
        cfg.column_fields = vec![];
        let result = fixture_result(&cfg);
        let cols = flatten_cols(&result, &HashSet::new(), &cfg);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].key, TOTAL_KEY);
    }

    #[test]
    fn flatten_empty_result_is_empty() {
        let cfg = PivotConfig::default();
        let result = PivotResult::default();
        assert!(flatten_rows(&result, &HashSet::new(), &cfg).is_empty());
        assert!(flatten_cols(&result, &HashSet::new(), &cfg).is_empty());
    }

    #[test]
    fn sort_axis_descending_reverses_roots_and_children() {
        let cfg = two_level_config();
        let mut result = fixture_result(&cfg);
        let labels = |result: &PivotResult| -> Vec<String> {
            result
                .row_roots
                .iter()
                .map(|&r| result.row_nodes[r].label.clone())
                .collect()
        };
        assert_eq!(labels(&result), vec!["Asia", "Europe"]);
        let mut roots = result.row_roots.clone();
        sort_axis_by_key(&mut result.row_nodes, &mut roots, true);
        result.row_roots = roots;
        assert_eq!(labels(&result), vec!["Europe", "Asia"]);
        let asia = result.row_roots[1];
        let child_labels: Vec<&str> = result.row_nodes[asia]
            .children
            .iter()
            .map(|&c| result.row_nodes[c].label.as_str())
            .collect();
        assert_eq!(child_labels, vec!["Widget", "Gadget"]);
    }

    #[test]
    fn sort_axis_by_values_orders_missing_first_ascending() {
        let cfg = two_level_config();
        let mut result = fixture_result(&cfg);
        // Sort roots by their 2024 column: Europe=20, Asia=3.
        let y2024 = result
            .col_roots
            .iter()
            .copied()
            .find(|&c| result.col_nodes[c].sort_key == Integer(2024))
            .unwrap();
        let keys: Vec<CellValue> = (0..result.row_nodes.len())
            .map(|id| {
                result
                    .values
                    .get(&(id, y2024))
                    .cloned()
                    .unwrap_or(CellValue::None)
            })
            .collect();
        let mut roots = result.row_roots.clone();
        sort_axis_by(&mut result.row_nodes, &mut roots, &keys, false);
        let labels: Vec<&str> = roots
            .iter()
            .map(|&r| result.row_nodes[r].label.as_str())
            .collect();
        assert_eq!(labels, vec!["Asia", "Europe"]);
        sort_axis_by(&mut result.row_nodes, &mut roots, &keys, true);
        let labels: Vec<&str> = roots
            .iter()
            .map(|&r| result.row_nodes[r].label.as_str())
            .collect();
        assert_eq!(labels, vec!["Europe", "Asia"]);
    }

    #[test]
    fn node_path_walks_to_root() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let europe = result
            .row_nodes
            .iter()
            .position(|n| n.label == "Europe")
            .unwrap();
        let widget = result.row_nodes[europe]
            .children
            .iter()
            .copied()
            .find(|&c| result.row_nodes[c].label == "Widget")
            .unwrap();
        assert_eq!(
            node_path(&result.row_nodes, widget),
            vec!["Europe".to_owned(), "Widget".to_owned()]
        );
    }

    #[test]
    fn all_group_paths_lists_only_non_leaves() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let paths = all_group_paths(&result.row_nodes);
        assert_eq!(paths.len(), 2); // Asia, Europe
        assert!(paths.contains(&vec!["Asia".to_owned()]));
    }

    #[test]
    fn csv_line_quotes_fields_with_commas_and_quotes() {
        let mut out = String::new();
        push_csv_line(
            &mut out,
            &["a,b".to_owned(), "plain".to_owned(), "q\"q".to_owned()],
        );
        assert_eq!(out, "\"a,b\",plain,\"q\"\"q\"\n");
    }

    fn fixture_columns() -> Vec<Column> {
        vec![
            Column::new("region", ColumnKind::Text, 100.0),
            Column::new("product", ColumnKind::Text, 100.0),
            Column::new("year", ColumnKind::Integer, 80.0),
            Column::new("amount", ColumnKind::Decimal, 100.0),
        ]
    }

    fn fixture_rows() -> Vec<Vec<CellValue>> {
        let r = |region: &str, product: &str, year: i64, amount: f64| {
            vec![
                Text(region.into()),
                Text(product.into()),
                Integer(year),
                Decimal(amount),
            ]
        };
        vec![
            r("Europe", "Widget", 2023, 10.0),
            r("Europe", "Widget", 2024, 20.0),
            r("Europe", "Gadget", 2023, 5.0),
            r("Asia", "Widget", 2023, 7.0),
            r("Asia", "Gadget", 2024, 3.0),
        ]
    }

    #[test]
    fn path_components_walk_root_to_leaf_with_field_metadata() {
        let cfg = two_level_config();
        let result = fixture_result(&cfg);
        let columns = fixture_columns();
        let europe = result
            .row_nodes
            .iter()
            .position(|n| n.label == "Europe")
            .unwrap();
        let widget = result.row_nodes[europe]
            .children
            .iter()
            .copied()
            .find(|&c| result.row_nodes[c].label == "Widget")
            .unwrap();
        let path = path_components(
            &result.row_nodes,
            &cfg.row_fields,
            &columns,
            "(blank)",
            widget,
        );
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].field_name, "region");
        assert_eq!(path[0].label, "Europe");
        assert_eq!(path[0].field_index, 0);
        assert_eq!(path[1].field_name, "product");
        assert_eq!(path[1].label, "Widget");
        assert!(!path[1].is_blank);
        // Totals have no path.
        assert!(path_components(
            &result.row_nodes,
            &cfg.row_fields,
            &columns,
            "(blank)",
            TOTAL_KEY
        )
        .is_empty());
    }

    #[test]
    fn rows_matching_path_selects_only_driving_rows() {
        let rows = fixture_rows();
        let columns = fixture_columns();
        let formats = GridConfig::default().resolve_all(&columns);
        let base: Vec<usize> = (0..rows.len()).collect();
        // Europe × Widget → source rows 0 and 1.
        let constraints = vec![(0, "Europe".to_owned()), (1, "Widget".to_owned())];
        assert_eq!(
            rows_matching_path(&rows, &formats, "(blank)", &base, &constraints),
            vec![0, 1]
        );
        // No constraints → everything in the base set.
        assert_eq!(
            rows_matching_path(&rows, &formats, "(blank)", &base, &[]),
            base
        );
        // Constraints respect a pre-filtered base.
        assert_eq!(
            rows_matching_path(&rows, &formats, "(blank)", &[1, 2, 3], &constraints),
            vec![1]
        );
    }

    #[test]
    fn rows_matching_path_buckets_nulls_under_blank_label() {
        let mut rows = fixture_rows();
        rows.push(vec![
            CellValue::None,
            Text("Widget".into()),
            Integer(2023),
            Decimal(2.0),
        ]);
        let columns = fixture_columns();
        let formats = GridConfig::default().resolve_all(&columns);
        let base: Vec<usize> = (0..rows.len()).collect();
        let constraints = vec![(0, "(blank)".to_owned())];
        assert_eq!(
            rows_matching_path(&rows, &formats, "(blank)", &base, &constraints),
            vec![5]
        );
    }

    #[test]
    fn drill_filter_set_maps_blank_to_empty_formatted_value() {
        let set = drill_filter_set("(blank)", "(blank)");
        assert!(set.contains(""));
        assert!(set.contains("(blank)"));
        let set = drill_filter_set("Europe", "(blank)");
        assert_eq!(set, HashSet::from(["Europe".to_owned()]));
    }
}
