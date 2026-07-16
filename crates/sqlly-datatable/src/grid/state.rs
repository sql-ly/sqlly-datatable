//! `GridState` plus all non-paint behaviour: input, scrollbars, drag,
//! sort/filter, scrolling, hit-testing, edge-scroll coordination, filter-prompt
//! cursor handling.

use crate::compare_cells;
use crate::data::{CellValue, ColumnKind, GridData, GridDataError};
use crate::filter::{
    cell_passes_filter, parse_ymd_to_unix, uses_number_ops, ColumnFilter, FilterPredicate,
    NumberOp, TextOp,
};
use crate::format::format_cell;
use crate::grid::state::state_inner::apply_edge_scroll;
use crate::grid::theme::{GridTheme, GridThemePair};

use crate::config::{GridConfig, ResolvedColumnFormat};
use gpui::{
    px, App, Bounds, FocusHandle, Keystroke, MouseButton, Pixels, Point, ScrollHandle, Size,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// Pull selection / menu types into scope unqualified for this module's impl.
use crate::grid::menu as menu_mod;
#[allow(unused_imports)]
pub(crate) use crate::grid::menu::{ContextMenu, MenuAction, MenuItem};
use crate::grid::selection::{
    is_cell_selected, is_row_selected, HitResult, ScrollbarAxis, Selection, SortDirection,
};

use crate::grid::context_menu::{
    ColumnContext, ContextMenuItem, ContextMenuProviderHandle, ContextMenuRequest,
    ContextMenuSelection, ContextMenuTarget, PendingCustomContextMenuAction,
};

/// Inline constructor / state mutators used by the widget's render loop.
/// Kept in its own submodule so this module remains the public surface while
/// its helpers are exposed for unit tests.
pub mod state_inner {
    use super::{
        format_cell, CellValue, GridState, HitResult, Pixels, Point, ResolvedColumnFormat,
    };
    pub use crate::grid::selection::screen_to_content;
    pub use crate::grid::selection::to_grid_relative;
    use std::fmt::Write as _;

    /// Per-tick edge-scroll velocity in pixels (positive scrolls the content
    /// forward; the caller applies sign). Three staged bands spaced 30 px
    /// apart, each a little faster than the last as the pointer approaches the
    /// edge, with a final "really fast" tier inside 30 px. Ticks fire every
    /// [`EDGE_SCROLL_TICK_MS`] (~60 fps), so px/sec ≈ px/tick × 62.5:
    ///
    /// | distance from edge | px/tick |  ~px/sec @ 60fps |
    /// |--------------------|---------|------------------|
    /// | > 90               | 0       | (no scroll)      |
    /// | 60 ..= 90          | 4       | 250              |
    /// | 30 ..= 60          | 8       | 500              |
    /// | < 30               | 16      | 1000 (really fast)|
    /// | < 0 (past edge)    | 16      | (saturate)       |
    const REALLY_FAST: f32 = 16.0;
    pub fn edge_scroll_speed(dist_from_edge: f32) -> f32 {
        if dist_from_edge > 90.0 {
            return 0.0;
        }
        if dist_from_edge < 0.0 {
            // Cursor dragged past the edge: saturate at the really-fast speed
            // so going further out never exceeds the closest in-bounds band.
            return REALLY_FAST;
        }
        if dist_from_edge < 30.0 {
            REALLY_FAST
        } else if dist_from_edge < 60.0 {
            8.0
        } else {
            4.0
        }
    }

    pub fn apply_edge_scroll(state: &mut GridState) -> bool {
        if !state.is_dragging {
            return false;
        }
        let Some(pos) = state.last_mouse_pos else {
            return false;
        };
        let bounds = state.bounds;
        // `pos` (last_mouse_pos) is grid-relative, and the viewport edges are
        // FIXED in that same frame — they don't move when the content scrolls
        // underneath. So distance-from-edge MUST be measured grid-relative.
        // Adding the scroll offset here (as this once did) slides the 90 px
        // trigger bands along with the content: the forward band collapses to
        // zero the moment any scrolling begins (instant max speed, no staged
        // acceleration) and the reverse band grows past 90 px and never
        // fires — so edge-scroll works only before you've scrolled at all.
        let vw: f32 = bounds.size.width.into();
        let vh: f32 = bounds.size.height.into();
        let px: f32 = pos.x.into();
        let py: f32 = pos.y.into();
        let right_dist = vw - px;
        let left_dist = px - state.row_header_width;
        let bottom_dist = vh - py;
        let top_dist = py - state.header_height;
        let mut dx = 0.0_f32;
        let mut dy = 0.0_f32;
        if right_dist < 90.0 && right_dist <= left_dist {
            dx = edge_scroll_speed(right_dist);
        } else if left_dist < 90.0 {
            dx = -edge_scroll_speed(left_dist);
        }
        if bottom_dist < 90.0 && bottom_dist <= top_dist {
            dy = edge_scroll_speed(bottom_dist);
        } else if top_dist < 90.0 {
            dy = -edge_scroll_speed(top_dist);
        }
        if dx == 0.0 && dy == 0.0 {
            return false;
        }
        state.scroll_one_edge_tick(dx, dy);
        if state.drag_start.is_some() {
            state.update_drag_from_last();
        }
        true
    }

    #[must_use]
    pub fn format_current_status(state: &GridState) -> String {
        let scroll = state.scroll_handle.offset();
        let (click_col, click_row) = col_row_from_hit(state.click_hit);
        let (hover_col, hover_row) = col_row_from_hit(state.hover_hit);
        let mut out = String::new();
        let _ = write!(
            out,
            "Click: {}  Scroll@Click: {}  Cell: {}  |  Cur: {}  Scroll: {}  Over: {}",
            fmt_point(state.click_pos),
            fmt_point(state.scroll_at_click),
            fmt_cr(click_col, click_row),
            fmt_point(state.last_mouse_pos),
            fmt_point(Some(scroll)),
            fmt_cr(hover_col, hover_row),
        );
        out
    }

    fn col_row_from_hit(hit: Option<HitResult>) -> (Option<usize>, Option<usize>) {
        match hit {
            Some(HitResult::Cell(r, c)) => (Some(c), Some(r)),
            Some(HitResult::RowHeader(r)) => (None, Some(r)),
            Some(HitResult::ColumnHeader(c)) | Some(HitResult::SortButton(c)) => (Some(c), None),
            _ => (None, None),
        }
    }

    fn fmt_point(p: Option<Point<Pixels>>) -> String {
        match p {
            Some(p) => format!("({:.0}, {:.0})", f32::from(p.x), f32::from(p.y)),
            None => "—".into(),
        }
    }

    fn fmt_cr(c: Option<usize>, r: Option<usize>) -> String {
        match (c, r) {
            (Some(c), Some(r)) => format!("(col {c}, row {r})"),
            (Some(c), None) => format!("(col {c})"),
            (None, Some(r)) => format!("(row {r})"),
            (None, None) => "—".into(),
        }
    }

    #[must_use]
    pub fn cell_text(cell: &CellValue, fmt: &ResolvedColumnFormat) -> String {
        format_cell(cell, fmt).0
    }
}

/// Width, in pixels, of vertical and horizontal scrollbar strips.
pub const SCROLLBAR_SIZE: f32 = 20.0;
/// Polling interval used to drive auto-scroll during drag.
pub const EDGE_SCROLL_TICK_MS: u64 = 16;

/// Read-only description of one section in a grouped flat grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowGroup {
    /// Formatted value shown in the section header.
    pub label: String,
    /// Number of filtered rows in this section, including hidden rows when
    /// the section is collapsed.
    pub row_count: usize,
    /// Whether the section currently hides its rows.
    pub collapsed: bool,
}

/// One visual row in the flat grid's presentation layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridDisplayRow {
    GroupHeader { group: usize },
    Data { source_row: usize, flat_row: usize },
}

/// Windowed-row mode: the grid presents `total_rows` virtual rows (scrollbar,
/// row numbers, hit-testing, selection all speak the virtual index space)
/// while `data.rows` holds only a resident window of them starting at
/// `offset`. The host pages rows in/out with [`GridState::set_row_window`] as
/// the user scrolls, keeping memory O(window) for arbitrarily large sets.
/// Sorting and filtering are disabled while a window is active — the grid
/// only sees a slice of the data, so a resident-only sort would lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowWindow {
    /// Total rows in the virtual set.
    pub total_rows: usize,
    /// Virtual index of `data.rows[0]`.
    pub offset: usize,
}

/// Complete grid state owned by a GPUI `Entity<GridState>`.
#[derive(Debug)]
pub struct GridState {
    pub data: GridData,
    pub config: GridConfig,
    /// When `Some`, the grid is in windowed-row mode (see [`RowWindow`]).
    pub window: Option<RowWindow>,
    /// Cached resolved-format list, kept in sync with `data.columns` and
    /// `config`. Paint, copy, and filter read this directly instead of
    /// recomputing per cell.
    pub resolved_formats: Vec<ResolvedColumnFormat>,
    /// Arc-wrapped row data so `PaintData::from_state` can clone cheaply
    /// (O(1)) instead of deep-cloning every cell every frame. Rows are
    /// immutable after `GridState::new`, so the Arc never needs rebuilding.
    pub(crate) data_rows: Arc<Vec<Vec<CellValue>>>,
    pub display_indices: Arc<Vec<usize>>,
    pub(crate) display_rows: Arc<Vec<GridDisplayRow>>,
    grouped_column: Option<usize>,
    pub(crate) row_groups: Arc<Vec<RowGroup>>,
    collapsed_group_labels: HashSet<String>,
    pub selection: Selection,
    /// Fixed corner of a keyboard/shift range selection (row, col). Set when a
    /// single cell is selected; held steady while shift+arrow moves the active
    /// corner. Mirrors the Swift grid's `ResultGridCellRange.anchor`.
    pub(crate) range_anchor: Option<(usize, usize)>,
    /// Moving corner of a keyboard/shift range selection (row, col). Mirrors
    /// the Swift grid's `ResultGridCellRange.extent`.
    pub(crate) range_active: Option<(usize, usize)>,
    pub sort: Option<(usize, SortDirection)>,
    pub filters: Vec<ColumnFilter>,
    pub scroll_handle: ScrollHandle,
    pub focus_handle: FocusHandle,
    pub bounds: Bounds<Pixels>,
    pub row_height: f32,
    pub header_height: f32,
    pub row_header_width: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub theme: GridTheme,
    /// The light/dark pair used when the widget follows the OS window
    /// appearance. Swap it (e.g. via
    /// [`crate::grid::widget::SqllyDataTable::set_theme_family`]) to change
    /// the shipped family — or supply a custom pair — while keeping
    /// automatic light/dark following.
    pub theme_family: GridThemePair,
    pub is_dragging: bool,
    pub drag_start: Option<Point<Pixels>>,
    pub drag_start_hit: Option<HitResult>,
    pub scroll_at_click: Option<Point<Pixels>>,
    pub last_mouse_pos: Option<Point<Pixels>>,
    pub status_bar_height: f32,
    /// When `true`, the debug status bar is painted at the bottom of the grid
    /// showing click position, scroll offset, and hovered cell. Off by
    /// default; enable via [`SqllyDataTableBuilder::debug_bar`] or
    /// [`GridState::set_debug_bar_enabled`].
    pub debug_bar_enabled: bool,
    pub click_pos: Option<Point<Pixels>>,
    pub click_hit: Option<HitResult>,
    pub hover_hit: Option<HitResult>,
    pub resizing_col: Option<usize>,
    pub resize_start_x: f32,
    pub resize_start_width: f32,
    pub context_menu: Option<ContextMenu>,
    pub filter_panel: Option<FilterPanel>,
    pub pending_action: Option<(MenuAction, usize)>,
    pub(crate) pending_custom_context_menu_action: Option<PendingCustomContextMenuAction>,
    pub(crate) context_menu_provider: Option<ContextMenuProviderHandle>,
    pub scrollbar_drag: Option<ScrollbarAxis>,
    pub scrollbar_drag_start_offset: f32,
    pub scrollbar_drag_start_pos: f32,
    /// Full window viewport size (updated each paint). Used to position the
    /// context menu against the window edges so it is never clipped by the
    /// grid area and flips up only when there is no room below on-screen.
    pub(crate) window_viewport: Size<Pixels>,
    /// `true` while a single edge-scroll timer task is running. Guards against
    /// `render` spawning a new task on every frame/notify during a drag, which
    /// would stack many concurrent 16 ms loops and multiply the scroll speed.
    pub(crate) edge_scroll_active: bool,
    /// Shared, immutable column metadata (index/name/kind) built once in
    /// `new()`. Cloned (O(1)) into every [`ContextMenuRequest`] so building a
    /// right-click request never walks the columns.
    pub(crate) column_meta: Arc<[ColumnContext]>,
    /// Weak handle to this state's own entity, set in
    /// [`SqllyDataTableBuilder::build`]. Lets [`GridState::spawn_background`]
    /// deliver results back to `self` from an async task.
    pub(crate) self_weak: Option<gpui::WeakEntity<GridState>>,
    /// When `Some`, a background task is in progress and the widget paints a
    /// loading overlay. Set by [`GridState::spawn_background`] /
    /// [`GridState::set_busy`]; cleared on completion.
    pub(crate) busy: Option<BusyState>,
}

/// State backing the built-in loading overlay shown while a background task
/// runs. Construct indirectly via [`GridState::spawn_background`] or set
/// directly with [`GridState::set_busy`].
#[derive(Clone, Debug)]
pub struct BusyState {
    /// Text shown in the overlay (e.g. `"Exporting…"`).
    pub label: String,
    /// Optional determinate progress in `0.0..=1.0`. `None` renders an
    /// indeterminate animated bar.
    pub progress: Option<f32>,
}

/// A minimal single-line text input with a **char-based** cursor (not a byte
/// offset), so multi-byte input never panics on a grapheme-misaligned insert.
/// Shared by the filter panel's search box and its operand fields.
#[derive(Clone, Debug, Default)]
pub struct TextInput {
    /// Current text value.
    pub value: String,
    /// Cursor position measured in characters from the start.
    pub cursor_chars: usize,
}

