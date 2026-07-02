//! `GridState` plus all non-paint behaviour: input, scrollbars, drag,
//! sort/filter, scrolling, hit-testing, edge-scroll coordination, filter-prompt
//! cursor handling.

use crate::compare_cells;
use crate::data::{CellValue, GridData};
use crate::format::{cell_matches_filter, format_cell};
use crate::grid::state::state_inner::apply_edge_scroll;
use crate::grid::theme::GridTheme;

use crate::config::{GridConfig, ResolvedColumnFormat};
use gpui::{
    px, App, Bounds, FocusHandle, Keystroke, MouseButton, Pixels, Point, ScrollHandle, Size,
};

// Pull selection / menu types into scope unqualified for this module's impl.
use crate::grid::menu as menu_mod;
#[allow(unused_imports)]
pub(crate) use crate::grid::menu::{ContextMenu, MenuAction, MenuItem};
use crate::grid::selection::{
    is_cell_selected, is_row_selected, HitResult, ScrollbarAxis, Selection, SortDirection,
};

use crate::grid::context_menu::{
    ColumnContext, ContextMenuItem, ContextMenuProviderHandle, ContextMenuRequest,
    ContextMenuSelection, ContextMenuTarget, PendingCustomContextMenuAction, SelectedCellContext,
    SelectedRowContext,
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

/// Complete grid state owned by a GPUI `Entity<GridState>`.
#[derive(Debug)]
pub struct GridState {
    pub data: GridData,
    pub config: GridConfig,
    /// Cached resolved-format list, kept in sync with `data.columns` and
    /// `config`. Paint, copy, and filter read this directly instead of
    /// recomputing per cell.
    pub resolved_formats: Vec<ResolvedColumnFormat>,
    pub display_indices: Vec<usize>,
    pub selection: Selection,
    /// Fixed corner of a keyboard/shift range selection (row, col). Set when a
    /// single cell is selected; held steady while shift+arrow moves the active
    /// corner. Mirrors the Swift grid's `ResultGridCellRange.anchor`.
    pub(crate) range_anchor: Option<(usize, usize)>,
    /// Moving corner of a keyboard/shift range selection (row, col). Mirrors
    /// the Swift grid's `ResultGridCellRange.extent`.
    pub(crate) range_active: Option<(usize, usize)>,
    pub sort: Option<(usize, SortDirection)>,
    pub filters: Vec<String>,
    pub scroll_handle: ScrollHandle,
    pub focus_handle: FocusHandle,
    pub bounds: Bounds<Pixels>,
    pub row_height: f32,
    pub header_height: f32,
    pub row_header_width: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub theme: GridTheme,
    pub is_dragging: bool,
    pub drag_start: Option<Point<Pixels>>,
    pub drag_start_hit: Option<HitResult>,
    pub scroll_at_click: Option<Point<Pixels>>,
    pub last_mouse_pos: Option<Point<Pixels>>,
    pub status_bar_height: f32,
    pub click_pos: Option<Point<Pixels>>,
    pub click_hit: Option<HitResult>,
    pub hover_hit: Option<HitResult>,
    pub resizing_col: Option<usize>,
    pub resize_start_x: f32,
    pub resize_start_width: f32,
    pub context_menu: Option<ContextMenu>,
    pub filter_prompt: Option<FilterPrompt>,
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
}

/// Filter-prompt input. Cursor is tracked as a **char count**, not a byte
/// offset, so multi-byte input never panics on grapheme-misaligned inserts.
#[derive(Clone, Debug)]
pub struct FilterPrompt {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub input: String,
    pub cursor_chars: usize,
}

impl FilterPrompt {
    fn new(col: usize, anchor: Point<Pixels>, input: String) -> Self {
        let cursor_chars = input.chars().count();
        Self {
            col,
            anchor,
            input,
            cursor_chars,
        }
    }

    fn clamp_cursor(&mut self) {
        let total = self.input.chars().count();
        if self.cursor_chars > total {
            self.cursor_chars = total;
        }
    }

    fn insert_char(&mut self, ch: char) {
        let byte_idx = byte_index_for_char(&self.input, self.cursor_chars);
        self.input.insert(byte_idx, ch);
        self.cursor_chars += 1;
    }

    fn backspace(&mut self) {
        if self.cursor_chars == 0 {
            return;
        }
        let end = byte_index_for_char(&self.input, self.cursor_chars);
        let start = byte_index_for_char(&self.input, self.cursor_chars - 1);
        self.input.replace_range(start..end, "");
        self.cursor_chars -= 1;
    }
}

fn byte_index_for_char(input: &str, char_idx: usize) -> usize {
    input
        .char_indices()
        .nth(char_idx)
        .map_or(input.len(), |(idx, _)| idx)
}

impl GridState {
    #[must_use]
    pub fn new(data: GridData, config: GridConfig, focus_handle: FocusHandle) -> Self {
        let resolved_formats = config.resolve_all(&data.columns);
        let col_count = data.columns.len();
        let display_indices = (0..data.rows.len()).collect();
        Self {
            data,
            config,
            resolved_formats,
            display_indices,
            selection: Selection::None,
            range_anchor: None,
            range_active: None,
            sort: None,
            filters: vec![String::new(); col_count],
            scroll_handle: ScrollHandle::new(),
            focus_handle,
            bounds: Bounds::default(),
            row_height: 24.0,
            header_height: 32.0,
            row_header_width: 50.0,
            font_size: 14.0,
            char_width: 7.6,
            theme: GridTheme::default(),
            is_dragging: false,
            drag_start: None,
            drag_start_hit: None,
            scroll_at_click: None,
            last_mouse_pos: None,
            status_bar_height: 24.0,
            click_pos: None,
            click_hit: None,
            hover_hit: None,
            resizing_col: None,
            resize_start_x: 0.0,
            resize_start_width: 0.0,
            context_menu: None,
            filter_prompt: None,
            pending_action: None,
            pending_custom_context_menu_action: None,
            context_menu_provider: None,
            scrollbar_drag: None,
            scrollbar_drag_start_offset: 0.0,
            scrollbar_drag_start_pos: 0.0,
            window_viewport: Size::default(),
            edge_scroll_active: false,
        }
    }

    pub fn set_config(&mut self, config: GridConfig) {
        self.config = config;
        self.rebuild_resolved_formats();
        self.recompute();
    }

    fn rebuild_resolved_formats(&mut self) {
        self.resolved_formats = self.config.resolve_all(&self.data.columns);
    }

    pub fn recompute(&mut self) {
        let mut indices: Vec<usize> = (0..self.data.rows.len())
            .filter(|&row_idx| {
                self.data.columns.iter().enumerate().all(|(col_idx, _col)| {
                    let filter = &self.filters[col_idx];
                    if filter.is_empty() {
                        return true;
                    }
                    let cell = &self.data.rows[row_idx][col_idx];
                    cell_matches_filter(cell, &self.resolved_formats[col_idx], filter)
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
        self.display_indices = indices;
    }

    fn content_size(&self) -> (f32, f32) {
        let cw: f32 = self.data.columns.iter().map(|c| c.width).sum();
        let ch = self.display_indices.len() as f32 * self.row_height;
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
                self.selection = Selection::Column(col);
                self.clear_drag();
            }
            HitResult::SortButton(col) => {
                self.selection = Selection::Column(col);
                self.toggle_sort(col);
                self.clear_drag();
            }
            HitResult::ContextMenuItem(_) => {}
            HitResult::RowHeader(row) => {
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
                self.filter_prompt = None;
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
        self.filter_prompt = None;
    }

    /// Convert a hit-test result to a context-menu target. Returns `None`
    /// for hits that don't map to a meaningful right-click target.
    pub(crate) fn context_menu_target_from_hit(&self, hit: HitResult) -> Option<ContextMenuTarget> {
        match hit {
            HitResult::Cell(row, col) => {
                let source_row = self.display_indices.get(row).copied().unwrap_or(row);
                Some(ContextMenuTarget::Cell {
                    display_row_index: row,
                    source_row_index: source_row,
                    column_index: col,
                })
            }
            HitResult::RowHeader(row) => {
                let source_row = self.display_indices.get(row).copied().unwrap_or(row);
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

    /// Build an owned snapshot of the right-click context. All indices are
    /// clamped to current display/column counts; empty data produces empty
    /// vectors, never panics.
    pub(crate) fn build_context_menu_request(
        &self,
        target: ContextMenuTarget,
        selection: &Selection,
    ) -> ContextMenuRequest {
        let nrows = self.display_indices.len();
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

        let column_contexts: Vec<ColumnContext> = self
            .data
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| ColumnContext {
                index: i,
                name: c.name.clone(),
                kind: c.kind,
            })
            .collect();

        let mut selected_cells = Vec::new();
        let mut selected_rows = Vec::new();

        for dr in r1..=r2 {
            if nrows == 0 || dr >= nrows {
                break;
            }
            let Some(source_row) = self.display_indices.get(dr).copied() else {
                continue;
            };
            let Some(row_values) = self.data.rows.get(source_row) else {
                continue;
            };

            selected_rows.push(SelectedRowContext {
                display_row_index: dr,
                source_row_index: source_row,
                values: row_values.clone(),
                columns: column_contexts.clone(),
            });

            for c in c1..=c2 {
                if ncols == 0 || c >= ncols {
                    break;
                }
                if let (Some(col), Some(value)) = (self.data.columns.get(c), row_values.get(c)) {
                    selected_cells.push(SelectedCellContext {
                        display_row_index: dr,
                        source_row_index: source_row,
                        column_index: c,
                        column_name: col.name.clone(),
                        value: value.clone(),
                    });
                }
            }
        }

        ContextMenuRequest {
            target,
            selection: Some(menu_selection),
            selected_cells,
            selected_rows,
        }
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
        self.filter_prompt = None;

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
            MenuAction::FilterPrompt => {
                let anchor = self.last_mouse_pos.unwrap_or(Point {
                    x: px(0.0),
                    y: px(0.0),
                });
                let existing = self.filters.get(col).cloned().unwrap_or_default();
                self.filter_prompt = Some(FilterPrompt::new(col, anchor, existing));
            }
            MenuAction::ClearFilter => {
                if col < self.filters.len() {
                    self.filters[col].clear();
                    self.recompute();
                }
            }
        }
        self.context_menu = None;
    }

    fn column_text(&self, col: usize) -> String {
        let mut text = String::new();
        let fmt = &self.resolved_formats[col];
        for &row_idx in &self.display_indices {
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
        let nrows = self.display_indices.len();
        let ncols = self.data.columns.len();
        if nrows > 0 && ncols > 0 {
            self.selection = Selection::CellRange(0, 0, nrows - 1, ncols - 1);
        }
    }

    pub fn copy_selection(&self, with_headers: bool, cx: &mut App) {
        let Some((raw_r1, raw_c1, raw_r2, raw_c2)) = self.selection.normalized_bounds() else {
            return;
        };
        if self.display_indices.is_empty() || self.data.columns.is_empty() {
            return;
        }
        let last_row = self.display_indices.len() - 1;
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
            let row_idx = self.display_indices[dr];
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
        if let Some(prompt) = &mut self.filter_prompt {
            match keystroke.key.as_str() {
                "escape" => self.filter_prompt = None,
                "enter" => {
                    let col = prompt.col;
                    self.filters[col] = prompt.input.clone();
                    self.filter_prompt = None;
                    self.recompute();
                }
                "backspace" => prompt.backspace(),
                "left" => {
                    if prompt.cursor_chars > 0 {
                        prompt.cursor_chars -= 1;
                    }
                }
                "right" => {
                    prompt.clamp_cursor();
                    if prompt.cursor_chars < prompt.input.chars().count() {
                        prompt.cursor_chars += 1;
                    }
                }
                _ => {
                    if let Some(ch) = keystroke_to_char(keystroke) {
                        prompt.insert_char(ch);
                    }
                }
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
        let nrows = self.display_indices.len() as i32;
        let ncols = self.data.columns.len() as i32;
        if nrows == 0 || ncols == 0 {
            return;
        }
        let last_row = nrows - 1;
        let last_col = ncols - 1;
        match self.selection {
            Selection::Cell(row, col) => {
                let nr = (row as i32 + dy).clamp(0, last_row) as usize;
                let nc = (col as i32 + dx).clamp(0, last_col) as usize;
                self.selection = Selection::Cell(nr, nc);
                self.range_anchor = Some((nr, nc));
                self.range_active = Some((nr, nc));
            }
            Selection::Row(row) if dy != 0 => {
                let nr = (row as i32 + dy).clamp(0, last_row) as usize;
                self.selection = Selection::Row(nr);
            }
            Selection::Column(col) if dx != 0 => {
                let nc = (col as i32 + dx).clamp(0, last_col) as usize;
                self.selection = Selection::Column(nc);
            }
            _ => {
                self.selection = Selection::Cell(0, 0);
                self.range_anchor = Some((0, 0));
                self.range_active = Some((0, 0));
            }
        }
    }

    /// Extend a rectangular cell selection by moving the active corner while
    /// holding the anchor corner fixed (shift+arrow). Mirrors the Swift grid's
    /// anchor/extent range model. Row and column selections are left unchanged.
    fn extend_selection(&mut self, dx: i32, dy: i32) {
        let nrows = self.display_indices.len() as i32;
        let ncols = self.data.columns.len() as i32;
        if nrows == 0 || ncols == 0 {
            return;
        }
        let last_row = nrows - 1;
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
                    self.range_anchor = Some((0, 0));
                    self.range_active = Some((0, 0));
                    self.selection = Selection::Cell(0, 0);
                }
            }
        }

        let anchor = self.range_anchor.unwrap_or((0, 0));
        let active = self.range_active.unwrap_or(anchor);
        let nr = (active.0 as i32 + dy).clamp(0, last_row) as usize;
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
            if row_idx < self.display_indices.len() {
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
        if row_idx >= self.display_indices.len() {
            return HitResult::None;
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

    fn anchor() -> Point<Pixels> {
        Point {
            x: px(0.0),
            y: px(0.0),
        }
    }

    fn prompt_with(text: &str, cursor: usize) -> FilterPrompt {
        let mut p = FilterPrompt::new(0, anchor(), text.to_owned());
        p.cursor_chars = cursor;
        p
    }

    #[test]
    fn filter_prompt_new_cursors_at_char_count_not_bytes() {
        // "hé🙂" is 3 chars but 7 bytes (h=1, é=2, 🙂=4).
        let p = FilterPrompt::new(0, anchor(), "hé🙂".into());
        assert_eq!(p.cursor_chars, 3);
        assert_eq!(p.input.len(), 7);
    }

    #[test]
    fn filter_prompt_insert_emoji_at_start_does_not_panic() {
        let mut p = prompt_with("ab", 0);
        p.insert_char('\u{1F600}');
        assert_eq!(p.input, "\u{1F600}ab");
        assert_eq!(p.cursor_chars, 1);
    }

    #[test]
    fn filter_prompt_insert_in_middle_keeps_cursor_at_char_position() {
        let mut p = prompt_with("helloworld", 5);
        p.insert_char(' ');
        assert_eq!(p.input, "hello world");
        assert_eq!(p.cursor_chars, 6);
    }

    #[test]
    fn filter_prompt_backspace_at_zero_is_noop() {
        let mut p = prompt_with("abc", 0);
        p.backspace();
        assert_eq!(p.input, "abc");
        assert_eq!(p.cursor_chars, 0);
    }

    #[test]
    fn filter_prompt_backspace_removes_one_char_value() {
        // Cursor sits after "hé" (2 chars); backspace should delete "é" only.
        let mut p = prompt_with("héx", 2);
        p.backspace();
        assert_eq!(p.input, "hx");
        assert_eq!(p.cursor_chars, 1);
    }

    #[test]
    fn filter_prompt_clamp_cursor_pulls_back_past_end() {
        let mut p = prompt_with("abc", 99);
        p.clamp_cursor();
        assert_eq!(p.cursor_chars, 3);
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
                        vec![CellValue::Text("beta".into())],
                        vec![CellValue::Text("gamma".into())],
                    ],
                )
                .expect("rectangular"),
                crate::config::GridConfig::default(),
                focus.clone(),
            );
            state.filters[0] = "a".into();
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices, vec![0, 2]);
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices, vec![2, 0]);
            state.filters[0].clear();
            state.toggle_sort(0);
            state.recompute();
            assert_eq!(state.display_indices, vec![0, 1, 2]);

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
            assert_eq!(state.display_indices, vec![2, 1, 0]);

            // Cell target at display row 0 -> source row 2.
            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 2,
                column_index: 1,
            };
            let sel = Selection::Cell(0, 1);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.target.column_index(), Some(1));
            assert_eq!(req.selected_cells.len(), 1);
            assert_eq!(req.selected_cells[0].source_row_index, 2);
            assert_eq!(req.selected_cells[0].column_name, "name");
            assert_eq!(req.selected_cells[0].value, CellValue::Text("gamma".into()));
            assert_eq!(req.selected_rows.len(), 1);
            assert_eq!(req.selected_rows[0].source_row_index, 2);
            assert_eq!(
                req.selected_rows[0].value_by_name("id"),
                Some(&CellValue::Integer(3))
            );

            // Cell-range selection (display rows 0-1, cols 0-1).
            let target = ContextMenuTarget::Cell {
                display_row_index: 0,
                source_row_index: 2,
                column_index: 0,
            };
            let sel = Selection::CellRange(0, 0, 1, 1);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.selected_cells.len(), 4); // 2 rows x 2 cols
            assert_eq!(req.selected_rows.len(), 2);
            // Display row 0 -> source 2, display row 1 -> source 1.
            assert_eq!(req.selected_rows[0].source_row_index, 2);
            assert_eq!(req.selected_rows[1].source_row_index, 1);

            // Row-range selection (display rows 0-2).
            let target = ContextMenuTarget::RowHeader {
                display_row_index: 1,
                source_row_index: 1,
            };
            let sel = Selection::RowRange(0, 2);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.selected_rows.len(), 3);
            // Each row should have all column values.
            assert_eq!(req.selected_rows[0].values.len(), 2);
            assert_eq!(req.selected_cells.len(), 6); // 3 rows x 2 cols

            // Column selection (all display rows, column 0).
            let target = ContextMenuTarget::ColumnHeader { column_index: 0 };
            let sel = Selection::Column(0);
            let req = state.build_context_menu_request(target, &sel);
            assert_eq!(req.selected_rows.len(), 3);
            assert_eq!(req.selected_cells.len(), 3); // 3 rows x 1 col

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
            assert!(req.selected_cells.is_empty());
            assert!(req.selected_rows.is_empty());

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
}