impl TextInput {
    fn new(value: String) -> Self {
        let cursor_chars = value.chars().count();
        Self {
            value,
            cursor_chars,
        }
    }

    fn clamp_cursor(&mut self) {
        let total = self.value.chars().count();
        if self.cursor_chars > total {
            self.cursor_chars = total;
        }
    }

    fn insert_char(&mut self, ch: char) {
        let byte_idx = byte_index_for_char(&self.value, self.cursor_chars);
        self.value.insert(byte_idx, ch);
        self.cursor_chars += 1;
    }

    fn backspace(&mut self) {
        if self.cursor_chars == 0 {
            return;
        }
        let end = byte_index_for_char(&self.value, self.cursor_chars);
        let start = byte_index_for_char(&self.value, self.cursor_chars - 1);
        self.value.replace_range(start..end, "");
        self.cursor_chars -= 1;
    }

    fn move_left(&mut self) {
        if self.cursor_chars > 0 {
            self.cursor_chars -= 1;
        }
    }

    fn move_right(&mut self) {
        self.clamp_cursor();
        if self.cursor_chars < self.value.chars().count() {
            self.cursor_chars += 1;
        }
    }
}

/// Which text field inside the filter panel currently receives typed keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterInput {
    /// The value-list search box.
    Search,
    /// The first operator operand (e.g. "greater than X", "between X …").
    OperandA,
    /// The second operator operand (the upper bound of a range).
    OperandB,
}

/// One row in the filter panel's searchable value checklist.
#[derive(Clone, Debug)]
pub struct FilterValueRow {
    /// The formatted value as displayed in the grid.
    pub label: String,
    /// Whether the value is currently included by the filter.
    pub checked: bool,
}

/// Interactive state backing the Numbers-style per-column filter popover.
///
/// This is the *working* copy that the overlay edits; it is committed to
/// [`GridState::filters`] automatically (auto-apply) as the user interacts
/// with the panel. Rendered as a `deferred` + `anchored` GPUI overlay in
/// `widget.rs`, mirroring the context-menu overlay.
#[derive(Clone, Debug)]
pub struct FilterPanel {
    /// Target column index.
    pub col: usize,
    /// Grid-relative anchor point (from the triggering click).
    pub anchor: Point<Pixels>,
    /// Column kind; selects the text vs. numeric/date operator set.
    pub kind: ColumnKind,
    /// The value-list search box.
    pub search: TextInput,
    /// Selected operator index into [`Self::op_labels`]; `0` == "Choose One"
    /// (no predicate).
    pub op_index: usize,
    /// Whether the operator dropdown is expanded.
    pub op_menu_open: bool,
    /// First operand input.
    pub operand_a: TextInput,
    /// Second operand input (range upper bound).
    pub operand_b: TextInput,
    /// Which text field currently has keyboard focus.
    pub focus: FilterInput,
    /// When set, edits apply to [`GridState::filters`] immediately.
    pub auto_apply: bool,
    /// All distinct formatted values for the column with their checked state.
    pub distinct: Vec<FilterValueRow>,
}

/// Operator labels for text/string-like columns. Index `0` is the inert
/// "Choose One" sentinel; the rest map 1:1 to [`TextOp`] via
/// [`FilterPanel::text_op_for_index`].
const TEXT_OP_LABELS: &[&str] = &[
    "Choose One",
    "contains",
    "does not contain",
    "begins with",
    "ends with",
    "is",
    "is not",
    "matches (regex)",
];

/// Operator labels for numeric/date columns. Index `0` is "Choose One".
const NUMBER_OP_LABELS: &[&str] = &[
    "Choose One",
    "equal to",
    "not equal to",
    "greater than",
    "greater than or equal to",
    "less than",
    "less than or equal to",
    "between",
    "not between",
];

impl FilterPanel {
    /// Operator labels appropriate to this column's kind.
    #[must_use]
    pub fn op_labels(&self) -> &'static [&'static str] {
        if uses_number_ops(self.kind) {
            NUMBER_OP_LABELS
        } else {
            TEXT_OP_LABELS
        }
    }

    /// The currently selected operator label.
    #[must_use]
    pub fn current_op_label(&self) -> &'static str {
        self.op_labels()
            .get(self.op_index)
            .copied()
            .unwrap_or("Choose One")
    }

    /// `true` when the selected operator needs at least one operand.
    #[must_use]
    pub fn needs_operand(&self) -> bool {
        self.op_index != 0
    }

    /// `true` when the selected operator is a range needing a second operand.
    #[must_use]
    pub fn needs_second_operand(&self) -> bool {
        uses_number_ops(self.kind) && matches!(self.op_index, 7 | 8)
    }

    fn text_op_for_index(index: usize) -> Option<TextOp> {
        match index {
            1 => Some(TextOp::Contains),
            2 => Some(TextOp::DoesNotContain),
            3 => Some(TextOp::BeginsWith),
            4 => Some(TextOp::EndsWith),
            5 => Some(TextOp::Is),
            6 => Some(TextOp::IsNot),
            7 => Some(TextOp::Matches),
            _ => None,
        }
    }

    fn number_op_for_index(index: usize) -> Option<NumberOp> {
        match index {
            1 => Some(NumberOp::Eq),
            2 => Some(NumberOp::Ne),
            3 => Some(NumberOp::Gt),
            4 => Some(NumberOp::Ge),
            5 => Some(NumberOp::Lt),
            6 => Some(NumberOp::Le),
            7 => Some(NumberOp::Between),
            8 => Some(NumberOp::NotBetween),
            _ => None,
        }
    }

    fn active_input_mut(&mut self) -> &mut TextInput {
        match self.focus {
            FilterInput::Search => &mut self.search,
            FilterInput::OperandA => &mut self.operand_a,
            FilterInput::OperandB => &mut self.operand_b,
        }
    }

    /// Indices into [`Self::distinct`] whose label matches the current search
    /// box (case-insensitive substring). Drives only which rows are rendered
    /// in the checklist; it does not affect the "(Select All)" state.
    #[must_use]
    pub fn visible_indices(&self) -> Vec<usize> {
        let needle = self.search.value.to_lowercase();
        self.distinct
            .iter()
            .enumerate()
            .filter(|(_, row)| needle.is_empty() || row.label.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// `true` when every distinct value row is checked. Deliberately
    /// independent of the search box: typing in the search only narrows which
    /// rows are *displayed*, it must never change the "(Select All)" state.
    #[must_use]
    pub fn all_checked(&self) -> bool {
        !self.distinct.is_empty() && self.distinct.iter().all(|r| r.checked)
    }

    /// Build the committed [`ColumnFilter`] from the working state. Returns an
    /// inert filter when no predicate is set and all values are checked.
    fn to_filter(&self) -> ColumnFilter {
        let predicate = self.build_predicate();
        let all_checked = self.distinct.iter().all(|r| r.checked);
        let values = if all_checked {
            None
        } else {
            Some(
                self.distinct
                    .iter()
                    .filter(|r| r.checked)
                    .map(|r| r.label.clone())
                    .collect(),
            )
        };
        ColumnFilter { predicate, values }
    }

    fn build_predicate(&self) -> FilterPredicate {
        if self.op_index == 0 {
            return FilterPredicate::None;
        }
        if uses_number_ops(self.kind) {
            let Some(op) = Self::number_op_for_index(self.op_index) else {
                return FilterPredicate::None;
            };
            let Some(a) = self.parse_number_operand(&self.operand_a.value) else {
                return FilterPredicate::None;
            };
            let b = if self.needs_second_operand() {
                self.parse_number_operand(&self.operand_b.value)
                    .unwrap_or(a)
            } else {
                a
            };
            FilterPredicate::Number { op, a, b }
        } else {
            let Some(op) = Self::text_op_for_index(self.op_index) else {
                return FilterPredicate::None;
            };
            FilterPredicate::Text {
                op,
                operand: self.operand_a.value.clone(),
            }
        }
    }

    fn parse_number_operand(&self, s: &str) -> Option<f64> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        if self.kind == ColumnKind::Date {
            return parse_ymd_to_unix(t).map(|v| v as f64);
        }
        // Tolerate thousands separators pasted from the grid's formatted view.
        t.replace(',', "").parse::<f64>().ok()
    }
}

fn byte_index_for_char(input: &str, char_idx: usize) -> usize {
    input
        .char_indices()
        .nth(char_idx)
        .map_or(input.len(), |(idx, _)| idx)
}

/// Derive a panel operator index and its operand strings from an already
/// committed predicate, so reopening a filter shows the same rule.
fn seed_operator(kind: ColumnKind, predicate: &FilterPredicate) -> (usize, String, String) {
    match predicate {
        FilterPredicate::None => (0, String::new(), String::new()),
        FilterPredicate::Text { op, operand } => {
            (text_op_index(*op), operand.clone(), String::new())
        }
        FilterPredicate::Number { op, a, b } => {
            let b_str = if matches!(op, NumberOp::Between | NumberOp::NotBetween) {
                fmt_number_operand(kind, *b)
            } else {
                String::new()
            };
            (number_op_index(*op), fmt_number_operand(kind, *a), b_str)
        }
    }
}

fn text_op_index(op: TextOp) -> usize {
    match op {
        TextOp::Contains => 1,
        TextOp::DoesNotContain => 2,
        TextOp::BeginsWith => 3,
        TextOp::EndsWith => 4,
        TextOp::Is => 5,
        TextOp::IsNot => 6,
        TextOp::Matches => 7,
    }
}

fn number_op_index(op: NumberOp) -> usize {
    match op {
        NumberOp::Eq => 1,
        NumberOp::Ne => 2,
        NumberOp::Gt => 3,
        NumberOp::Ge => 4,
        NumberOp::Lt => 5,
        NumberOp::Le => 6,
        NumberOp::Between => 7,
        NumberOp::NotBetween => 8,
    }
}

fn fmt_number_operand(kind: ColumnKind, v: f64) -> String {
    if kind == ColumnKind::Date {
        let secs = v as i64;
        let fmt = crate::config::DateFormat {
            format: "%Y-%m-%d".into(),
            ..Default::default()
        };
        crate::format::format_date_at(secs, secs, &fmt)
    } else {
        // Display prints `50.0` as `50`, so integer operands stay clean.
        v.to_string()
    }
}

impl GridState {
    #[must_use]
    pub fn new(data: GridData, config: GridConfig, focus_handle: FocusHandle) -> Self {
        let resolved_formats = config.resolve_all(&data.columns);
        let col_count = data.columns.len();
        let display_indices = Arc::new((0..data.rows.len()).collect::<Vec<_>>());
        let display_rows = Arc::new(
            display_indices
                .iter()
                .copied()
                .enumerate()
                .map(|(flat_row, source_row)| GridDisplayRow::Data {
                    source_row,
                    flat_row,
                })
                .collect(),
        );
        let data_rows = Arc::new(data.rows.clone());
        let column_meta: Arc<[ColumnContext]> = data
            .columns
            .iter()
            .enumerate()
            .map(|(index, col)| ColumnContext {
                index,
                name: col.name.clone(),
                kind: col.kind,
            })
            .collect();
        Self {
            data,
            config,
            window: None,
            resolved_formats,
            data_rows,
            display_indices,
            display_rows,
            grouped_column: None,
            row_groups: Arc::new(Vec::new()),
            collapsed_group_labels: HashSet::new(),
            selection: Selection::None,
            range_anchor: None,
            range_active: None,
            sort: None,
            filters: vec![ColumnFilter::default(); col_count],
            scroll_handle: ScrollHandle::new(),
            focus_handle,
            bounds: Bounds::default(),
            row_height: 24.0,
            header_height: 32.0,
            row_header_width: 50.0,
            font_size: 14.0,
            char_width: crate::grid::paint::default_char_width(14.0),
            theme: GridTheme::default(),
            theme_family: GridThemePair::default(),
            is_dragging: false,
            drag_start: None,
            drag_start_hit: None,
            scroll_at_click: None,
            last_mouse_pos: None,
            status_bar_height: 24.0,
            debug_bar_enabled: false,
            click_pos: None,
            click_hit: None,
            hover_hit: None,
            resizing_col: None,
            resize_start_x: 0.0,
            resize_start_width: 0.0,
            context_menu: None,
            filter_panel: None,
            pending_action: None,
            pending_custom_context_menu_action: None,
            context_menu_provider: None,
            scrollbar_drag: None,
            scrollbar_drag_start_offset: 0.0,
            scrollbar_drag_start_pos: 0.0,
            window_viewport: Size::default(),
            edge_scroll_active: false,
            column_meta,
            self_weak: None,
            busy: None,
        }
    }

    pub fn set_config(&mut self, config: GridConfig) {
        self.config = config;
        self.rebuild_resolved_formats();
        self.recompute();
    }

    /// Append rows to the grid in place — the streaming-results fast path.
    ///
    /// Rows are validated against the rectangular invariant, then appended to
    /// the canonical `data.rows`, the paint-path `data_rows` snapshot, and the
    /// display order. With no active sort or per-column filter this is
    /// O(new rows): the fresh indices are pushed onto `display_indices`
    /// directly. When a sort or filter is active, [`GridState::recompute`]
    /// re-derives the full display order so the new rows land in the right
    /// place.
    ///
    /// The `data_rows` Arc is extended via [`Arc::make_mut`]; a clone only
    /// occurs if a paint or context-menu snapshot is still alive, so repeated
    /// appends stay cheap. Selection, scroll position, filters, and sort state
    /// are untouched.
    ///
    /// # Errors
    ///
    /// Returns [`GridDataError::RaggedRow`] (with the would-be absolute row
    /// index) if any incoming row's length differs from the column count; the
    /// grid is left unmodified in that case.
    pub fn append_rows(&mut self, rows: Vec<Vec<CellValue>>) -> Result<(), GridDataError> {
        let expected = self.data.columns.len();
        let base = self.data.rows.len();
        for (offset, row) in rows.iter().enumerate() {
            if row.len() != expected {
                return Err(GridDataError::RaggedRow {
                    row_index: base + offset,
                    expected,
                    actual: row.len(),
                });
            }
        }
        if rows.is_empty() {
            return Ok(());
        }
        Arc::make_mut(&mut self.data_rows).extend(rows.iter().cloned());
        self.data.rows.extend(rows);
        if self.sort.is_some()
            || self.grouped_column.is_some()
            || self.filters.iter().any(ColumnFilter::is_active)
        {
            self.recompute();
        } else {
            Arc::make_mut(&mut self.display_indices).extend(base..self.data.rows.len());
            Arc::make_mut(&mut self.display_rows).extend((base..self.data.rows.len()).map(
                |source_row| GridDisplayRow::Data {
                    source_row,
                    flat_row: source_row,
                },
            ));
        }
        if let Some(window) = &mut self.window {
            window.total_rows += self.data.rows.len() - base;
        }
        Ok(())
    }

    /// Number of rows the grid PRESENTS — the basis for the scrollbar, row
    /// numbers, hit-testing, and selection clamping. Equal to the sort/filter
    /// display order length normally, or the virtual total in windowed mode.
    #[must_use]
    pub fn display_row_count(&self) -> usize {
        self.window
            .map(|w| w.total_rows)
            .unwrap_or(self.display_rows.len())
    }

    /// Maps a display-row index to an index into `data.rows`. Normal mode
    /// goes through the sort/filter display order; windowed mode subtracts
    /// the window offset. `None` when the display row is out of range or not
    /// currently resident (windowed rows that have not been paged in).
    #[must_use]
    pub fn resident_row_for_display(&self, display_row: usize) -> Option<usize> {
        match self.window {
            Some(w) => display_row
                .checked_sub(w.offset)
                .filter(|r| *r < self.data.rows.len() && display_row < w.total_rows),
            None => match self.display_rows.get(display_row) {
                Some(GridDisplayRow::Data { source_row, .. }) => Some(*source_row),
                _ => None,
            },
        }
    }

    /// Column currently used to group the flat grid, or `None` when rows are
    /// displayed without section headers.
    #[must_use]
    pub fn grouped_column(&self) -> Option<usize> {
        self.grouped_column
    }

    /// Group the flat grid by `column`, or clear grouping with `None`.
    /// Invalid column indices and grouping requests in windowed-row mode are
    /// ignored because only a resident slice is available there.
    pub fn set_grouped_column(&mut self, column: Option<usize>) {
        if self.window.is_some() && column.is_some() {
            return;
        }
        if let Some(column) = column {
            if column >= self.data.columns.len() {
                return;
            }
        }
        if self.grouped_column != column {
            self.grouped_column = column;
            self.collapsed_group_labels.clear();
            self.selection = Selection::None;
            self.range_anchor = None;
            self.range_active = None;
        }
        self.rebuild_display_rows();
        self.clamp_scroll_to_bounds();
    }

    /// Current grouped sections in display order.
    #[must_use]
    pub fn row_groups(&self) -> &[RowGroup] {
        &self.row_groups
    }

    /// Expand or collapse a grouped section by its current display index.
    pub fn set_group_collapsed(&mut self, group: usize, collapsed: bool) {
        let Some(label) = self.row_groups.get(group).map(|group| group.label.clone()) else {
            return;
        };
        if collapsed {
            self.collapsed_group_labels.insert(label);
        } else {
            self.collapsed_group_labels.remove(&label);
        }
        self.selection = Selection::None;
        self.range_anchor = None;
        self.range_active = None;
        self.clear_drag();
        self.rebuild_display_rows();
        self.clamp_scroll_to_bounds();
    }

    /// Toggle a grouped section by its current display index.
    pub fn toggle_group(&mut self, group: usize) {
        if let Some(section) = self.row_groups.get(group) {
            self.set_group_collapsed(group, !section.collapsed);
        }
    }

    /// Enter (or update) windowed-row mode: the grid presents `total_rows`
    /// virtual rows while holding only `rows` in memory, positioned so that
    /// `rows[0]` is virtual row `offset`. Replaces the resident rows, the
    /// paint snapshot, and the display order in one step; selection and
    /// scroll position (both in virtual space) are untouched. Clears any
    /// active sort/filter — they are unsupported while windowed.
    ///
    /// # Errors
    ///
    /// Returns [`GridDataError::RaggedRow`] if any incoming row's length
    /// differs from the column count; the grid is left unmodified.
    pub fn set_row_window(
        &mut self,
        total_rows: usize,
        offset: usize,
        rows: Vec<Vec<CellValue>>,
    ) -> Result<(), GridDataError> {
        let expected = self.data.columns.len();
        for (i, row) in rows.iter().enumerate() {
            if row.len() != expected {
                return Err(GridDataError::RaggedRow {
                    row_index: offset + i,
                    expected,
                    actual: row.len(),
                });
            }
        }
        self.sort = None;
        self.grouped_column = None;
        self.row_groups = Arc::new(Vec::new());
        self.collapsed_group_labels.clear();
        for filter in &mut self.filters {
            *filter = ColumnFilter::default();
        }
        self.data_rows = Arc::new(rows.clone());
        self.display_indices = Arc::new((0..rows.len()).collect());
        self.data.rows = rows;
        self.window = Some(RowWindow { total_rows, offset });
        self.rebuild_display_rows();
        Ok(())
    }

    /// The half-open display-row range currently visible in the viewport,
    /// derived from the scroll offset and painted bounds — the same math the
    /// paint pass uses. Hosts drive window paging from this: when the range
    /// nears the resident window's edges, page more rows in via
    /// [`GridState::set_row_window`]. Returns `(0, 0)` before first paint.
    #[must_use]
    pub fn visible_row_range(&self) -> (usize, usize) {
        let total = self.display_row_count();
        let sy: f32 = self.scroll_handle.offset().y.into();
        let vh: f32 = self.bounds.size.height.into();
        let visible_h = vh - self.header_height;
        if visible_h <= 0.0 || self.row_height <= 0.0 {
            return (0, 0);
        }
        let first = ((sy / self.row_height) as usize).min(total);
        let last = (first + (visible_h / self.row_height) as usize + 1).min(total);
        (first, last)
    }

    /// Enable or disable the debug status bar at runtime. When enabled, a bar
    /// is painted at the bottom of the grid showing click position, scroll
    /// offset, and hovered cell coordinates.
    pub fn set_debug_bar_enabled(&mut self, enabled: bool) {
        self.debug_bar_enabled = enabled;
    }

    /// Whether a background task is currently running (the loading overlay is
    /// shown).
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// The current busy state, if any.
    #[must_use]
    pub fn busy(&self) -> Option<&BusyState> {
        self.busy.as_ref()
    }

    /// Show the loading overlay with the given label and indeterminate
    /// progress. Call [`GridState::clear_busy`] to hide it. For work that
    /// should run off the UI thread, prefer [`GridState::spawn_background`],
    /// which manages this automatically.
    pub fn set_busy(&mut self, label: impl Into<String>) {
        self.busy = Some(BusyState {
            label: label.into(),
            progress: None,
        });
    }

    /// Update the determinate progress (`0.0..=1.0`) of the current busy
    /// state. No-op if not busy.
    pub fn set_busy_progress(&mut self, progress: f32) {
        if let Some(b) = self.busy.as_mut() {
            b.progress = Some(progress.clamp(0.0, 1.0));
        }
    }

    /// Hide the loading overlay.
    pub fn clear_busy(&mut self) {
        self.busy = None;
    }

    /// Run `work` on a background thread, showing the loading overlay labelled
    /// `label` for the duration, then deliver the result back on the UI thread
    /// via `on_done` (which receives `&mut GridState` and `&mut App`).
    ///
    /// This is the recommended way to do expensive work triggered from a
    /// context-menu action (e.g. building an export from a large selection):
    /// the right-click stays instant, the work does not block the UI, and a
    /// loading indicator is shown until it completes.
    ///
    /// `work` and its result `R` must be `Send + 'static`; `on_done` runs on
    /// the UI thread and need not be `Send`. A cloned [`ContextMenuRequest`]
    /// can be moved into `work` (it is `Send + Sync + 'static`).
    ///
    /// If this state has no entity handle yet (constructed via
    /// [`GridState::new`] directly, e.g. in tests rather than through the
    /// builder), `work` runs synchronously as a fallback.
    pub fn spawn_background<R, W, D>(
        &mut self,
        cx: &mut App,
        label: impl Into<String>,
        work: W,
        on_done: D,
    ) where
        R: Send + 'static,
        W: FnOnce() -> R + Send + 'static,
        D: FnOnce(R, &mut GridState, &mut App) + 'static,
    {
        let Some(weak) = self.self_weak.clone() else {
            // No entity handle: run synchronously so the callback still fires.
            let result = work();
            on_done(result, self, cx);
            return;
        };

        self.busy = Some(BusyState {
            label: label.into(),
            progress: None,
        });

        let background = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            // Paint the overlay before starting the heavy work.
            let _ = cx.update(|app| {
                let _ = weak.update(app, |_s, c| c.notify());
            });
            let result = background.spawn(async move { work() }).await;
            let _ = cx.update(|app| {
                let _ = weak.update(app, |s, c| {
                    s.busy = None;
                    on_done(result, s, c);
                    c.notify();
                });
            });
        })
        .detach();
    }

    fn rebuild_resolved_formats(&mut self) {
        self.resolved_formats = self.config.resolve_all(&self.data.columns);
    }

    pub fn recompute(&mut self) {
        // Windowed-row mode: sort/filter are unsupported, the display order
        // is always the identity over the resident window.
        if self.window.is_some() {
            self.display_indices = Arc::new((0..self.data.rows.len()).collect());
            self.rebuild_display_rows();
            return;
        }
        let mut indices: Vec<usize> = (0..self.data.rows.len())
            .filter(|&row_idx| {
                self.data.columns.iter().enumerate().all(|(col_idx, _col)| {
                    let filter = &self.filters[col_idx];
                    if !filter.is_active() {
                        return true;
                    }
                    let cell = &self.data.rows[row_idx][col_idx];
                    cell_passes_filter(cell, &self.resolved_formats[col_idx], filter)
                })
            })
            .collect();

        if let Some((sort_col, direction)) = self.sort {
            indices.sort_by(|&a, &b| {
                let cell_a = &self.data.rows[a][sort_col];
                let cell_b = &self.data.rows[b][sort_col];
                let ord = compare_cells(cell_a, cell_b);
                match direction {
                    SortDirection::Ascending => ord,
                    SortDirection::Descending => ord.reverse(),
                }
            });
        }
        self.display_indices = Arc::new(indices);
        if self.grouped_column.is_some() {
            self.selection = Selection::None;
            self.range_anchor = None;
            self.range_active = None;
            self.clear_drag();
        }
        self.rebuild_display_rows();
    }

    fn rebuild_display_rows(&mut self) {
        let Some(group_col) = self.grouped_column.filter(|_| self.window.is_none()) else {
            self.row_groups = Arc::new(Vec::new());
            self.display_rows = Arc::new(
                self.display_indices
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(flat_row, source_row)| GridDisplayRow::Data {
                        source_row,
                        flat_row,
                    })
                    .collect(),
            );
            return;
        };

        let mut group_positions = HashMap::<String, usize>::new();
        let mut grouped_rows = Vec::<(String, Vec<(usize, usize)>)>::new();
        for (flat_row, &source_row) in self.display_indices.iter().enumerate() {
            let (label, _) = format_cell(
                &self.data.rows[source_row][group_col],
                &self.resolved_formats[group_col],
            );
            let group = *group_positions.entry(label.clone()).or_insert_with(|| {
                let index = grouped_rows.len();
                grouped_rows.push((label, Vec::new()));
                index
            });
            grouped_rows[group].1.push((source_row, flat_row));
        }

        self.collapsed_group_labels
            .retain(|label| group_positions.contains_key(label));
        let groups: Vec<RowGroup> = grouped_rows
            .iter()
            .map(|(label, rows)| RowGroup {
                label: label.clone(),
                row_count: rows.len(),
                collapsed: self.collapsed_group_labels.contains(label),
            })
            .collect();
        let mut display_rows = Vec::with_capacity(
            groups.len()
                + groups
                    .iter()
                    .filter(|group| !group.collapsed)
                    .map(|group| group.row_count)
                    .sum::<usize>(),
        );
        for (group, (_, rows)) in grouped_rows.into_iter().enumerate() {
            display_rows.push(GridDisplayRow::GroupHeader { group });
            if !groups[group].collapsed {
                display_rows.extend(rows.into_iter().map(|(source_row, flat_row)| {
                    GridDisplayRow::Data {
                        source_row,
                        flat_row,
                    }
                }));
            }
        }
        self.row_groups = Arc::new(groups);
        self.display_rows = Arc::new(display_rows);
    }

    fn content_size(&self) -> (f32, f32) {
        let cw: f32 = self.data.columns.iter().map(|c| c.width).sum();
        let ch = self.display_row_count() as f32 * self.row_height;
        (cw, ch)
    }

    pub(crate) fn max_scroll(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let (rw, rh) = self.scrollbar_reserved();
        let vw: f32 = self.bounds.size.width.into();
        let vh: f32 = self.bounds.size.height.into();
        let vw = vw - self.row_header_width - rw;
        let vh = vh - self.header_height - rh;
        ((cw - vw).max(0.0), (ch - vh).max(0.0))
    }

    /// Re-clamp the scroll offset after the grid's layout bounds change
    /// (e.g. the host resizes the area allocated to the grid). Without this
    /// the offset can sit beyond the new maximum until the next scroll event,
    /// leaving the painted rows and scrollbar geometry stale. Called only
    /// when bounds actually change, so it adds no per-frame cost.
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

    fn scrollbar_reserved(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let vw: f32 = self.bounds.size.width.into();
        let vh: f32 = self.bounds.size.height.into();
        let vw = vw - self.row_header_width;
        let vh = vh - self.header_height;
        let reserved_w = if ch > vh { SCROLLBAR_SIZE } else { 0.0 };
        let reserved_h = if cw > vw { SCROLLBAR_SIZE } else { 0.0 };
        (reserved_w, reserved_h)
    }

    fn vbar_geom(&self) -> Option<(f32, f32, f32, f32, f32)> {
        let (_, ch) = self.content_size();
        let (_, rh) = self.scrollbar_reserved();
        let vh: f32 = self.bounds.size.height.into();
        let vh = vh - self.header_height - rh;
        if ch <= vh {
            return None;
        }
        // Grid-relative track geometry (matches the grid-relative mouse coords
        // passed to `scroll_to_vbar`).
        let sw: f32 = self.bounds.size.width.into();
        let sh: f32 = self.bounds.size.height.into();
        let track_x = sw - SCROLLBAR_SIZE;
        let track_y = self.header_height;
        let track_h = sh - self.header_height - rh;
        let thumb_h = ((track_h * (vh / ch)).max(20.0)).min(track_h);
        Some((track_x, track_y, SCROLLBAR_SIZE, track_h, thumb_h))
    }

    fn hbar_geom(&self) -> Option<(f32, f32, f32, f32, f32)> {
        let (cw, _) = self.content_size();
        let (rw, _) = self.scrollbar_reserved();
        let vw: f32 = self.bounds.size.width.into();
        let vw = vw - self.row_header_width - rw;
        if cw <= vw {
            return None;
        }
        // Grid-relative track geometry (matches the grid-relative mouse coords
        // passed to `scroll_to_hbar`).
        let sw: f32 = self.bounds.size.width.into();
        let sh: f32 = self.bounds.size.height.into();
        let track_x = self.row_header_width;
        let track_y = sh - SCROLLBAR_SIZE;
        let track_w = sw - self.row_header_width - rw;
        let thumb_w = ((track_w * (vw / cw)).max(20.0)).min(track_w);
        Some((track_x, track_y, track_w, SCROLLBAR_SIZE, thumb_w))
    }

    pub(crate) fn scroll_to_vbar(&mut self, mouse_y: f32) {
        if let Some((_, track_y, _, track_h, thumb_h)) = self.vbar_geom() {
            let (_, max_y) = self.max_scroll();
            let range = (track_h - thumb_h).max(0.0);
            let rel = (mouse_y - track_y - thumb_h * 0.5).clamp(0.0, range);
            let frac = if range > 0.0 { rel / range } else { 0.0 };
            let new_y = frac * max_y;
            let x = self.scroll_handle.offset().x;
            self.scroll_handle.set_offset(Point { x, y: px(new_y) });
        }
    }

    pub(crate) fn scroll_to_hbar(&mut self, mouse_x: f32) {
        if let Some((track_x, _, track_w, _, thumb_w)) = self.hbar_geom() {
            let (max_x, _) = self.max_scroll();
            let range = (track_w - thumb_w).max(0.0);
            let rel = (mouse_x - track_x - thumb_w * 0.5).clamp(0.0, range);
            let frac = if range > 0.0 { rel / range } else { 0.0 };
            let new_x = frac * max_x;
            let y = self.scroll_handle.offset().y;
            self.scroll_handle.set_offset(Point { x: px(new_x), y });
        }
    }

    pub(crate) fn scroll_one_edge_tick(&mut self, dx: f32, dy: f32) {
        let (mx, my) = self.max_scroll();
        let s = self.scroll_handle.offset();
        let new_x: f32 = (f32::from(s.x) + dx).clamp(0.0, mx);
        let new_y: f32 = (f32::from(s.y) + dy).clamp(0.0, my);
        self.scroll_handle.set_offset(Point {
            x: px(new_x),
            y: px(new_y),
        });
    }

    pub fn toggle_sort(&mut self, col: usize) {
        // Sorting is unsupported in windowed-row mode — only a slice of the
        // set is resident, so a resident-only sort would present wrong data.
        if self.window.is_some() {
            return;
        }
        self.sort = match self.sort {
            Some((c, SortDirection::Ascending)) if c == col => {
                Some((col, SortDirection::Descending))
            }
            Some((c, SortDirection::Descending)) if c == col => None,
            _ => Some((col, SortDirection::Ascending)),
        };
        self.recompute();
    }

    pub fn handle_mouse_down(&mut self, pos: Point<Pixels>, shift: bool) {
        self.handle_mouse_down_with_modifiers(pos, shift, false);
    }

    pub fn handle_mouse_down_with_modifiers(&mut self, pos: Point<Pixels>, shift: bool, cmd: bool) {
        let hit = self.hit_test(pos);
        self.click_pos = Some(pos);
        self.click_hit = Some(hit);
        match hit {
            HitResult::VerticalScrollbar => {
                self.scrollbar_drag = Some(ScrollbarAxis::Vertical);
                self.scroll_to_vbar(f32::from(pos.y));
                self.clear_drag();
            }
            HitResult::HorizontalScrollbar => {
                self.scrollbar_drag = Some(ScrollbarAxis::Horizontal);
                self.scroll_to_hbar(f32::from(pos.x));
                self.clear_drag();
            }
            HitResult::ColumnBorder(col) => {
                self.resizing_col = Some(col);
                self.resize_start_x = f32::from(pos.x);
                self.resize_start_width = self.data.columns[col].width;
                self.clear_drag();
            }
            HitResult::ColumnHeader(col) => {
                if cmd {
                    // Cmd-click toggles the column in/out of the current
                    // column selection set.
                    let mut cols: Vec<usize> = match &self.selection {
                        Selection::Column(c) => vec![*c],
                        Selection::Columns(cs) => cs.clone(),
                        _ => Vec::new(),
                    };
                    if let Some(idx) = cols.iter().position(|&c| c == col) {
                        cols.remove(idx);
                    } else {
                        cols.push(col);
                        cols.sort_unstable();
                    }
                    self.selection = match cols.len() {
                        0 => Selection::None,
                        1 => Selection::Column(cols[0]),
                        _ => Selection::Columns(cols),
                    };
                    self.clear_drag();
                } else {
                    self.selection = Selection::Column(col);
                    // Dragging across headers extends to a column range.
                    self.start_drag(pos);
                    self.drag_start_hit = Some(HitResult::ColumnHeader(col));
                }
            }
            HitResult::SortButton(col) => {
                // Clicking the sort button only toggles sort; it must not
                // change the current selection (the column is not selected).
                self.toggle_sort(col);
                self.clear_drag();
            }
            HitResult::ContextMenuItem(_) => {}
            HitResult::GroupHeader(group) => {
                self.toggle_group(group);
                self.clear_drag();
            }
            HitResult::RowHeader(row) => {
                if cmd {
                    // Cmd-click toggles the row in/out of the current row
                    // selection set.
                    let mut rows: Vec<usize> = match &self.selection {
                        Selection::Row(r) => vec![*r],
                        Selection::RowRange(r1, r2) => (*r1.min(r2)..=*r1.max(r2)).collect(),
                        Selection::Rows(rs) => rs.clone(),
                        _ => Vec::new(),
                    };
                    if let Some(idx) = rows.iter().position(|&r| r == row) {
                        rows.remove(idx);
                    } else {
                        rows.push(row);
                        rows.sort_unstable();
                    }
                    self.selection = match rows.len() {
                        0 => Selection::None,
                        1 => Selection::Row(rows[0]),
                        _ => Selection::Rows(rows),
                    };
                    self.clear_drag();
                    return;
                }
                self.selection = if shift {
                    if let Selection::Row(prev) = self.selection {
                        let (s, e) = (prev, row);
                        Selection::RowRange(s.min(e), s.max(e))
                    } else {
                        Selection::Row(row)
                    }
                } else {
                    Selection::Row(row)
                };
                self.start_drag(pos);
                self.drag_start_hit = Some(HitResult::RowHeader(row));
            }
            HitResult::Cell(row, col) => {
                if cmd {
                    // Cmd-click toggles the individual cell in/out of the
                    // current cell selection set.
                    let mut cells: Vec<(usize, usize)> = match &self.selection {
                        Selection::Cell(r, c) => vec![(*r, *c)],
                        Selection::Cells(cs) => cs.clone(),
                        _ => Vec::new(),
                    };
                    if let Some(idx) = cells.iter().position(|&rc| rc == (row, col)) {
                        cells.remove(idx);
                    } else {
                        cells.push((row, col));
                        cells.sort_unstable();
                    }
                    self.selection = match cells.len() {
                        0 => Selection::None,
                        1 => Selection::Cell(cells[0].0, cells[0].1),
                        _ => Selection::Cells(cells),
                    };
                    self.range_anchor = None;
                    self.range_active = None;
                    self.clear_drag();
                    return;
                }
                self.selection = if shift {
                    // Extend from the existing anchor (Swift: anchor/extent).
                    let anchor = self
                        .range_anchor
                        .or(match self.selection {
                            Selection::Cell(pr, pc) => Some((pr, pc)),
                            _ => None,
                        })
                        .unwrap_or((row, col));
                    self.range_anchor = Some(anchor);
                    self.range_active = Some((row, col));
                    Selection::CellRange(
                        anchor.0.min(row),
                        anchor.1.min(col),
                        anchor.0.max(row),
                        anchor.1.max(col),
                    )
                } else {
                    self.range_anchor = Some((row, col));
                    self.range_active = Some((row, col));
                    Selection::Cell(row, col)
                };
                self.start_drag(pos);
                self.drag_start_hit = Some(HitResult::Cell(row, col));
            }
            HitResult::Corner | HitResult::None => {
                self.selection = Selection::None;
                self.range_anchor = None;
                self.range_active = None;
                self.context_menu = None;
                self.filter_panel = None;
                self.clear_drag();
            }
        }
    }

    fn start_drag(&mut self, pos: Point<Pixels>) {
        self.is_dragging = false;
        self.drag_start = Some(pos);
        self.scroll_at_click = Some(self.scroll_handle.offset());
        self.last_mouse_pos = Some(pos);
    }

    pub(crate) fn open_context_menu(&mut self, col: usize, anchor: Point<Pixels>) {
        self.context_menu = Some(menu_mod::ContextMenu::standard(col, anchor));
        self.filter_panel = None;
    }

    /// Convert a hit-test result to a context-menu target. Returns `None`
    /// for hits that don't map to a meaningful right-click target.
    pub(crate) fn context_menu_target_from_hit(&self, hit: HitResult) -> Option<ContextMenuTarget> {
        match hit {
            HitResult::Cell(row, col) => {
                let source_row = self.resident_row_for_display(row).unwrap_or(row);
                Some(ContextMenuTarget::Cell {
                    display_row_index: row,
                    source_row_index: source_row,
                    column_index: col,
                })
            }
            HitResult::RowHeader(row) => {
                let source_row = self.resident_row_for_display(row).unwrap_or(row);
                Some(ContextMenuTarget::RowHeader {
                    display_row_index: row,
                    source_row_index: source_row,
                })
            }
            HitResult::ColumnHeader(col) => {
                Some(ContextMenuTarget::ColumnHeader { column_index: col })
            }
            HitResult::SortButton(col) => Some(ContextMenuTarget::SortButton { column_index: col }),
            _ => None,
        }
    }

    /// Compute the effective selection for a context-menu target. If the
    /// target is inside the current selection, the selection is preserved.
    /// If outside, the selection collapses to the target. Column-header
    /// targets do not change selection.
    pub(crate) fn effective_selection_for_context_target(
        &self,
        target: &ContextMenuTarget,
    ) -> Selection {
        match target {
            ContextMenuTarget::Cell {
                display_row_index,
                column_index,
                ..
            } => {
                if is_cell_selected(&self.selection, *display_row_index, *column_index) {
                    self.selection.clone()
                } else {
                    Selection::Cell(*display_row_index, *column_index)
                }
            }
            ContextMenuTarget::RowHeader {
                display_row_index, ..
            } => {
                if is_row_selected(&self.selection, *display_row_index) {
                    self.selection.clone()
                } else {
                    Selection::Row(*display_row_index)
                }
            }
            ContextMenuTarget::ColumnHeader { .. } | ContextMenuTarget::SortButton { .. } => {
                self.selection.clone()
            }
        }
    }

    /// Build a **lazy** snapshot of the right-click context. Construction is
    /// O(1): it clamps the selection bounds and clones three shared [`Arc`]
    /// handles (row data, display order, column metadata). No per-cell or
    /// per-row data is cloned here, so right-clicking a huge selection is
    /// instant; the owned snapshots are materialized on demand by
    /// [`ContextMenuRequest`]'s accessors (ideally off the UI thread via
    /// [`GridState::spawn_background`]).
    ///
    /// For column-oriented targets (`ColumnHeader`, `SortButton`, or an
    /// explicit `Selection::Column`), the request is flagged column-oriented so
    /// its row accessors stay empty (`clicked_row()` is `None`).
    pub(crate) fn build_context_menu_request(
        &self,
        target: ContextMenuTarget,
        selection: &Selection,
    ) -> ContextMenuRequest {
        // Windowed-row mode: the request's row data and display order cover
        // only the RESIDENT window, so translate the virtual display rows in
        // the target and selection into resident space (clamped to the
        // window). Right-clicks land on visible — hence resident — rows, so
        // this is lossless for the clicked cell; a selection reaching beyond
        // the window is clamped to its resident part.
        let mut target = target;
        let mut selection = selection.clone();
        if let Some(w) = self.window {
            let resident_last = self.data.rows.len().saturating_sub(1);
            let to_resident = |dr: usize| dr.saturating_sub(w.offset).min(resident_last);
            target = match target {
                ContextMenuTarget::Cell {
                    display_row_index,
                    source_row_index,
                    column_index,
                } => ContextMenuTarget::Cell {
                    display_row_index: to_resident(display_row_index),
                    source_row_index,
                    column_index,
                },
                ContextMenuTarget::RowHeader {
                    display_row_index,
                    source_row_index,
                } => ContextMenuTarget::RowHeader {
                    display_row_index: to_resident(display_row_index),
                    source_row_index,
                },
                other => other,
            };
            selection = match selection {
                Selection::Cell(r, c) => Selection::Cell(to_resident(r), c),
                Selection::Row(r) => Selection::Row(to_resident(r)),
                Selection::CellRange(r1, c1, r2, c2) => {
                    Selection::CellRange(to_resident(r1), c1, to_resident(r2), c2)
                }
                other => other,
            };
        }

        let request_display_indices = if self.grouped_column.is_some() && self.window.is_none() {
            let mut row_map = vec![None; self.display_rows.len()];
            let mut indices = Vec::new();
            for (display_row, row) in self.display_rows.iter().enumerate() {
                if let GridDisplayRow::Data { source_row, .. } = row {
                    row_map[display_row] = Some(indices.len());
                    indices.push(*source_row);
                }
            }
            let map_row = |display_row: usize| {
                row_map
                    .get(display_row)
                    .copied()
                    .flatten()
                    .or_else(|| {
                        row_map
                            .iter()
                            .skip(display_row.saturating_add(1))
                            .flatten()
                            .copied()
                            .next()
                    })
                    .or_else(|| {
                        row_map
                            .iter()
                            .take(display_row)
                            .rev()
                            .flatten()
                            .copied()
                            .next()
                    })
                    .unwrap_or(0)
            };
            target = match target {
                ContextMenuTarget::Cell {
                    display_row_index,
                    source_row_index,
                    column_index,
                } => ContextMenuTarget::Cell {
                    display_row_index: map_row(display_row_index),
                    source_row_index,
                    column_index,
                },
                ContextMenuTarget::RowHeader {
                    display_row_index,
                    source_row_index,
                } => ContextMenuTarget::RowHeader {
                    display_row_index: map_row(display_row_index),
                    source_row_index,
                },
                other => other,
            };
            selection = match selection {
                Selection::Cell(row, col) => Selection::Cell(map_row(row), col),
                Selection::Row(row) => Selection::Row(map_row(row)),
                Selection::CellRange(r1, c1, r2, c2) => {
                    Selection::CellRange(map_row(r1), c1, map_row(r2), c2)
                }
                Selection::RowRange(r1, r2) => Selection::RowRange(map_row(r1), map_row(r2)),
                Selection::Rows(rows) => Selection::Rows(
                    rows.into_iter()
                        .filter_map(|row| row_map.get(row).copied().flatten())
                        .collect(),
                ),
                Selection::Cells(cells) => Selection::Cells(
                    cells
                        .into_iter()
                        .filter_map(|(row, col)| {
                            row_map.get(row).copied().flatten().map(|row| (row, col))
                        })
                        .collect(),
                ),
                other => other,
            };
            Arc::new(indices)
        } else {
            Arc::clone(&self.display_indices)
        };
        let selection = &selection;

        let nrows = request_display_indices.len();
        let ncols = self.data.columns.len();

        let (r1, c1, r2, c2) = match selection.normalized_bounds() {
            Some((r1, c1, r2, c2)) => {
                let r1 = r1.min(nrows.saturating_sub(1));
                let r2 = r2.min(nrows.saturating_sub(1));
                let c1 = c1.min(ncols.saturating_sub(1));
                let c2 = c2.min(ncols.saturating_sub(1));
                (r1, c1, r2, c2)
            }
            None => match &target {
                ContextMenuTarget::Cell {
                    display_row_index,
                    column_index,
                    ..
                } => (
                    *display_row_index,
                    *column_index,
                    *display_row_index,
                    *column_index,
                ),
                ContextMenuTarget::RowHeader {
                    display_row_index, ..
                } => (
                    *display_row_index,
                    0,
                    *display_row_index,
                    ncols.saturating_sub(1),
                ),
                ContextMenuTarget::ColumnHeader { column_index }
                | ContextMenuTarget::SortButton { column_index } => {
                    (0, *column_index, nrows.saturating_sub(1), *column_index)
                }
            },
        };

        let menu_selection = ContextMenuSelection {
            row_start: r1,
            row_end: r2,
            column_start: c1,
            column_end: c2,
        };

        // A column-oriented right-click (column header, sort button, or an
        // explicit whole-column selection) selects cells within one column,
        // not whole rows. `clicked_row()` is always `None` for these targets,
        // so the request's row accessors stay empty.
        let column_oriented =
            matches!(
                target,
                ContextMenuTarget::ColumnHeader { .. } | ContextMenuTarget::SortButton { .. }
            ) || matches!(selection, Selection::Column(_) | Selection::Columns(_));

        ContextMenuRequest::new(
            target,
            Some(menu_selection),
            Arc::clone(&self.data_rows),
            request_display_indices,
            Arc::clone(&self.column_meta),
            column_oriented,
        )
    }

    /// Execute a deferred custom context-menu action by invoking the
    /// provider. The provider handle is cloned before the call to avoid
    /// `&mut self` borrow conflicts.
    pub(crate) fn execute_custom_context_menu_action(
        &mut self,
        pending: PendingCustomContextMenuAction,
        cx: &mut App,
    ) {
        self.context_menu = None;
        self.filter_panel = None;

        let Some(provider) = self.context_menu_provider.clone() else {
            return;
        };

        provider.on_action(&pending.id, &pending.request, self, cx);
    }

    /// Convert public [`ContextMenuItem`]s to internal `MenuItem`s for the
    /// rendering pipeline.
    pub(crate) fn convert_context_menu_items(items: Vec<ContextMenuItem>) -> Vec<MenuItem> {
        items
            .into_iter()
            .map(|item| match item {
                ContextMenuItem::BuiltIn(action) => MenuItem::Action(action),
                ContextMenuItem::Action { id, label } => MenuItem::Custom { id, label },
                ContextMenuItem::Separator => MenuItem::Separator,
            })
            .collect()
    }

    pub fn execute_action(&mut self, action: MenuAction, col: usize, cx: &mut App) {
        match action {
            MenuAction::SelectColumn => {
                self.selection = Selection::Column(col);
            }
            MenuAction::CopyColumn => {
                let text = self.column_text(col);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            MenuAction::CopyColumnWithHeaders => {
                let mut text = String::new();
                text.push_str(&self.data.columns[col].name);
                text.push('\n');
                text.push_str(&self.column_text(col));
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            MenuAction::SortAscending => {
                self.sort = Some((col, SortDirection::Ascending));
                self.recompute();
            }
            MenuAction::SortDescending => {
                self.sort = Some((col, SortDirection::Descending));
                self.recompute();
            }
            MenuAction::ClearSort => {
                self.sort = None;
                self.recompute();
            }
            MenuAction::GroupBy => self.set_grouped_column(Some(col)),
            MenuAction::ClearGrouping => self.set_grouped_column(None),
            MenuAction::FilterPrompt => {
                let anchor = self.context_menu.as_ref().map(|m| m.anchor);
                self.open_filter_panel(col, anchor);
            }
            MenuAction::ClearFilter => {
                if col < self.filters.len() {
                    self.filters[col] = ColumnFilter::default();
                    self.recompute();
                }
            }
        }
        self.context_menu = None;
    }

    /// Open the rich per-column filter popover for `col`, seeding its working
    /// state from any filter already committed on that column. The overlay is
    /// rendered by `widget.rs` as a `deferred` + `anchored` element so it can
    /// paint and receive events outside the grid's own layout bounds, exactly
    /// like the right-click context menu.
    ///
    /// `anchor` overrides the panel's spawn position; pass the original
    /// context-menu / header right-click position so the panel doesn't jump to
    /// the mouse's current location (which by now has moved to the menu item).
    /// Falls back to `last_mouse_pos` when `None`.
    pub fn open_filter_panel(&mut self, col: usize, _anchor: Option<Point<Pixels>>) {
        if col >= self.data.columns.len() {
            return;
        }
        let sx = f32::from(self.scroll_handle.offset().x);
        let col_x = self.row_header_width
            + self.data.columns[..col]
                .iter()
                .map(|c| c.width)
                .sum::<f32>()
            - sx;
        let anchor = Point {
            x: px(col_x + self.data.columns[col].width * 0.5),
            y: px(0.0),
        };
        let kind = self.data.columns[col].kind;
        let existing = self.filters.get(col).cloned().unwrap_or_default();

        // Distinct formatted values in natural cell order, deduped by label.
        let distinct = {
            let fmt = &self.resolved_formats[col];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut pairs: Vec<(String, &CellValue)> = Vec::new();
            for row in &self.data.rows {
                let cell = &row[col];
                let (label, _) = format_cell(cell, fmt);
                if seen.insert(label.clone()) {
                    pairs.push((label, cell));
                }
            }
            pairs.sort_by(|(_, a), (_, b)| compare_cells(a, b));
            pairs
                .into_iter()
                .map(|(label, _)| {
                    let checked = match &existing.values {
                        None => true,
                        Some(set) => set.contains(&label),
                    };
                    FilterValueRow { label, checked }
                })
                .collect()
        };

        let (op_index, operand_a, operand_b) = seed_operator(kind, &existing.predicate);

        self.context_menu = None;
        self.filter_panel = Some(FilterPanel {
            col,
            anchor,
            kind,
            search: TextInput::default(),
            op_index,
            op_menu_open: false,
            operand_a: TextInput::new(operand_a),
            operand_b: TextInput::new(operand_b),
            focus: FilterInput::Search,
            auto_apply: true,
            distinct,
        });
    }

    /// Commit the panel's working state to [`Self::filters`] and re-filter.
    /// Called automatically on every interaction (auto-apply).
    pub fn apply_filter_panel(&mut self) {
        let Some(panel) = &self.filter_panel else {
            return;
        };
        let col = panel.col;
        let filter = panel.to_filter();
        if col < self.filters.len() {
            self.filters[col] = filter;
            self.recompute();
        }
    }

    /// Apply immediately — the panel always auto-applies.
    pub fn maybe_auto_apply(&mut self) {
        if self.filter_panel.is_some() {
            self.apply_filter_panel();
        }
    }

    /// Reset both the committed filter for the panel's column and the panel's
    /// working state (all values checked, no operator), then re-filter.
    pub fn clear_filter_panel(&mut self) {
        let mut target_col = None;
        if let Some(panel) = &mut self.filter_panel {
            panel.op_index = 0;
            panel.op_menu_open = false;
            panel.operand_a = TextInput::default();
            panel.operand_b = TextInput::default();
            panel.search = TextInput::default();
            for row in &mut panel.distinct {
                row.checked = true;
            }
            target_col = Some(panel.col);
        }
        if let Some(col) = target_col {
            if col < self.filters.len() {
                self.filters[col] = ColumnFilter::default();
            }
        }
        self.recompute();
    }

    /// Set the sort direction on the panel's column (the panel's Sort buttons).
    /// Clicking the already-active direction turns the sort off.
    pub fn set_panel_sort(&mut self, direction: SortDirection) {
        if let Some(panel) = &self.filter_panel {
            let col = panel.col;
            self.sort = match self.sort {
                Some((c, d)) if c == col && d == direction => None,
                _ => Some((col, direction)),
            };
            self.recompute();
        }
    }

    /// Toggle the checked state of a single distinct value row (by index into
    /// [`FilterPanel::distinct`]), then auto-apply if enabled.
    pub fn toggle_filter_value(&mut self, index: usize) {
        if let Some(panel) = &mut self.filter_panel {
            if let Some(row) = panel.distinct.get_mut(index) {
                row.checked = !row.checked;
            }
        }
        self.maybe_auto_apply();
    }

    /// Toggle every distinct value row at once, then auto-apply if enabled.
    /// Mirrors the "(Select All)" checkbox. Operates on all values regardless
    /// of the active search, so searching never changes what "(Select All)"
    /// does.
    pub fn toggle_filter_select_all(&mut self) {
        if let Some(panel) = &mut self.filter_panel {
            let target = !panel.all_checked();
            for row in &mut panel.distinct {
                row.checked = target;
            }
        }
        self.maybe_auto_apply();
    }

    /// Select an operator by its index in [`FilterPanel::op_labels`], close the
    /// dropdown, and auto-apply if enabled.
    pub fn set_filter_operator(&mut self, op_index: usize) {
        if let Some(panel) = &mut self.filter_panel {
            panel.op_index = op_index;
            panel.op_menu_open = false;
            if op_index != 0 {
                panel.focus = FilterInput::OperandA;
            }
        }
        self.maybe_auto_apply();
    }

    /// Toggle the operator dropdown's expanded state.
    pub fn toggle_filter_op_menu(&mut self) {
        if let Some(panel) = &mut self.filter_panel {
            panel.op_menu_open = !panel.op_menu_open;
        }
    }

    /// Point keyboard focus at one of the panel's text fields.
    pub fn set_filter_focus(&mut self, focus: FilterInput) {
        if let Some(panel) = &mut self.filter_panel {
            panel.focus = focus;
        }
    }

    /// Toggle the panel's auto-apply flag; kept for API completeness.
    pub fn toggle_filter_auto_apply(&mut self) {
        if let Some(panel) = &mut self.filter_panel {
            panel.auto_apply = !panel.auto_apply;
        }
        self.maybe_auto_apply();
    }

    fn column_text(&self, col: usize) -> String {
        let mut text = String::new();
        let fmt = &self.resolved_formats[col];
        for &row_idx in self.display_indices.iter() {
            let cell = &self.data.rows[row_idx][col];
            let (s, _) = format_cell(cell, fmt);
            text.push_str(&s);
            text.push('\n');
        }
        text
    }

    fn clear_drag(&mut self) {
        self.is_dragging = false;
        self.drag_start = None;
        self.drag_start_hit = None;
        self.scroll_at_click = None;
    }

    fn drag_world_corners(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let start = self.drag_start?;
        let mouse = self.last_mouse_pos?;
        let click_scroll = self
            .scroll_at_click
            .unwrap_or_else(|| self.scroll_handle.offset());
        let scroll = self.scroll_handle.offset();
        let sx_click: f32 = click_scroll.x.into();
        let sy_click: f32 = click_scroll.y.into();
        let sx: f32 = scroll.x.into();
        let sy: f32 = scroll.y.into();
        let sx0: f32 = start.x.into();
        let sy0: f32 = start.y.into();
        let mx: f32 = mouse.x.into();
        let my: f32 = mouse.y.into();
        let start_world = Point {
            x: px(sx0 + sx_click),
            y: px(sy0 + sy_click),
        };
        let end_world = Point {
            x: px(mx + sx),
            y: px(my + sy),
        };
        Some((start_world, end_world))
    }

    pub fn drag_screen_rect(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        if !self.is_dragging {
            return None;
        }
        let (start_world, end_world) = self.drag_world_corners()?;
        let scroll = self.scroll_handle.offset();
        let sx: f32 = scroll.x.into();
        let sy: f32 = scroll.y.into();
        let start_screen = Point {
            x: px(f32::from(start_world.x) - sx),
            y: px(f32::from(start_world.y) - sy),
        };
        let end_screen = Point {
            x: px(f32::from(end_world.x) - sx),
            y: px(f32::from(end_world.y) - sy),
        };
        Some((start_screen, end_screen))
    }

    fn update_drag(&mut self) {
        let (start_world, end_world) = match self.drag_world_corners() {
            Some(c) => c,
            None => return,
        };
        if !self.is_dragging {
            let dx = f32::from(end_world.x) - f32::from(start_world.x);
            let dy = f32::from(end_world.y) - f32::from(start_world.y);
            if dx * dx + dy * dy <= 400.0 {
                return;
            }
            self.is_dragging = true;
        }
        let r1 = match self.drag_start_hit {
            Some(h) => h,
            None => return,
        };
        // `end_world` is already grid-relative + scroll (content space), since
        // `drag_start`/`last_mouse_pos` are stored grid-relative. Feed it
        // straight into content hit-testing with a zero scroll delta.
        let r2 = self.hit_test_content(f32::from(end_world.x), f32::from(end_world.y), 0.0, 0.0);
        match (r1, r2) {
            (HitResult::Cell(r1c, c1), HitResult::Cell(r2c, c2)) => {
                self.selection =
                    Selection::CellRange(r1c.min(r2c), c1.min(c2), r1c.max(r2c), c1.max(c2));
            }
            (HitResult::RowHeader(r1r), HitResult::RowHeader(r2r)) => {
                self.selection = Selection::RowRange(r1r.min(r2r), r1r.max(r2r));
            }
            (
                HitResult::ColumnHeader(c1),
                HitResult::ColumnHeader(c2)
                | HitResult::SortButton(c2)
                | HitResult::ColumnBorder(c2),
            ) => {
                self.selection = if c1 == c2 {
                    Selection::Column(c1)
                } else {
                    Selection::Columns((c1.min(c2)..=c1.max(c2)).collect())
                };
            }
            _ => {}
        }
    }

    fn update_drag_from_last(&mut self) {
        self.update_drag();
    }

    pub fn handle_mouse_move(&mut self, pos: Point<Pixels>, pressed_button: Option<MouseButton>) {
        if self.is_dragging && pressed_button != Some(MouseButton::Left) {
            self.handle_mouse_up();
            return;
        }
        if let Some(col) = self.resizing_col {
            if pressed_button != Some(MouseButton::Left) {
                self.resizing_col = None;
                return;
            }
            let new_w =
                (self.resize_start_width + (f32::from(pos.x) - self.resize_start_x)).max(40.0);
            self.data.columns[col].width = new_w;
            return;
        }
        if let Some(axis) = self.scrollbar_drag {
            if pressed_button != Some(MouseButton::Left) {
                self.scrollbar_drag = None;
                return;
            }
            match axis {
                ScrollbarAxis::Vertical => self.scroll_to_vbar(f32::from(pos.y)),
                ScrollbarAxis::Horizontal => self.scroll_to_hbar(f32::from(pos.x)),
            }
            self.last_mouse_pos = Some(pos);
            return;
        }
        self.last_mouse_pos = Some(pos);
        if self.context_menu.is_some() {
            // A menu is open. Hover highlighting is driven by the deferred
            // overlay's per-item `on_mouse_move` handlers (widget.rs), which
            // work even when the pointer is outside the grid's layout bounds.
            // Don't run grid hit-testing or drag logic underneath the menu.
            return;
        }
        self.hover_hit = Some(self.hit_test(pos));
        if self.drag_start.is_none() {
            return;
        }
        self.update_drag();
    }

    pub fn handle_scroll_drag(&mut self) {
        if self.drag_start.is_some() && self.last_mouse_pos.is_some() {
            self.update_drag();
        }
    }

    pub fn handle_mouse_up(&mut self) {
        self.resizing_col = None;
        self.scrollbar_drag = None;
        self.clear_drag();
    }

    pub fn apply_edge_scroll(&mut self) -> bool {
        apply_edge_scroll(self)
    }

    pub fn select_all(&mut self) {
        let nrows = self.display_row_count();
        let ncols = self.data.columns.len();
        if nrows > 0 && ncols > 0 {
            self.selection = Selection::CellRange(0, 0, nrows - 1, ncols - 1);
        }
    }

    /// Display-row indices of fully selected rows, sorted ascending and
    /// clamped to the current row count. Empty when the selection is not
    /// row-oriented.
    #[must_use]
    pub fn selected_rows(&self) -> Vec<usize> {
        let nrows = self.display_row_count();
        let rows = match &self.selection {
            Selection::Row(r) if *r < nrows => vec![*r],
            Selection::RowRange(r1, r2) => {
                (*r1.min(r2)..=*r1.max(r2)).filter(|&r| r < nrows).collect()
            }
            Selection::Rows(rows) => rows.iter().copied().filter(|&r| r < nrows).collect(),
            _ => Vec::new(),
        };
        rows.into_iter()
            .filter(|&row| self.is_data_display_row(row))
            .collect()
    }

    /// Column indices of fully selected columns, sorted ascending and clamped
    /// to the current column count. Empty when the selection is not
    /// column-oriented.
    #[must_use]
    pub fn selected_columns(&self) -> Vec<usize> {
        let ncols = self.data.columns.len();
        match &self.selection {
            Selection::Column(c) if *c < ncols => vec![*c],
            Selection::Columns(cols) => cols.iter().copied().filter(|&c| c < ncols).collect(),
            _ => Vec::new(),
        }
    }

    /// Every selected `(display_row, column)` cell, expanded from whatever
    /// the current selection variant is (single cell, ranges, whole rows or
    /// columns, or discontiguous cmd-click sets), clamped to the current data
    /// dimensions. Row-major order.
    #[must_use]
    pub fn selected_cells(&self) -> Vec<(usize, usize)> {
        let nrows = self.display_row_count();
        let ncols = self.data.columns.len();
        if nrows == 0 || ncols == 0 {
            return Vec::new();
        }
        match &self.selection {
            Selection::None => Vec::new(),
            Selection::Cell(r, c) if *r < nrows && *c < ncols && self.is_data_display_row(*r) => {
                vec![(*r, *c)]
            }
            Selection::Cell(..) => Vec::new(),
            Selection::Cells(cells) => cells
                .iter()
                .copied()
                .filter(|&(r, c)| r < nrows && c < ncols && self.is_data_display_row(r))
                .collect(),
            Selection::Column(_) | Selection::Columns(_) => {
                let cols = self.selected_columns();
                (0..nrows)
                    .filter(|&r| self.is_data_display_row(r))
                    .flat_map(|r| cols.iter().map(move |&c| (r, c)))
                    .collect()
            }
            Selection::Row(_) | Selection::RowRange(..) | Selection::Rows(_) => self
                .selected_rows()
                .into_iter()
                .flat_map(|r| (0..ncols).map(move |c| (r, c)))
                .collect(),
            Selection::CellRange(r1, c1, r2, c2) => {
                let (rmin, rmax) = (*r1.min(r2), *r1.max(r2));
                let (cmin, cmax) = (*c1.min(c2), *c1.max(c2));
                (rmin..=rmax.min(nrows.saturating_sub(1)))
                    .filter(|&r| self.is_data_display_row(r))
                    .flat_map(|r| (cmin..=cmax.min(ncols.saturating_sub(1))).map(move |c| (r, c)))
                    .collect()
            }
        }
    }

    pub fn copy_selection(&self, with_headers: bool, cx: &mut App) {
        let Some((raw_r1, raw_c1, raw_r2, raw_c2)) = self.selection.normalized_bounds() else {
            return;
        };
        if self.display_row_count() == 0 || self.data.columns.is_empty() {
            return;
        }
        let last_row = self.display_row_count() - 1;
        let last_col = self.data.columns.len() - 1;
        let r1 = raw_r1.min(last_row);
        let r2 = raw_r2.min(last_row);
        let c1 = raw_c1.min(last_col);
        let c2 = raw_c2.min(last_col);
        let mut text = String::new();
        if with_headers {
            for c in c1..=c2 {
                if c > c1 {
                    text.push('\t');
                }
                text.push_str(&self.data.columns[c].name);
            }
            text.push('\n');
        }
        for dr in r1..=r2 {
            let Some(row_idx) = self.resident_row_for_display(dr) else {
                continue;
            };
            for c in c1..=c2 {
                if c > c1 {
                    text.push('\t');
                }
                let cell = &self.data.rows[row_idx][c];
                let (s, _) = format_cell(cell, &self.resolved_formats[c]);
                text.push_str(&s);
            }
            text.push('\n');
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    pub fn page_up(&mut self) {
        let vh: f32 = self.bounds.size.height.into();
        let rows = ((vh - self.header_height) / self.row_height) as i32;
        self.move_selection(0, -rows);
    }

    pub fn page_down(&mut self) {
        let vh: f32 = self.bounds.size.height.into();
        let rows = ((vh - self.header_height) / self.row_height) as i32;
        self.move_selection(0, rows);
    }

    pub fn handle_key(&mut self, keystroke: &Keystroke) {
        if self.filter_panel.is_some() {
            match keystroke.key.as_str() {
                "escape" => {
                    self.filter_panel = None;
                    return;
                }
                "enter" => {
                    self.apply_filter_panel();
                    return;
                }
                _ => {}
            }
            let mut edited = false;
            if let Some(panel) = &mut self.filter_panel {
                let input = panel.active_input_mut();
                match keystroke.key.as_str() {
                    "backspace" => {
                        input.backspace();
                        edited = true;
                    }
                    "left" => input.move_left(),
                    "right" => input.move_right(),
                    _ => {
                        if let Some(ch) = keystroke_to_char(keystroke) {
                            input.insert_char(ch);
                            edited = true;
                        }
                    }
                }
            }
            // Typing into an operand re-applies live (search only narrows the
            // rendered checklist, so re-applying is a harmless no-op there).
            if edited {
                self.maybe_auto_apply();
            }
            return;
        }
        if self.context_menu.is_some() {
            if keystroke.key.as_str() == "escape" {
                self.context_menu = None;
            }
            return;
        }
        let shift = keystroke.modifiers.shift;
        match keystroke.key.as_str() {
            "up" if shift => self.extend_selection(0, -1),
            "down" if shift => self.extend_selection(0, 1),
            "left" if shift => self.extend_selection(-1, 0),
            "right" if shift => self.extend_selection(1, 0),
            "up" => self.move_selection(0, -1),
            "down" => self.move_selection(0, 1),
            "left" => self.move_selection(-1, 0),
            "right" => self.move_selection(1, 0),
            "escape" => {
                self.selection = Selection::None;
                self.range_anchor = None;
                self.range_active = None;
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, dx: i32, dy: i32) {
        let nrows = self.display_row_count() as i32;
        let ncols = self.data.columns.len() as i32;
        if nrows == 0 || ncols == 0 {
            return;
        }
        let last_col = ncols - 1;
        match self.selection {
            Selection::Cell(row, col) => {
                let nr = self.move_data_row(row, dy);
                let nc = (col as i32 + dx).clamp(0, last_col) as usize;
                self.selection = Selection::Cell(nr, nc);
                self.range_anchor = Some((nr, nc));
                self.range_active = Some((nr, nc));
            }
            Selection::Row(row) if dy != 0 => {
                let nr = self.move_data_row(row, dy);
                self.selection = Selection::Row(nr);
            }
            Selection::Column(col) if dx != 0 => {
                let nc = (col as i32 + dx).clamp(0, last_col) as usize;
                self.selection = Selection::Column(nc);
            }
            _ => {
                if let Some(row) = self.first_data_row() {
                    self.selection = Selection::Cell(row, 0);
                    self.range_anchor = Some((row, 0));
                    self.range_active = Some((row, 0));
                }
            }
        }
    }

    fn first_data_row(&self) -> Option<usize> {
        if self.window.is_some() {
            return (self.display_row_count() > 0).then_some(0);
        }
        self.display_rows
            .iter()
            .position(|row| matches!(row, GridDisplayRow::Data { .. }))
    }

    fn is_data_display_row(&self, row: usize) -> bool {
        if let Some(window) = self.window {
            return row < window.total_rows;
        }
        matches!(
            self.display_rows.get(row),
            Some(GridDisplayRow::Data { .. })
        )
    }

    fn move_data_row(&self, row: usize, delta: i32) -> usize {
        if delta == 0 || self.grouped_column.is_none() || self.window.is_some() {
            let last = self.display_row_count().saturating_sub(1) as i32;
            return (row as i32 + delta).clamp(0, last) as usize;
        }

        let direction = delta.signum();
        let mut current = row as i32;
        let mut remaining = delta.unsigned_abs();
        let last = self.display_row_count().saturating_sub(1) as i32;
        while remaining > 0 {
            let mut candidate = current;
            loop {
                let next = candidate + direction;
                if next < 0 || next > last {
                    return current as usize;
                }
                candidate = next;
                if matches!(
                    self.display_rows.get(candidate as usize),
                    Some(GridDisplayRow::Data { .. })
                ) {
                    break;
                }
            }
            current = candidate;
            remaining -= 1;
        }
        current as usize
    }

    /// Extend a rectangular cell selection by moving the active corner while
    /// holding the anchor corner fixed (shift+arrow). Mirrors the Swift grid's
    /// anchor/extent range model. Row and column selections are left unchanged.
    fn extend_selection(&mut self, dx: i32, dy: i32) {
        let nrows = self.display_row_count() as i32;
        let ncols = self.data.columns.len() as i32;
        if nrows == 0 || ncols == 0 {
            return;
        }
        let last_col = ncols - 1;

        // Seed anchor/active from the current selection when not already set.
        if self.range_anchor.is_none() || self.range_active.is_none() {
            match self.selection {
                Selection::Cell(r, c) => {
                    self.range_anchor = Some((r, c));
                    self.range_active = Some((r, c));
                }
                Selection::CellRange(r1, c1, r2, c2) => {
                    self.range_anchor = Some((r1, c1));
                    self.range_active = Some((r2, c2));
                }
                _ => {
                    let Some(row) = self.first_data_row() else {
                        return;
                    };
                    self.range_anchor = Some((row, 0));
                    self.range_active = Some((row, 0));
                    self.selection = Selection::Cell(row, 0);
                }
            }
        }

        let anchor = self.range_anchor.unwrap_or((0, 0));
        let active = self.range_active.unwrap_or(anchor);
        let nr = self.move_data_row(active.0, dy);
        let nc = (active.1 as i32 + dx).clamp(0, last_col) as usize;
        self.range_active = Some((nr, nc));

        self.selection = if (nr, nc) == anchor {
            Selection::Cell(nr, nc)
        } else {
            Selection::CellRange(
                anchor.0.min(nr),
                anchor.1.min(nc),
                anchor.0.max(nr),
                anchor.1.max(nc),
            )
        };
    }

    pub(crate) fn hit_test(&self, pos: Point<Pixels>) -> HitResult {
        let bounds = self.bounds;
        let (sx, sy) = (
            f32::from(self.scroll_handle.offset().x),
            f32::from(self.scroll_handle.offset().y),
        );
        let bw: f32 = bounds.size.width.into();
        let bh: f32 = bounds.size.height.into();
        let (mx, my) = self.max_scroll();
        if let Some(menu) = &self.context_menu {
            let cw = self.char_width;
            // `pos` is grid-relative and the menu anchor is stored
            // grid-relative, so compare directly — no origin, no scroll.
            let x_rel = f32::from(pos.x);
            let y_rel = f32::from(pos.y);
            if let Some(idx) = menu_mod::hover_at(menu, x_rel, y_rel, cw) {
                return HitResult::ContextMenuItem(idx);
            }
        }
        if my > 0.0
            && f32::from(pos.x) >= bw - SCROLLBAR_SIZE
            && f32::from(pos.y) >= self.header_height
        {
            return HitResult::VerticalScrollbar;
        }
        if mx > 0.0
            && f32::from(pos.y) >= bh - SCROLLBAR_SIZE
            && f32::from(pos.x) >= self.row_header_width
        {
            return HitResult::HorizontalScrollbar;
        }
        // `pos` is grid-relative. `hit_test_content` folds the scroll offset in
        // itself for each scrolling region, so pass `pos` directly — NOT
        // content-space coordinates, which would double-apply the offset and
        // also break the fixed header-region checks (`y < header_height`,
        // `x < row_header_width`) that are evaluated in grid-relative space.
        let px = f32::from(pos.x);
        let py = f32::from(pos.y);
        if px < 0.0 || py < 0.0 || px > bw || py > bh {
            return HitResult::None;
        }
        self.hit_test_content(px, py, sx, sy)
    }

    fn hit_test_content(&self, x: f32, y: f32, sx: f32, sy: f32) -> HitResult {
        if y < self.header_height {
            if x < self.row_header_width {
                return HitResult::Corner;
            }
            let col_x = x - self.row_header_width + sx;
            let mut acc = 0.0;
            for (i, col) in self.data.columns.iter().enumerate() {
                let right = acc + col.width;
                if i + 1 < self.data.columns.len() && col_x >= right - 5.0 && col_x <= right + 5.0 {
                    return HitResult::ColumnBorder(i);
                }
                if col_x >= acc && col_x < right {
                    if col_x >= right - 20.0 {
                        return HitResult::SortButton(i);
                    }
                    return HitResult::ColumnHeader(i);
                }
                acc = right;
            }
            return HitResult::None;
        }
        if x < self.row_header_width {
            let row_y = y - self.header_height + sy;
            if row_y < 0.0 {
                return HitResult::None;
            }
            let row_idx = (row_y / self.row_height) as usize;
            if row_idx < self.display_row_count() {
                if let Some(GridDisplayRow::GroupHeader { group }) = self.display_rows.get(row_idx)
                {
                    return HitResult::GroupHeader(*group);
                }
                return HitResult::RowHeader(row_idx);
            }
            return HitResult::None;
        }
        let col_x = x - self.row_header_width + sx;
        let row_y = y - self.header_height + sy;
        if row_y < 0.0 {
            return HitResult::None;
        }
        let row_idx = (row_y / self.row_height) as usize;
        if row_idx >= self.display_row_count() {
            return HitResult::None;
        }
        if let Some(GridDisplayRow::GroupHeader { group }) = self.display_rows.get(row_idx) {
            return HitResult::GroupHeader(*group);
        }
        let mut acc = 0.0;
        for (i, col) in self.data.columns.iter().enumerate() {
            if col_x >= acc && col_x < acc + col.width {
                return HitResult::Cell(row_idx, i);
            }
            acc += col.width;
        }
        HitResult::None
    }

    #[must_use]
    pub fn wants_edge_scroll_tick(&self) -> bool {
        self.is_dragging
    }
}

fn keystroke_to_char(k: &Keystroke) -> Option<char> {
    if k.modifiers.control || k.modifiers.platform || k.modifiers.alt {
        return None;
    }
    if let Some(key_char) = k.key_char.as_ref() {
        return key_char.chars().next();
    }
    if k.key.chars().count() == 1 {
        let c = k.key.chars().next()?;
        if k.modifiers.shift {
            Some(c.to_ascii_uppercase())
        } else {
            Some(c)
        }
    } else {
        None
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use crate::data::{CellValue, Column, ColumnKind};
    use crate::grid::state::state_inner::{edge_scroll_speed, format_current_status};

    fn input_with(text: &str, cursor: usize) -> TextInput {
        let mut p = TextInput::new(text.to_owned());
        p.cursor_chars = cursor;
        p
    }

    #[test]
    fn text_input_new_cursors_at_char_count_not_bytes() {
        // "hé🙂" is 3 chars but 7 bytes (h=1, é=2, 🙂=4).
        let p = TextInput::new("hé🙂".into());
        assert_eq!(p.cursor_chars, 3);
        assert_eq!(p.value.len(), 7);
    }

    #[test]
    fn text_input_insert_emoji_at_start_does_not_panic() {
        let mut p = input_with("ab", 0);
        p.insert_char('\u{1F600}');
        assert_eq!(p.value, "\u{1F600}ab");
        assert_eq!(p.cursor_chars, 1);
    }

    #[test]
    fn text_input_insert_in_middle_keeps_cursor_at_char_position() {
        let mut p = input_with("helloworld", 5);
        p.insert_char(' ');
        assert_eq!(p.value, "hello world");
        assert_eq!(p.cursor_chars, 6);
    }

    #[test]
    fn text_input_backspace_at_zero_is_noop() {
        let mut p = input_with("abc", 0);
        p.backspace();
        assert_eq!(p.value, "abc");
        assert_eq!(p.cursor_chars, 0);
    }

    #[test]
    fn text_input_backspace_removes_one_char_value() {
        // Cursor sits after "hé" (2 chars); backspace should delete "é" only.
        let mut p = input_with("héx", 2);
        p.backspace();
        assert_eq!(p.value, "hx");
        assert_eq!(p.cursor_chars, 1);
    }

    #[test]
    fn text_input_clamp_cursor_pulls_back_past_end() {
        let mut p = input_with("abc", 99);
        p.clamp_cursor();
        assert_eq!(p.cursor_chars, 3);
    }

    #[test]
    fn text_input_move_left_and_right_respect_bounds() {
        let mut p = input_with("ab", 2);
        p.move_right();
        assert_eq!(p.cursor_chars, 2);
        p.move_left();
        p.move_left();
        p.move_left();
        assert_eq!(p.cursor_chars, 0);
    }

    #[test]
    fn edge_scroll_speed_stops_outside_band() {
        // Outside the 90 px trigger band: no scroll.
        assert_eq!(edge_scroll_speed(120.0), 0.0);
        assert_eq!(edge_scroll_speed(90.01), 0.0);
        // 60 ..= 90 -> 4 px/tick (slowest band).
        assert_eq!(edge_scroll_speed(90.0), 4.0);
        assert_eq!(edge_scroll_speed(60.0), 4.0);
        assert_eq!(edge_scroll_speed(59.99), 8.0);
        // 30 ..= 60 -> 8 px/tick.
        assert_eq!(edge_scroll_speed(30.0), 8.0);
        assert_eq!(edge_scroll_speed(29.99), 16.0);
        // < 30 -> 16 px/tick (really fast).
        assert_eq!(edge_scroll_speed(0.0), 16.0);
        assert_eq!(edge_scroll_speed(29.99), 16.0);
    }

    #[test]
    fn edge_scroll_speed_caps_negative_runaway() {
        // Past the edge: saturate at the really-fast speed (16), not higher.
        assert_eq!(edge_scroll_speed(-100.0), 16.0);
        assert_eq!(edge_scroll_speed(-1000.0), 16.0);
    }

    /// `GridState` requires a real GPUI `FocusHandle` from
    /// `gpui::Application`, but `gpui::Application::new()` panics on any
    /// thread other than `main`. Since Rust's test runner executes on a
    /// worker pool, the GPUI-backed assertions cannot run alongside pure
    /// tests. We mark this test `#[ignore]` so `cargo test` stays green; run
    /// it with `cargo test -- --ignored grid_state_behavior_under_application`
    /// from the workspace root on the test thread observable to GPUI.
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn grid_state_behavior_under_application() {
        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();

            // format_current_status_handles_initial_state
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("n", ColumnKind::Integer, 100.0)],
                    vec![vec![CellValue::Integer(1)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            let _ = format_current_status(&state);
            assert_eq!(state.selection, Selection::None);

            // format_current_status_replaces_with_supplied_pos
            state.last_mouse_pos = Some(Point {
                x: px(120.0),
                y: px(80.0),
            });
            let s = format_current_status(&state);
            assert!(s.contains("(120, 80)"), "missing positional, got: {s}");

            // recompute_filters_then_sorts_then_clears
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("name", ColumnKind::Text, 100.0)],
                    vec![
                        vec![CellValue::Text("alpha".into())],
                        vec![CellValue::Text("beeb".into())],
                        vec![CellValue::Text("gamma".into())],
                    ],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            state.filters[0] = ColumnFilter {
                predicate: FilterPredicate::Text {
                    op: TextOp::Contains,
                    operand: "a".into(),
                },
                values: None,
            };
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices.as_slice(), &[0, 2]);
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices.as_slice(), &[2, 0]);
            state.filters[0] = ColumnFilter::default();
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices.as_slice(), &[0, 1, 2]);

            // toggle_sort_cycles_through_three_states
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("v", ColumnKind::Integer, 80.0)],
                    vec![vec![CellValue::Integer(1)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            state.toggle_sort(0);
            assert_eq!(state.sort, Some((0, SortDirection::Ascending)));
            state.toggle_sort(0);
            assert_eq!(state.sort, Some((0, SortDirection::Descending)));
            state.toggle_sort(0);
            assert_eq!(state.sort, None);

            // select_all_picks_full_range_when_data_present
            let mut state = GridState::new(
                GridData::new(
                    vec![
                        Column::new("a", ColumnKind::Integer, 80.0),
                        Column::new("b", ColumnKind::Integer, 80.0),
                    ],
                    vec![vec![CellValue::Integer(1), CellValue::Integer(2)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            state.select_all();
            assert_eq!(state.selection, Selection::CellRange(0, 0, 0, 1));

            // select_all_is_noop_on_empty
            let mut state = GridState::new(
                GridData::new(vec![Column::new("a", ColumnKind::Integer, 80.0)], vec![])
                    .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            state.select_all();
            assert_eq!(state.selection, Selection::None);

            // set_config_refreshes_resolved_formats
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("v", ColumnKind::Decimal, 100.0)],
                    vec![vec![CellValue::Decimal(1.234)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            assert_eq!(state.resolved_formats[0].number.decimals, 2);
            let mut cfg = crate::config::GridConfig::default();
            cfg.column_overrides = vec![crate::config::ColumnOverride {
                number: Some(crate::config::NumberFormat {
                    decimals: 6,
                    ..Default::default()
                }),
                ..Default::default()
            }];
            state.set_config(cfg);
            assert_eq!(state.resolved_formats[0].number.decimals, 6);

            // wants_edge_scroll_tick_mirrors_is_dragging
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("a", ColumnKind::Integer, 80.0)],
                    vec![vec![CellValue::Integer(1)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            assert!(!state.wants_edge_scroll_tick());
            state.is_dragging = true;
            assert!(state.wants_edge_scroll_tick());

            cx.quit();
        });
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn context_menu_request_construction() {
        use crate::grid::context_menu::ContextMenuTarget;

        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();

            // 3 rows, 2 columns. Sort descending so display_indices != source.
            let mut state = GridState::new(
                GridData::new(
                    vec![
                        Column::new("id", ColumnKind::Integer, 80.0),
                        Column::new("name", ColumnKind::Text, 100.0),
                    ],
                    vec![
                        vec![CellValue::Integer(1), CellValue::Text("alpha".into())],
                        vec![CellValue::Integer(2), CellValue::Text("beta".into())],
                        vec![CellValue::Integer(3), CellValue::Text("gamma".into())],
                    ],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            // Sort descending on column 0: display order is [2, 1, 0].
            state.sort = Some((0, SortDirection::Descending));
            state.recompute();
            assert_eq!(state.display_indices.as_slice(), &[2, 1, 0]);

            // Cell target at display row 0 -> source row 2.
            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 2,
                column_index: 1,
            };
            let sel = Selection::Cell(0, 1);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.target.column_index(), Some(1));
            let cells = req.selected_cells();
            assert_eq!(cells.len(), 1);
            assert_eq!(cells[0].source_row_index, 2);
            assert_eq!(cells[0].column_name, "name");
            assert_eq!(cells[0].value, CellValue::Text("gamma".into()));
            let rows = req.selected_rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].source_row_index, 2);
            assert_eq!(rows[0].value_by_name("id"), Some(&CellValue::Integer(3)));

            // Cell-range selection (display rows 0-1, cols 0-1).
            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 2,
                column_index: 0,
            };
            let sel = Selection::CellRange(0, 0, 1, 1);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.selected_cell_count(), 4); // 2 rows x 2 cols
            let rows = req.selected_rows();
            assert_eq!(rows.len(), 2);
            // Display row 0 -> source 2, display row 1 -> source 1.
            assert_eq!(rows[0].source_row_index, 2);
            assert_eq!(rows[1].source_row_index, 1);

            // Row-range selection (display rows 0-2).
            let target = ContextMenuTarget::RowHeader {
                display_row_index: 1,
                source_row_index: 1,
            };
            let sel = Selection::RowRange(0, 2);
            let req = state.build_context_menu_request(target, &sel);
            let rows = req.selected_rows();
            assert_eq!(rows.len(), 3);
            // Each row should have all column values.
            assert_eq!(rows[0].values.len(), 2);
            assert_eq!(req.selected_cell_count(), 6); // 3 rows x 2 cols

            // Column selection (all display rows, column 0). Column-oriented
            // targets do not populate `selected_rows` (see doc comment); the
            // column's values are exposed via `selected_cells`.
            let target = ContextMenuTarget::ColumnHeader { column_index: 0 };
            let sel = Selection::Column(0);
            let req = state.build_context_menu_request(target, &sel);
            assert!(req.is_column_oriented());
            assert_eq!(req.selected_row_count(), 0);
            assert!(req.selected_rows().is_empty());
            assert_eq!(req.selected_cells().len(), 3); // 3 rows x 1 col

            // Empty data — no panic, empty vectors.
            let empty_state = GridState::new(
                GridData::new(vec![Column::new("x", ColumnKind::Integer, 80.0)], vec![])
                    .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 0,
                column_index: 0,
            };
            let req = empty_state.build_context_menu_request(target, &Selection::None);
            assert!(req.selected_cells().is_empty());
            assert!(req.selected_rows().is_empty());

            cx.quit();
        });
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn effective_selection_for_context_target() {
        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();
            let mut state = GridState::new(
                GridData::new(
                    vec![
                        Column::new("a", ColumnKind::Integer, 80.0),
                        Column::new("b", ColumnKind::Integer, 80.0),
                    ],
                    vec![
                        vec![CellValue::Integer(1), CellValue::Integer(2)],
                        vec![CellValue::Integer(3), CellValue::Integer(4)],
                    ],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus,
            );

            // Outside current selection -> collapses to target cell.
            state.selection = Selection::Cell(0, 0);
            let target = ContextMenuTarget::Cell {
                display_row_index: 1,
                source_row_index: 1,
                column_index: 1,
            };
            let eff = state.effective_selection_for_context_target(&target);
            assert_eq!(eff, Selection::Cell(1, 1));

            // Inside current selection -> keeps selection.
            state.selection = Selection::CellRange(0, 0, 1, 1);
            let target = ContextMenuTarget::Cell {
                display_row_index: 1,
                source_row_index: 1,
                column_index: 1,
            };
            let eff = state.effective_selection_for_context_target(&target);
            assert_eq!(eff, Selection::CellRange(0, 0, 1, 1));

            // Row header outside -> collapses to row.
            state.selection = Selection::Cell(0, 0);
            let target = ContextMenuTarget::RowHeader {
                display_row_index: 1,
                source_row_index: 1,
            };
            let eff = state.effective_selection_for_context_target(&target);
            assert_eq!(eff, Selection::Row(1));

            // Row header inside row range -> keeps range.
            state.selection = Selection::RowRange(0, 1);
            let target = ContextMenuTarget::RowHeader {
                display_row_index: 1,
                source_row_index: 1,
            };
            let eff = state.effective_selection_for_context_target(&target);
            assert_eq!(eff, Selection::RowRange(0, 1));

            // Column header -> does not change selection.
            state.selection = Selection::Cell(1, 1);
            let target = ContextMenuTarget::ColumnHeader { column_index: 0 };
            let eff = state.effective_selection_for_context_target(&target);
            assert_eq!(eff, Selection::Cell(1, 1));

            cx.quit();
        });
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn context_menu_target_from_hit_maps_correctly() {
        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();
            let state = GridState::new(
                GridData::new(
                    vec![Column::new("a", ColumnKind::Integer, 80.0)],
                    vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus,
            );

            // Cell hit -> Cell target with source mapping.
            let t = state
                .context_menu_target_from_hit(HitResult::Cell(1, 0))
                .unwrap();
            assert_eq!(
                t,
                ContextMenuTarget::Cell {
                    display_row_index: 1,
                    source_row_index: 1,
                    column_index: 0,
                }
            );

            // Row header -> RowHeader target.
            let t = state
                .context_menu_target_from_hit(HitResult::RowHeader(0))
                .unwrap();
            assert_eq!(
                t,
                ContextMenuTarget::RowHeader {
                    display_row_index: 0,
                    source_row_index: 0,
                }
            );

            // Column header -> ColumnHeader target.
            let t = state
                .context_menu_target_from_hit(HitResult::ColumnHeader(0))
                .unwrap();
            assert_eq!(t, ContextMenuTarget::ColumnHeader { column_index: 0 });

            // Sort button -> SortButton target.
            let t = state
                .context_menu_target_from_hit(HitResult::SortButton(0))
                .unwrap();
            assert_eq!(t, ContextMenuTarget::SortButton { column_index: 0 });

            // Unsupported hits -> None.
            assert!(state
                .context_menu_target_from_hit(HitResult::VerticalScrollbar)
                .is_none());
            assert!(state
                .context_menu_target_from_hit(HitResult::None)
                .is_none());

            cx.quit();
        });
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn convert_context_menu_items_maps_variants() {
        use crate::grid::context_menu::ContextMenuItem;

        let items = vec![
            ContextMenuItem::BuiltIn(MenuAction::SortAscending),
            ContextMenuItem::action("copy", "Copy value"),
            ContextMenuItem::separator(),
        ];
        let internal = GridState::convert_context_menu_items(items);
        assert!(matches!(
            internal[0],
            MenuItem::Action(MenuAction::SortAscending)
        ));
        assert!(
            matches!(&internal[1], MenuItem::Custom { id, label } if id == "copy" && label == "Copy value")
        );
        assert!(matches!(internal[2], MenuItem::Separator));
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn execute_custom_context_menu_action_invokes_provider() {
        use crate::grid::context_menu::{
            ContextMenuProvider, ContextMenuProviderHandle, ContextMenuRequest,
        };
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct TestProvider {
            last_action: Arc<Mutex<Option<String>>>,
        }
        impl ContextMenuProvider for TestProvider {
            fn menu_items(&self, _request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
                vec![ContextMenuItem::action("test", "Test")]
            }
            fn on_action(
                &self,
                action_id: &str,
                _request: &ContextMenuRequest,
                _state: &mut GridState,
                _cx: &mut gpui::App,
            ) {
                *self.last_action.lock().unwrap() = Some(action_id.to_string());
            }
        }

        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("a", ColumnKind::Integer, 80.0)],
                    vec![vec![CellValue::Integer(1)]],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus,
            );

            let last = Arc::new(Mutex::new(None));
            state.context_menu_provider = Some(ContextMenuProviderHandle::new(TestProvider {
                last_action: last.clone(),
            }));

            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 0,
                column_index: 0,
            };
            let request = state.build_context_menu_request(target, &Selection::Cell(0, 0));
            state.execute_custom_context_menu_action(
                PendingCustomContextMenuAction {
                    id: "test".into(),
                    request,
                },
                cx,
            );
            assert_eq!(*last.lock().unwrap(), Some("test".to_string()));
            assert!(state.context_menu.is_none());

            cx.quit();
        });
    }

    #[test]
    fn filter_panel_to_filter_with_all_checked_has_no_value_set() {
        let panel = FilterPanel {
            col: 0,
            anchor: Point {
                x: px(0.0),
                y: px(0.0),
            },
            kind: ColumnKind::Text,
            search: TextInput::default(),
            op_index: 0,
            op_menu_open: false,
            operand_a: TextInput::default(),
            operand_b: TextInput::default(),
            focus: FilterInput::Search,
            auto_apply: true,
            distinct: vec![
                FilterValueRow {
                    label: "alpha".into(),
                    checked: true,
                },
                FilterValueRow {
                    label: "beta".into(),
                    checked: true,
                },
            ],
        };
        let f = panel.to_filter();
        assert!(f.values.is_none(), "all checked => no value allow-list");
        assert!(
            !f.is_active(),
            "default predicate + all checked => inactive"
        );
    }

    #[test]
    fn filter_panel_to_filter_with_unchecked_value_builds_allow_set() {
        let panel = FilterPanel {
            col: 0,
            anchor: Point {
                x: px(0.0),
                y: px(0.0),
            },
            kind: ColumnKind::Text,
            search: TextInput::default(),
            op_index: 0,
            op_menu_open: false,
            operand_a: TextInput::default(),
            operand_b: TextInput::default(),
            focus: FilterInput::Search,
            auto_apply: true,
            distinct: vec![
                FilterValueRow {
                    label: "alpha".into(),
                    checked: true,
                },
                FilterValueRow {
                    label: "beta".into(),
                    checked: false,
                },
            ],
        };
        let f = panel.to_filter();
        assert!(f.is_active(), "unchecked value => active filter");
        let set = f.values.expect("should have a value set");
        assert!(set.contains("alpha"));
        assert!(!set.contains("beta"));
    }

    #[test]
    fn filter_panel_visible_indices_respects_search() {
        let panel = FilterPanel {
            col: 0,
            anchor: Point {
                x: px(0.0),
                y: px(0.0),
            },
            kind: ColumnKind::Text,
            search: TextInput::new("al".into()),
            op_index: 0,
            op_menu_open: false,
            operand_a: TextInput::default(),
            operand_b: TextInput::default(),
            focus: FilterInput::Search,
            auto_apply: true,
            distinct: vec![
                FilterValueRow {
                    label: "alpha".into(),
                    checked: true,
                },
                FilterValueRow {
                    label: "beta".into(),
                    checked: true,
                },
                FilterValueRow {
                    label: "gamma".into(),
                    checked: true,
                },
            ],
        };
        let vis = panel.visible_indices();
        assert_eq!(vis, vec![0], "search 'al' matches only alpha");
    }

    #[test]
    fn filter_panel_all_checked_ignores_search() {
        let mut panel = FilterPanel {
            col: 0,
            anchor: Point {
                x: px(0.0),
                y: px(0.0),
            },
            kind: ColumnKind::Text,
            search: TextInput::new("al".into()),
            op_index: 0,
            op_menu_open: false,
            operand_a: TextInput::default(),
            operand_b: TextInput::default(),
            focus: FilterInput::Search,
            auto_apply: true,
            distinct: vec![
                FilterValueRow {
                    label: "alpha".into(),
                    checked: true,
                },
                FilterValueRow {
                    label: "beta".into(),
                    checked: false,
                },
                FilterValueRow {
                    label: "gamma".into(),
                    checked: true,
                },
            ],
        };
        // Even though the search "al" hides beta (unchecked), "(Select All)"
        // reflects the GLOBAL checked state, so it must be false.
        assert!(
            !panel.all_checked(),
            "beta is unchecked, so not all values are checked (search is irrelevant)"
        );

        // A search that matches nothing must not flip "(Select All)".
        panel.search = TextInput::new("zzz".into());
        for row in &mut panel.distinct {
            row.checked = true;
        }
        assert!(
            panel.all_checked(),
            "all values checked -> Select All stays checked regardless of empty search"
        );
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    #[test]
    #[ignore = "requires gpui::Application which must run on the OS main thread; can only be executed under a custom main harness"]
    fn filter_panel_open_apply_clear_state_flow() {
        gpui::Application::new().run(|cx| {
            let focus = cx.focus_handle();
            let mut state = GridState::new(
                GridData::new(
                    vec![Column::new("name", ColumnKind::Text, 100.0)],
                    vec![
                        vec![CellValue::Text("alpha".into())],
                        vec![CellValue::Text("beta".into())],
                        vec![CellValue::Text("gamma".into())],
                    ],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus,
            );

            // Open filter panel for column 0 with an explicit anchor.
            let anchor = Point {
                x: px(50.0),
                y: px(20.0),
            };
            state.open_filter_panel(0, Some(anchor));
            let panel = state.filter_panel.as_ref().expect("panel should be open");
            assert_eq!(panel.col, 0);
            assert_eq!(panel.anchor, anchor);
            assert_eq!(panel.distinct.len(), 3);
            assert!(
                panel.distinct.iter().all(|r| r.checked),
                "all checked by default"
            );
            assert!(panel.auto_apply, "auto_apply defaults to true");
            assert_eq!(panel.kind, ColumnKind::Text);

            // Uncheck "beta" (index 1) and apply.
            state.toggle_filter_value(1);
            state.apply_filter_panel();
            assert_eq!(
                state.display_indices.as_slice(),
                &[0, 2],
                "beta should be filtered out"
            );

            // Clear the filter panel.
            state.clear_filter_panel();
            assert_eq!(
                state.display_indices.as_slice(),
                &[0, 1, 2],
                "all rows visible after clear"
            );
            assert!(
                state.filters[0] == ColumnFilter::default(),
                "filter reset to default"
            );

            // Open with a text "contains" predicate.
            state.open_filter_panel(0, Some(anchor));
            let panel = state.filter_panel.as_mut().expect("panel open");
            panel.op_index = 1; // "contains"
            panel.operand_a = TextInput::new("a".into());
            state.apply_filter_panel();
            assert_eq!(
                state.display_indices.as_slice(),
                &[0, 2],
                "contains 'a' matches alpha and gamma"
            );

            // Clear and verify restored.
            state.clear_filter_panel();
            assert_eq!(state.display_indices.as_slice(), &[0, 1, 2]);

            cx.quit();
        });
    }
}
