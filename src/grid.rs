use crate::data::{CellValue, GridData, compare_cells};
use crate::format::{cell_matches_filter, format_cell};
use gpui::prelude::*;
use gpui::{
    canvas, div, point, px, size, App, Bounds, Context, CursorStyle, Entity, FocusHandle,
    Focusable, Hsla, InteractiveElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point, Render,
    ScrollHandle, ScrollWheelEvent, Styled, TextRun, Window, WindowTextSystem,
};
use std::time::Duration;

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

fn pf(p: Pixels) -> f32 {
    p.into()
}

#[derive(Clone)]
pub struct GridTheme {
    pub bg: Hsla,
    pub header_bg: Hsla,
    pub filter_bg: Hsla,
    pub filter_active_bg: Hsla,
    pub row_header_bg: Hsla,
    pub selection_bg: Hsla,
    pub alt_row_bg: Hsla,
    pub grid_line: Hsla,
    pub header_fg: Hsla,
    pub text_fg: Hsla,
    pub negative_fg: Hsla,
    pub sort_indicator: Hsla,
    pub filter_cursor: Hsla,
}

impl Default for GridTheme {
    fn default() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 1.0, 1.0),
            header_bg: hsla(0.0, 0.0, 0.93, 1.0),
            filter_bg: hsla(0.0, 0.0, 0.96, 1.0),
            filter_active_bg: hsla(0.58, 0.30, 0.85, 1.0),
            row_header_bg: hsla(0.0, 0.0, 0.90, 1.0),
            selection_bg: hsla(0.58, 0.50, 0.80, 0.50),
            alt_row_bg: hsla(0.0, 0.0, 0.95, 1.0),
            grid_line: hsla(0.0, 0.0, 0.85, 1.0),
            header_fg: hsla(0.0, 0.0, 0.15, 1.0),
            text_fg: hsla(0.0, 0.0, 0.1, 1.0),
            negative_fg: hsla(0.0, 0.75, 0.45, 1.0),
            sort_indicator: hsla(0.58, 0.50, 0.40, 1.0),
            filter_cursor: hsla(0.0, 0.0, 0.1, 1.0),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Selection {
    None,
    Cell(usize, usize),
    Row(usize),
    Column(usize),
    CellRange(usize, usize, usize, usize),
    RowRange(usize, usize),
}

#[derive(Clone, Copy, Debug)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy)]
pub enum HitResult {
    None,
    ColumnHeader(usize),
    SortButton(usize),
    ColumnBorder(usize),
    RowHeader(usize),
    Cell(usize, usize),
    Corner,
    ContextMenuItem(usize),
    VerticalScrollbar,
    HorizontalScrollbar,
}

const SCROLLBAR_SIZE: f32 = 20.0;

pub struct GridState {
    pub data: GridData,
    pub display_indices: Vec<usize>,
    pub selection: Selection,
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
    pub scroll_at_click: Option<Point<Pixels>>,
    pub last_mouse_pos: Option<Point<Pixels>>,
    pub status_bar_height: f32,
    pub click_pos: Option<Point<Pixels>>,
    pub click_hit: Option<HitResult>,
    pub hover_hit: Option<HitResult>,
    pub resizing_col: Option<usize>,
    pub resize_start_x: f32,
    pub resize_start_width: f32,
    pub edge_scroll_started: bool,
    pub context_menu: Option<ContextMenu>,
    pub filter_prompt: Option<FilterPrompt>,
    pub pending_action: Option<(MenuAction, usize)>,
    pub pending_drag_selection: Option<Selection>,
    pub scrollbar_drag: Option<ScrollbarAxis>,
    pub scrollbar_drag_start_offset: f32,
    pub scrollbar_drag_start_pos: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone)]
pub struct ContextMenu {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub items: Vec<MenuItem>,
    pub hovered: Option<usize>,
}

#[derive(Clone)]
pub enum MenuItem {
    Action(MenuAction),
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAction {
    SelectColumn,
    CopyColumn,
    CopyColumnWithHeaders,
    SortAscending,
    SortDescending,
    ClearSort,
    FilterPrompt,
    ClearFilter,
}

#[derive(Clone)]
pub struct FilterPrompt {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub input: String,
    pub cursor: usize,
}

impl GridState {
    pub fn new(data: GridData, focus_handle: FocusHandle) -> Self {
        let col_count = data.columns.len();
        let display_indices = (0..data.rows.len()).collect();
        Self {
            data,
            display_indices,
            selection: Selection::None,
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
            scroll_at_click: None,
            last_mouse_pos: None,
            status_bar_height: 24.0,
            click_pos: None,
            click_hit: None,
            hover_hit: None,
            resizing_col: None,
            resize_start_x: 0.0,
            resize_start_width: 0.0,
            edge_scroll_started: false,
            context_menu: None,
            filter_prompt: None,
            pending_action: None,
            pending_drag_selection: None,
            scrollbar_drag: None,
            scrollbar_drag_start_offset: 0.0,
            scrollbar_drag_start_pos: 0.0,
        }
    }

    pub fn recompute(&mut self) {
        let mut indices: Vec<usize> = (0..self.data.rows.len())
            .filter(|&row_idx| {
                self.data.columns.iter().enumerate().all(|(col_idx, col)| {
                    let filter = &self.filters[col_idx];
                    if filter.is_empty() {
                        return true;
                    }
                    let cell = &self.data.rows[row_idx][col_idx];
                    cell_matches_filter(cell, &col.col_type, filter)
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

    fn max_scroll(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let (rw, rh) = self.scrollbar_reserved();
        let vw = pf(self.bounds.size.width) - self.row_header_width - rw;
        let vh = pf(self.bounds.size.height) - self.header_height - rh;
        ((cw - vw).max(0.0), (ch - vh).max(0.0))
    }

    /// Returns (reserved_width_on_right, reserved_height_on_bottom).
    /// Vertical bar (right, reserves width) appears when content is taller
    /// than the viewport; horizontal bar (bottom, reserves height) appears
    /// when content is wider than the viewport.
    fn scrollbar_reserved(&self) -> (f32, f32) {
        let (cw, ch) = self.content_size();
        let vw = pf(self.bounds.size.width) - self.row_header_width;
        let vh = pf(self.bounds.size.height) - self.header_height;
        let reserved_w = if ch > vh { SCROLLBAR_SIZE } else { 0.0 };
        let reserved_h = if cw > vw { SCROLLBAR_SIZE } else { 0.0 };
        (reserved_w, reserved_h)
    }

    /// Vertical scrollbar geometry in window coords: (track_x, track_y, track_w, track_h, thumb_h).
    /// Returns None when no vertical scrolling is available.
    fn vbar_geom(&self) -> Option<(f32, f32, f32, f32, f32)> {
        let (_, ch) = self.content_size();
        let (_rw, rh) = self.scrollbar_reserved();
        let vh = pf(self.bounds.size.height) - self.header_height - rh;
        if ch <= vh {
            return None;
        }
        let ox = pf(self.bounds.origin.x);
        let oy = pf(self.bounds.origin.y);
        let sw = pf(self.bounds.size.width);
        let sh = pf(self.bounds.size.height);
        let track_x = ox + sw - SCROLLBAR_SIZE;
        let track_y = oy + self.header_height;
        let track_h = sh - self.header_height - rh;
        let thumb_h = ((track_h * (vh / ch)).max(20.0)).min(track_h);
        Some((track_x, track_y, SCROLLBAR_SIZE, track_h, thumb_h))
    }

    /// Horizontal scrollbar geometry in window coords: (track_x, track_y, track_w, track_h, thumb_w).
    /// Returns None when no horizontal scrolling is available.
    fn hbar_geom(&self) -> Option<(f32, f32, f32, f32, f32)> {
        let (cw, _) = self.content_size();
        let (rw, _rh) = self.scrollbar_reserved();
        let vw = pf(self.bounds.size.width) - self.row_header_width - rw;
        if cw <= vw {
            return None;
        }
        let ox = pf(self.bounds.origin.x);
        let oy = pf(self.bounds.origin.y);
        let sw = pf(self.bounds.size.width);
        let sh = pf(self.bounds.size.height);
        let track_x = ox + self.row_header_width;
        let track_y = oy + sh - SCROLLBAR_SIZE;
        let track_w = sw - self.row_header_width - rw;
        let thumb_w = ((track_w * (vw / cw)).max(20.0)).min(track_w);
        Some((track_x, track_y, track_w, SCROLLBAR_SIZE, thumb_w))
    }

    /// Set vertical scroll so the thumb is centered under `mouse_y` (window coord).
    fn scroll_to_vbar(&mut self, mouse_y: f32) {
        if let Some((_, track_y, _, track_h, thumb_h)) = self.vbar_geom() {
            let (_, max_y) = self.max_scroll();
            let range = (track_h - thumb_h).max(0.0);
            let rel = (mouse_y - track_y - thumb_h * 0.5).clamp(0.0, range);
            let frac = if range > 0.0 { rel / range } else { 0.0 };
            let new_y = frac * max_y;
            let x = self.scroll_handle.offset().x;
            self.scroll_handle.set_offset(point(x, px(new_y)));
        }
    }

    /// Set horizontal scroll so the thumb is centered under `mouse_x` (window coord).
    fn scroll_to_hbar(&mut self, mouse_x: f32) {
        if let Some((track_x, _, track_w, _, thumb_w)) = self.hbar_geom() {
            let (max_x, _) = self.max_scroll();
            let range = (track_w - thumb_w).max(0.0);
            let rel = (mouse_x - track_x - thumb_w * 0.5).clamp(0.0, range);
            let frac = if range > 0.0 { rel / range } else { 0.0 };
            let new_x = frac * max_x;
            let y = self.scroll_handle.offset().y;
            self.scroll_handle.set_offset(point(px(new_x), y));
        }
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
                self.scroll_to_vbar(pf(pos.y));
                self.clear_drag();
            }
            HitResult::HorizontalScrollbar => {
                self.scrollbar_drag = Some(ScrollbarAxis::Horizontal);
                self.scroll_to_hbar(pf(pos.x));
                self.clear_drag();
            }
            HitResult::ColumnBorder(col) => {
                self.resizing_col = Some(col);
                self.resize_start_x = pf(pos.x);
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
            HitResult::ContextMenuItem(idx) => {
                if let Some(menu) = &self.context_menu {
                    let col = menu.col;
                    let mut cur = 0;
                    for item in &menu.items {
                        if let MenuItem::Action(action) = item {
                            if cur == idx {
                                self.pending_action = Some((*action, col));
                                self.context_menu = None;
                                break;
                            }
                            cur += 1;
                        }
                    }
                }
            }
            HitResult::RowHeader(row) => {
                if shift {
                    if let Selection::Row(prev) = &self.selection {
                        let (s, e) = (*prev, row);
                        self.selection = Selection::RowRange(s.min(e), s.max(e));
                    } else {
                        self.selection = Selection::Row(row);
                    }
                } else {
                    self.selection = Selection::Row(row);
                }
                self.start_drag(pos);
            }
            HitResult::Cell(row, col) => {
                if shift {
                    if let Selection::Cell(pr, pc) = &self.selection {
                        self.selection =
                            Selection::CellRange(*pr.min(&row), *pc.min(&col), *pr.max(&row), *pc.max(&col));
                    } else {
                        self.selection = Selection::Cell(row, col);
                    }
                } else {
                    self.selection = Selection::Cell(row, col);
                }
                self.start_drag(pos);
            }
            HitResult::Corner | HitResult::None => {
                self.selection = Selection::None;
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

    fn open_context_menu(&mut self, col: usize, anchor: Point<Pixels>) {
        let items = vec![
            MenuItem::Action(MenuAction::SelectColumn),
            MenuItem::Action(MenuAction::CopyColumn),
            MenuItem::Action(MenuAction::CopyColumnWithHeaders),
            MenuItem::Separator,
            MenuItem::Action(MenuAction::SortAscending),
            MenuItem::Action(MenuAction::SortDescending),
            MenuItem::Action(MenuAction::ClearSort),
            MenuItem::Separator,
            MenuItem::Action(MenuAction::FilterPrompt),
            MenuItem::Action(MenuAction::ClearFilter),
        ];
        self.context_menu = Some(ContextMenu {
            col,
            anchor,
            items,
            hovered: None,
        });
        self.filter_prompt = None;
    }

    fn menu_bounds(&self, menu: &ContextMenu) -> Option<(f32, f32, f32, f32)> {
        if self.data.columns.is_empty() {
            return None;
        }
        let fs = self.font_size;
        let cw = self.char_width;
        let item_h = fs + 8.0;
        let pad_x = 12.0;
        let min_w: f32 = 180.0;
        let mut max_label_w = 0.0_f32;
        for item in &menu.items {
            if let MenuItem::Action(a) = item {
                max_label_w = max_label_w.max(menu_label(*a).len() as f32 * cw);
            }
        }
        let w = min_w.max(max_label_w + pad_x * 2.0);
        let total_h = menu.items.len() as f32 * item_h + 8.0;
        Some((pf(menu.anchor.x), pf(menu.anchor.y), w, total_h))
    }

    pub fn execute_action(&mut self, action: MenuAction, col: usize, cx: &mut App) {
        match action {
            MenuAction::SelectColumn => {
                self.selection = Selection::Column(col);
            }
            MenuAction::CopyColumn => {
                let mut text = String::new();
                for &row_idx in &self.display_indices {
                    let cell = &self.data.rows[row_idx][col];
                    let (s, _) = crate::format::format_cell(cell, &self.data.columns[col].col_type);
                    text.push_str(&s);
                    text.push('\n');
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            MenuAction::CopyColumnWithHeaders => {
                let mut text = String::new();
                text.push_str(&self.data.columns[col].name);
                text.push('\n');
                for &row_idx in &self.display_indices {
                    let cell = &self.data.rows[row_idx][col];
                    let (s, _) = crate::format::format_cell(cell, &self.data.columns[col].col_type);
                    text.push_str(&s);
                    text.push('\n');
                }
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
                let anchor = self.last_mouse_pos.unwrap_or(point(px(0.0), px(0.0)));
                let existing = self.filters.get(col).cloned().unwrap_or_default();
                let cursor = existing.len();
                self.filter_prompt = Some(FilterPrompt {
                    col,
                    anchor,
                    input: existing,
                    cursor,
                });
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

    fn clear_drag(&mut self) {
        self.is_dragging = false;
        self.drag_start = None;
        self.scroll_at_click = None;
        self.last_mouse_pos = None;
        self.pending_drag_selection = None;
    }

    /// World (content) coordinates of the two drag corners.
    /// World = window position + scroll offset, so the start corner stays
    /// anchored to the content cell that was clicked even as the viewport scrolls.
    fn drag_world_corners(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let start = self.drag_start?;
        let mouse = self.last_mouse_pos?;
        let click_scroll = self.scroll_at_click.unwrap_or_else(|| self.scroll_handle.offset());
        let scroll = self.scroll_handle.offset();
        let start_world = point(
            px(pf(start.x) + pf(click_scroll.x)),
            px(pf(start.y) + pf(click_scroll.y)),
        );
        let end_world = point(
            px(pf(mouse.x) + pf(scroll.x)),
            px(pf(mouse.y) + pf(scroll.y)),
        );
        Some((start_world, end_world))
    }

    /// The drag rectangle in window (screen) coordinates, for painting.
    /// Converts world -> viewport by subtracting the current scroll offset.
    pub fn drag_screen_rect(&self) -> Option<(Point<Pixels>, Point<Pixels>)> {
        if !self.is_dragging {
            return None;
        }
        let (start_world, end_world) = self.drag_world_corners()?;
        let scroll = self.scroll_handle.offset();
        let start_screen = point(
            px(pf(start_world.x) - pf(scroll.x)),
            px(pf(start_world.y) - pf(scroll.y)),
        );
        let end_screen = point(
            px(pf(end_world.x) - pf(scroll.x)),
            px(pf(end_world.y) - pf(scroll.y)),
        );
        Some((start_screen, end_screen))
    }

    fn update_drag(&mut self) {
        let (start_world, end_world) = match self.drag_world_corners() {
            Some(c) => c,
            None => return,
        };
        if !self.is_dragging {
            let dx = pf(end_world.x) - pf(start_world.x);
            let dy = pf(end_world.y) - pf(start_world.y);
            if dx * dx + dy * dy <= 400.0 {
                return;
            }
            self.is_dragging = true;
        }
        // Convert world -> window for hit_test (hit_test adds scroll internally).
        let scroll = self.scroll_handle.offset();
        let start_win = point(
            px(pf(start_world.x) - pf(scroll.x)),
            px(pf(start_world.y) - pf(scroll.y)),
        );
        let end_win = point(
            px(pf(end_world.x) - pf(scroll.x)),
            px(pf(end_world.y) - pf(scroll.y)),
        );
        let r1 = self.hit_test(start_win);
        let r2 = self.hit_test(end_win);
        match (r1, r2) {
            (HitResult::Cell(r1c, _), HitResult::Cell(r2c, c2)) => {
                let (r1c, c1) = if let Selection::Cell(_, pc) = &self.selection {
                    (r1c, *pc)
                } else {
                    (r1c, c2)
                };
                self.pending_drag_selection =
                    Some(Selection::CellRange(r1c.min(r2c), c1.min(c2), r1c.max(r2c), c1.max(c2)));
            }
            (HitResult::RowHeader(r1r), HitResult::RowHeader(r2r)) => {
                self.pending_drag_selection = Some(Selection::RowRange(r1r.min(r2r), r1r.max(r2r)));
            }
            _ => {}
        }
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
            let new_w = (self.resize_start_width + (pf(pos.x) - self.resize_start_x)).max(40.0);
            self.data.columns[col].width = new_w;
            return;
        }
        // Scrollbar drag
        if let Some(axis) = self.scrollbar_drag {
            if pressed_button != Some(MouseButton::Left) {
                self.scrollbar_drag = None;
                return;
            }
            match axis {
                ScrollbarAxis::Vertical => self.scroll_to_vbar(pf(pos.y)),
                ScrollbarAxis::Horizontal => self.scroll_to_hbar(pf(pos.x)),
            }
            self.last_mouse_pos = Some(pos);
            return;
        }
        self.last_mouse_pos = Some(pos);
        // Context menu hover tracking
        if let Some(menu) = &mut self.context_menu {
            let x = pf(pos.x) - pf(self.bounds.origin.x);
            let y = pf(pos.y) - pf(self.bounds.origin.y);
            menu.hovered = menu_hover_at(menu, x, y);
            self.hover_hit = Some(self.hit_test(pos));
            return;
        }
        self.hover_hit = Some(self.hit_test(pos));
        if self.drag_start.is_none() {
            return;
        }
        self.update_drag();
    }

    pub fn handle_scroll_drag(&mut self) {
        if self.drag_start.is_none() || self.last_mouse_pos.is_none() {
            return;
        }
        self.update_drag();
    }

    pub fn handle_mouse_up(&mut self) {
        self.resizing_col = None;
        self.scrollbar_drag = None;
        if let Some(sel) = self.pending_drag_selection.take() {
            self.selection = sel;
        }
        self.clear_drag();
    }

    fn edge_scroll_speed(dist_from_edge: f32) -> f32 {
        if dist_from_edge > 150.0 {
            return 0.0;
        }
        if dist_from_edge < 0.0 {
            // Past the edge: over a header or outside the window. Accelerate
            // hard, scaling with how far past the edge the mouse is.
            return (24.0 + (-dist_from_edge) * 0.6).min(80.0);
        }
        if dist_from_edge < 25.0 {
            12.0
        } else if dist_from_edge < 50.0 {
            6.0
        } else if dist_from_edge < 100.0 {
            3.0
        } else {
            1.0
        }
    }

    pub fn apply_edge_scroll(&mut self) -> bool {
        if !self.is_dragging {
            return false;
        }
        let pos = match self.last_mouse_pos {
            Some(p) => p,
            None => return false,
        };
        let bounds = self.bounds;
        let x = pf(pos.x) - pf(bounds.origin.x);
        let y = pf(pos.y) - pf(bounds.origin.y);
        let vw = pf(bounds.size.width);
        let vh = pf(bounds.size.height);

        let right_dist = vw - x;
        let left_dist = x - self.row_header_width;
        let bottom_dist = vh - y;
        let top_dist = y - self.header_height;

        let mut dx = 0.0_f32;
        let mut dy = 0.0_f32;

        if right_dist < 150.0 && right_dist <= left_dist {
            dx = Self::edge_scroll_speed(right_dist);
        } else if left_dist < 150.0 {
            dx = -Self::edge_scroll_speed(left_dist);
        }

        if bottom_dist < 150.0 && bottom_dist <= top_dist {
            dy = Self::edge_scroll_speed(bottom_dist);
        } else if top_dist < 150.0 {
            dy = -Self::edge_scroll_speed(top_dist);
        }

        if dx == 0.0 && dy == 0.0 {
            return false;
        }

        let (mx, my) = self.max_scroll();
        let scroll = self.scroll_handle.offset();
        let new_x = (pf(scroll.x) + dx).clamp(0.0, mx);
        let new_y = (pf(scroll.y) + dy).clamp(0.0, my);
        self.scroll_handle.set_offset(point(px(new_x), px(new_y)));

        if self.drag_start.is_some() {
            self.handle_scroll_drag();
        }
        true
    }

    pub fn handle_key(&mut self, keystroke: &Keystroke) {
        if let Some(prompt) = &mut self.filter_prompt {
            match keystroke.key.as_str() {
                "escape" => {
                    self.filter_prompt = None;
                }
                "enter" => {
                    let col = prompt.col;
                    self.filters[col] = prompt.input.clone();
                    self.filter_prompt = None;
                    self.recompute();
                }
                "backspace" => {
                    if prompt.cursor > 0 {
                        prompt.input.remove(prompt.cursor - 1);
                        prompt.cursor -= 1;
                    }
                }
                "left" => {
                    if prompt.cursor > 0 {
                        prompt.cursor -= 1;
                    }
                }
                "right" => {
                    if prompt.cursor < prompt.input.len() {
                        prompt.cursor += 1;
                    }
                }
                _ => {
                    if let Some(ch) = keystroke_to_char(keystroke) {
                        prompt.input.insert(prompt.cursor, ch);
                        prompt.cursor += 1;
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
        match keystroke.key.as_str() {
            "up" => self.move_selection(0, -1),
            "down" => self.move_selection(0, 1),
            "left" => self.move_selection(-1, 0),
            "right" => self.move_selection(1, 0),
            "escape" => self.selection = Selection::None,
            _ => {}
        }
    }

    fn move_selection(&mut self, dx: i32, dy: i32) {
        let nrows = self.display_indices.len();
        let ncols = self.data.columns.len();
        if nrows == 0 || ncols == 0 {
            return;
        }
        match &self.selection {
            Selection::Cell(row, col) => {
                let nr = (*row as i32 + dy).max(0).min(nrows as i32 - 1) as usize;
                let nc = (*col as i32 + dx).max(0).min(ncols as i32 - 1) as usize;
                self.selection = Selection::Cell(nr, nc);
            }
            Selection::Row(row) => {
                if dy != 0 {
                    let nr = (*row as i32 + dy).max(0).min(nrows as i32 - 1) as usize;
                    self.selection = Selection::Row(nr);
                }
            }
            Selection::Column(col) => {
                if dx != 0 {
                    let nc = (*col as i32 + dx).max(0).min(ncols as i32 - 1) as usize;
                    self.selection = Selection::Column(nc);
                }
            }
            _ => {
                self.selection = Selection::Cell(0, 0);
            }
        }
    }

    fn hit_test(&self, pos: Point<Pixels>) -> HitResult {
        let bounds = self.bounds;
        let x = pf(pos.x) - pf(bounds.origin.x);
        let y = pf(pos.y) - pf(bounds.origin.y);
        let scroll = self.scroll_handle.offset();
        let sx = pf(scroll.x);
        let sy = pf(scroll.y);
        let bw = pf(bounds.size.width);
        let bh = pf(bounds.size.height);
        let (mx, my) = self.max_scroll();
        if x < 0.0 || y < 0.0 {
            return HitResult::None;
        }
        // Scrollbars: 20px strips at right and bottom edges, only when scrollable
        if my > 0.0 && x >= bw - SCROLLBAR_SIZE && y >= self.header_height {
            return HitResult::VerticalScrollbar;
        }
        if mx > 0.0 && y >= bh - SCROLLBAR_SIZE && x >= self.row_header_width {
            return HitResult::HorizontalScrollbar;
        }
        if y < self.header_height {
            if x < self.row_header_width {
                return HitResult::Corner;
            }
            let col_x = x - self.row_header_width + sx;
            let mut acc = 0.0;
            for (i, col) in self.data.columns.iter().enumerate() {
                let right = acc + col.width;
                if i + 1 < self.data.columns.len()
                    && col_x >= right - 5.0
                    && col_x <= right + 5.0
                {
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
}

fn keystroke_to_char(k: &Keystroke) -> Option<char> {
    if k.modifiers.control || k.modifiers.platform || k.modifiers.alt {
        return None;
    }
    // Prefer key_char (the actual typed character) if available
    if let Some(key_char) = &k.key_char {
        return key_char.chars().next();
    }
    // Fall back to key for single-char keys
    if k.key.len() == 1 {
        let c = k.key.chars().next().unwrap();
        if k.modifiers.shift {
            Some(c.to_ascii_uppercase())
        } else {
            Some(c)
        }
    } else {
        None
    }
}

fn fill_quad(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
    window.paint_quad(PaintQuad {
        bounds: Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        },
        background: color.into(),
        border_color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 },
        border_widths: Default::default(),
        corner_radii: Default::default(),
        border_style: Default::default(),
    });
}

fn paint_scrollbars(
    data: &PaintData,
    window: &mut Window,
    ox: f32,
    oy: f32,
    sw: f32,
    sh: f32,
    theme: &GridTheme,
) {
    let scroll = data.scroll_offset;
    let (content_w, content_h) = (
        data.columns.iter().map(|c| c.width).sum::<f32>(),
        data.display_indices.len() as f32 * data.row_height,
    );
    let vw_full = sw - data.row_header_width;
    let vh_full = sh - data.header_height;
    // Vertical bar (right) present when content is taller than viewport;
    // horizontal bar (bottom) present when content is wider than viewport.
    let has_v = content_h > vh_full;
    let has_h = content_w > vw_full;
    let reserved_w = if has_v { SCROLLBAR_SIZE } else { 0.0 };
    let reserved_h = if has_h { SCROLLBAR_SIZE } else { 0.0 };
    let vw = vw_full - reserved_w;
    let vh = vh_full - reserved_h;
    let max_x = (content_w - vw).max(0.0);
    let max_y = (content_h - vh).max(0.0);
    let sx = pf(scroll.x);
    let sy = pf(scroll.y);
    let track_bg = theme.row_header_bg;
    let thumb_color = hsla(0.0, 0.0, 0.55, 1.0);

    // Vertical scrollbar (right edge)
    if has_v {
        let track_x = ox + sw - SCROLLBAR_SIZE;
        let track_y = oy + data.header_height;
        let track_h = sh - data.header_height - reserved_h;
        if track_h > 0.0 {
            fill_quad(window, track_x, track_y, SCROLLBAR_SIZE, track_h, track_bg);
            let thumb_h = ((track_h * (vh / content_h)).max(20.0)).min(track_h);
            let frac = if max_y > 0.0 { sy / max_y } else { 0.0 };
            let thumb_y = track_y + frac * (track_h - thumb_h);
            fill_quad(window, track_x + 3.0, thumb_y, SCROLLBAR_SIZE - 6.0, thumb_h, thumb_color);
        }
    }

    // Horizontal scrollbar (bottom edge)
    if has_h {
        let track_x = ox + data.row_header_width;
        let track_y = oy + sh - SCROLLBAR_SIZE;
        let track_w = sw - data.row_header_width - reserved_w;
        if track_w > 0.0 {
            fill_quad(window, track_x, track_y, track_w, SCROLLBAR_SIZE, track_bg);
            let thumb_w = ((track_w * (vw / content_w)).max(20.0)).min(track_w);
            let frac = if max_x > 0.0 { sx / max_x } else { 0.0 };
            let thumb_x = track_x + frac * (track_w - thumb_w);
            fill_quad(window, thumb_x, track_y + 3.0, thumb_w, SCROLLBAR_SIZE - 6.0, thumb_color);
        }
    }

    // Corner (bottom-right 20x20)
    if has_v && has_h {
        fill_quad(
            window,
            ox + sw - SCROLLBAR_SIZE,
            oy + sh - SCROLLBAR_SIZE,
            SCROLLBAR_SIZE,
            SCROLLBAR_SIZE,
            track_bg,
        );
    }
}

fn menu_hover_at(menu: &ContextMenu, x: f32, y: f32) -> Option<usize> {
    let fs = 14.0_f32;
    let item_h = fs + 8.0;
    let pad_x = 12.0;
    let min_w: f32 = 180.0;
    let cw = 7.6_f32;
    let mut max_label_w = 0.0_f32;
    for item in &menu.items {
        if let MenuItem::Action(a) = item {
            max_label_w = max_label_w.max(menu_label(*a).len() as f32 * cw);
        }
    }
    let w = min_w.max(max_label_w + pad_x * 2.0);
    let total_h = menu.items.len() as f32 * item_h + 8.0;
    let ax = pf(menu.anchor.x);
    let ay = pf(menu.anchor.y);
    if x < ax || x > ax + w || y < ay || y > ay + total_h {
        return None;
    }
    let rel_y = y - ay - 4.0;
    if rel_y < 0.0 {
        return None;
    }
    let idx = (rel_y / item_h) as usize;
    if idx >= menu.items.len() {
        return None;
    }
    let mut cur_row = 0;
    let mut action_idx = 0;
    for item in &menu.items {
        if cur_row == idx {
            return match item {
                MenuItem::Action(_) => Some(action_idx),
                MenuItem::Separator => None,
            };
        }
        cur_row += 1;
        if let MenuItem::Action(_) = item {
            action_idx += 1;
        }
    }
    None
}

struct PaintData {
    display_indices: Vec<usize>,
    selection: Selection,
    sort: Option<(usize, SortDirection)>,
    theme: GridTheme,
    columns: Vec<crate::data::Column>,
    rows: Vec<Vec<CellValue>>,
    filters: Vec<String>,
    scroll_offset: Point<Pixels>,
    row_height: f32,
    header_height: f32,
    row_header_width: f32,
    font_size: f32,
    char_width: f32,
    drag_rect: Option<(Point<Pixels>, Point<Pixels>)>,
    hover_hit: Option<HitResult>,
    context_menu: Option<ContextMenu>,
    filter_prompt: Option<FilterPrompt>,
}

impl PaintData {
    fn from_state(s: &GridState) -> Self {
        Self {
            display_indices: s.display_indices.clone(),
            selection: if s.is_dragging { Selection::None } else { s.selection.clone() },
            sort: s.sort,
            theme: s.theme.clone(),
            columns: s.data.columns.clone(),
            rows: s.data.rows.clone(),
            filters: s.filters.clone(),
            scroll_offset: s.scroll_handle.offset(),
            row_height: s.row_height,
            header_height: s.header_height,
            row_header_width: s.row_header_width,
            font_size: s.font_size,
            char_width: s.char_width,
            drag_rect: s.drag_screen_rect(),
            hover_hit: s.hover_hit,
            context_menu: s.context_menu.clone(),
            filter_prompt: s.filter_prompt.clone(),
        }
    }
}

struct StatusBarData {
    text: String,
    theme: GridTheme,
    font_size: f32,
}

fn hit_to_col_row(hit: Option<HitResult>) -> (Option<usize>, Option<usize>) {
    match hit {
        Some(HitResult::Cell(r, c)) => (Some(c), Some(r)),
        Some(HitResult::RowHeader(r)) => (None, Some(r)),
        Some(HitResult::ColumnHeader(c)) | Some(HitResult::SortButton(c)) => (Some(c), None),
        _ => (None, None),
    }
}

impl StatusBarData {
    fn from_state(s: &GridState) -> Self {
        let scroll = s.scroll_handle.offset();
        let click = s.click_pos;
        let click_scroll = s.scroll_at_click;
        let (click_col, click_row) = hit_to_col_row(s.click_hit);
        let cur = s.last_mouse_pos;
        let (hover_col, hover_row) = hit_to_col_row(s.hover_hit);

        let fmt_pt = |p: Option<Point<Pixels>>| -> String {
            match p {
                Some(p) => format!("({:.0}, {:.0})", pf(p.x), pf(p.y)),
                None => "—".to_string(),
            }
        };
        let fmt_scroll = |p: Option<Point<Pixels>>| -> String {
            match p {
                Some(p) => format!("({:.0}, {:.0})", pf(p.x), pf(p.y)),
                None => "—".to_string(),
            }
        };
        let fmt_cr = |c: Option<usize>, r: Option<usize>| -> String {
            match (c, r) {
                (Some(c), Some(r)) => format!("(col {}, row {})", c, r),
                (Some(c), None) => format!("(col {})", c),
                (None, Some(r)) => format!("(row {})", r),
                (None, None) => "—".to_string(),
            }
        };

        let text = format!(
            "Click: {}  Scroll@Click: {}  Cell: {}  |  Cur: {}  Scroll: {}  Over: {}",
            fmt_pt(click),
            fmt_scroll(click_scroll),
            fmt_cr(click_col, click_row),
            fmt_pt(cur),
            fmt_scroll(Some(scroll)),
            fmt_cr(hover_col, hover_row),
        );

        Self {
            text,
            theme: s.theme.clone(),
            font_size: s.font_size,
        }
    }
}

fn paint_status_bar(data: &StatusBarData, window: &mut Window, cx: &mut App, bounds: Bounds<Pixels>) {
    let ox = pf(bounds.origin.x);
    let oy = pf(bounds.origin.y);
    let sw = pf(bounds.size.width);
    let sh = pf(bounds.size.height);
    let theme = &data.theme;
    let fs = data.font_size;

    fill_quad(window, ox, oy, sw, sh, theme.header_bg);
    fill_quad(window, ox, oy, sw, 1.0, theme.grid_line);

    let text_system = window.text_system().clone();
    let font_size = px(fs);
    let line_height = px(fs * 1.2);
    let font = gpui::font("monospace");
    let run = TextRun {
        len: data.text.len(),
        color: theme.text_fg,
        font,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = text_system.shape_line(data.text.clone().into(), font_size, &[run], None);
    let _ = shaped.paint(point(px(ox + 8.0), px(oy + (sh - fs) * 0.5)), line_height, window, cx);
}

fn is_cell_sel(sel: &Selection, row: usize, col: usize) -> bool {
    match sel {
        Selection::Cell(r, c) => *r == row && *c == col,
        Selection::CellRange(r1, c1, r2, c2) => row >= *r1 && row <= *r2 && col >= *c1 && col <= *c2,
        Selection::Row(r) => *r == row,
        Selection::RowRange(r1, r2) => row >= *r1 && row <= *r2,
        Selection::Column(c) => *c == col,
        Selection::None => false,
    }
}

fn is_row_sel(sel: &Selection, row: usize) -> bool {
    match sel {
        Selection::Row(r) => *r == row,
        Selection::RowRange(r1, r2) => row >= *r1 && row <= *r2,
        _ => false,
    }
}

fn is_col_sel(sel: &Selection, col: usize) -> bool {
    matches!(sel, Selection::Column(c) if *c == col)
}

fn paint_grid(data: &PaintData, window: &mut Window, cx: &mut App, bounds: Bounds<Pixels>) {
    if matches!(data.hover_hit, Some(HitResult::ColumnBorder(_))) {
        window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
    }
    let ox = pf(bounds.origin.x);
    let oy = pf(bounds.origin.y);
    let sw = pf(bounds.size.width);
    let sh = pf(bounds.size.height);
    let sx = pf(data.scroll_offset.x);
    let sy = pf(data.scroll_offset.y);
    let row_h = data.row_height;
    let hdr_h = data.header_height;
    let rhw = data.row_header_width;
    let fs = data.font_size;
    let cw = data.char_width;
    let theme = &data.theme;

    let text_system = window.text_system().clone();
    let font_size = px(fs);
    let line_height = px(fs * 1.2);
    let font = gpui::font("monospace");
    let paint_txt = |win: &mut Window, cx: &mut App, text: &str, x: f32, y: f32, color: Hsla, max_w: Option<f32>| {
        let mk_run = |t: &str| TextRun {
            len: t.len(),
            color,
            font: font.clone(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = text_system.shape_line(text.to_string().into(), font_size, &[mk_run(text)], None);
        let shaped = match max_w {
            Some(mw) if mw <= 0.0 => return,
            Some(mw) if pf(shaped.width) > mw => {
                // with_len does NOT drop glyphs when painting, so re-shape a
                // truncated substring of the text to actually clip it.
                let byte_idx = shaped.index_for_x(px(mw)).unwrap_or(0);
                let truncated = &text[..byte_idx.min(text.len())];
                text_system.shape_line(truncated.to_string().into(), font_size, &[mk_run(truncated)], None)
            }
            _ => shaped,
        };
        let _ = shaped.paint(point(px(x), px(y)), line_height, win, cx);
    };

    fill_quad(window, ox, oy, sw, sh, theme.bg);
    fill_quad(window, ox, oy, rhw, sh, theme.row_header_bg);

    let data_y = hdr_h;
    let visible_h = sh - data_y;
    let first_row = ((sy / row_h) as usize).min(data.display_indices.len());
    let vis_rows = ((visible_h / row_h) as usize) + 1;
    let last_row = (first_row + vis_rows).min(data.display_indices.len());

    // Pass 1: data-area row backgrounds + cell contents.
    for dr in first_row..last_row {
        let y = oy + data_y + (dr as f32 * row_h) - sy;
        if y + row_h < oy + data_y || y > oy + sh {
            continue;
        }
        let row_idx = data.display_indices[dr];
        let row_sel = is_row_sel(&data.selection, dr);
        let alt = dr % 2 == 1;
        if row_sel {
            fill_quad(window, ox + rhw, y, sw - rhw, row_h, theme.selection_bg);
        } else if alt {
            fill_quad(window, ox + rhw, y, sw - rhw, row_h, theme.alt_row_bg);
        }

        let mut col_x = rhw - sx;
        for (ci, col) in data.columns.iter().enumerate() {
            let x = ox + col_x;
            let w = col.width;
            if x + w < ox + rhw || x > ox + sw {
                col_x += w;
                continue;
            }
            let cell_sel = is_cell_sel(&data.selection, dr, ci);
            if cell_sel {
                fill_quad(window, x, y, w, row_h, theme.selection_bg);
            }
            let cell = &data.rows[row_idx][ci];
            let (text, is_neg) = format_cell(cell, &col.col_type);
            let color = if is_neg { theme.negative_fg } else { theme.text_fg };
            let text_w = text.len() as f32 * cw;
            let tx = match col.col_type {
                crate::data::ColumnType::Text => x + 8.0,
                crate::data::ColumnType::Date { .. } => x + (w - text_w) * 0.5,
                _ => x + w - text_w - 8.0,
            };
            let ty = y + (row_h - fs) * 0.5;
            paint_txt(window, cx, &text, tx, ty, color, Some(w - 16.0));
            fill_quad(window, x + w, y, 1.0, row_h, theme.grid_line);
            col_x += w;
        }
        fill_quad(window, ox, y + row_h, sw, 1.0, theme.grid_line);
    }

    // Pass 2: row-header column painted on top so horizontally-scrolled cells
    // can't bleed over it.
    for dr in first_row..last_row {
        let y = oy + data_y + (dr as f32 * row_h) - sy;
        if y + row_h < oy + data_y || y > oy + sh {
            continue;
        }
        let row_sel = is_row_sel(&data.selection, dr);
        let alt = dr % 2 == 1;
        let rh_bg = if row_sel { theme.selection_bg } else if alt { theme.alt_row_bg } else { theme.row_header_bg };
        fill_quad(window, ox, y, rhw, row_h, rh_bg);
        paint_txt(window, cx, &(dr + 1).to_string(), ox + 6.0, y + (row_h - fs) * 0.5, theme.header_fg, None);
        fill_quad(window, ox, y + row_h, rhw, 1.0, theme.grid_line);
    }

    // Pass 3: column header painted last so vertically-scrolled rows can't bleed
    // over it.
    fill_quad(window, ox, oy, sw, hdr_h, theme.header_bg);
    let mut col_x = rhw - sx;
    for (ci, col) in data.columns.iter().enumerate() {
        let x = ox + col_x;
        let w = col.width;
        if x + w < ox + rhw || x > ox + sw {
            col_x += w;
            continue;
        }
        if is_col_sel(&data.selection, ci) {
            fill_quad(window, x, oy, w, hdr_h, theme.selection_bg);
        }
        paint_txt(window, cx, &col.name, x + 8.0, oy + (hdr_h - fs) * 0.5, theme.header_fg, Some(w - 28.0));
        let btn_w = 20.0;
        let btn_x = x + w - btn_w;
        let is_sorted = matches!(data.sort, Some((sc, _)) if sc == ci);
        let btn_bg = if is_sorted { hsla(0.58, 0.30, 0.70, 0.50) } else { hsla(0.0, 0.0, 0.88, 1.0) };
        fill_quad(window, btn_x, oy + 4.0, btn_w, hdr_h - 8.0, btn_bg);
        fill_quad(window, btn_x, oy + 4.0, 1.0, hdr_h - 8.0, theme.grid_line);
        let (ind, ind_color) = match data.sort {
            Some((sc, SortDirection::Ascending)) if sc == ci => ("^", theme.sort_indicator),
            Some((sc, SortDirection::Descending)) if sc == ci => ("v", theme.sort_indicator),
            _ => ("-", theme.header_fg),
        };
        paint_txt(window, cx, ind, btn_x + (btn_w - cw) * 0.5, oy + (hdr_h - fs) * 0.5, ind_color, None);
        // Filter indicator: show a small marker on header if this column has a filter
        if !data.filters[ci].is_empty() {
            let marker_w = 4.0;
            let marker_x = btn_x - marker_w - 2.0;
            fill_quad(window, marker_x, oy + (hdr_h - 12.0) * 0.5, marker_w, 12.0, theme.sort_indicator);
        }
        fill_quad(window, x + w, oy, 1.0, hdr_h, theme.grid_line);
        col_x += w;
    }
    // Top-left corner (above the row headers) painted last to cover any bleed.
    fill_quad(window, ox, oy, rhw, hdr_h, theme.row_header_bg);

    fill_quad(window, ox, oy + hdr_h, sw, 1.0, theme.grid_line);
    fill_quad(window, ox + rhw, oy, 1.0, sh, theme.grid_line);

    if let Some((start, current)) = data.drag_rect {
        let sx0 = pf(start.x);
        let sy0 = pf(start.y);
        let sx1 = pf(current.x);
        let sy1 = pf(current.y);
        let rx = sx0.min(sx1);
        let ry = sy0.min(sy1);
        let rw = (sx1 - sx0).abs();
        let rh = (sy1 - sy0).abs();
        let drag_fill = hsla(0.58, 0.50, 0.50, 0.20);
        let drag_border = hsla(0.58, 0.60, 0.45, 0.90);
        window.paint_quad(PaintQuad {
            bounds: Bounds {
                origin: point(px(rx), px(ry)),
                size: size(px(rw), px(rh)),
            },
            background: drag_fill.into(),
            border_color: drag_border,
            border_widths: Default::default(),
            corner_radii: Default::default(),
            border_style: Default::default(),
        });
    }

    // Scrollbars
    paint_scrollbars(data, window, ox, oy, sw, sh, theme);

    // Context menu (right-clicked column header)
    if let Some(menu) = &data.context_menu {
        paint_context_menu(
            window,
            cx,
            menu,
            ox,
            oy,
            sw,
            sh,
            fs,
            cw,
            theme,
            &*text_system,
            font_size,
            line_height,
        );
    }

    // Filter input popup
    if let Some(prompt) = &data.filter_prompt {
        paint_filter_prompt(
            window,
            cx,
            prompt,
            ox,
            oy,
            sw,
            sh,
            fs,
            theme,
            &*text_system,
            font_size,
            line_height,
        );
    }
}

fn menu_label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::SelectColumn => "Select column",
        MenuAction::CopyColumn => "Copy column",
        MenuAction::CopyColumnWithHeaders => "Copy column with headers",
        MenuAction::SortAscending => "Sort Ascending",
        MenuAction::SortDescending => "Sort Descending",
        MenuAction::ClearSort => "Clear sort",
        MenuAction::FilterPrompt => "Filter...",
        MenuAction::ClearFilter => "Clear filter",
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_context_menu(
    window: &mut Window,
    cx: &mut App,
    menu: &ContextMenu,
    ox: f32,
    oy: f32,
    sw: f32,
    sh: f32,
    fs: f32,
    cw: f32,
    theme: &GridTheme,
    text_system: &WindowTextSystem,
    font_size: Pixels,
    line_height: Pixels,
) {
    let item_h = fs + 8.0;
    let pad_x = 12.0;
    let min_w: f32 = 180.0;
    let mut max_label_w = 0.0_f32;
    for item in &menu.items {
        if let MenuItem::Action(a) = item {
            let label = menu_label(*a);
            max_label_w = max_label_w.max(label.len() as f32 * cw);
        }
    }
    let menu_w = min_w.max(max_label_w + pad_x * 2.0);
    let total_h = menu.items.len() as f32 * item_h + 8.0;
    let ax = pf(menu.anchor.x);
    let ay = pf(menu.anchor.y);
    let mut mx = ax;
    let mut my = ay;
    if mx + menu_w > ox + sw {
        mx = ox + sw - menu_w - 4.0;
    }
    if mx < ox + 2.0 {
        mx = ox + 2.0;
    }
    if my + total_h > oy + sh {
        my = oy + sh - total_h - 4.0;
    }
    if my < oy + 2.0 {
        my = oy + 2.0;
    }
    let bg = hsla(0.0, 0.0, 1.0, 1.0);
    let border = theme.grid_line;
    fill_quad(window, mx, my, menu_w, total_h, bg);
    // top/left/bottom/right border
    fill_quad(window, mx, my, menu_w, 1.0, border);
    fill_quad(window, mx, my + total_h - 1.0, menu_w, 1.0, border);
    fill_quad(window, mx, my, 1.0, total_h, border);
    fill_quad(window, mx + menu_w - 1.0, my, 1.0, total_h, border);

    let font = gpui::font("monospace");
    let mk_run = |t: &str, color: Hsla| TextRun {
        len: t.len(),
        color,
        font: font.clone(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut cur = 0;
    for (i, item) in menu.items.iter().enumerate() {
        let iy = my + 4.0 + i as f32 * item_h;
        match item {
            MenuItem::Separator => {
                let sep_y = iy + item_h * 0.5;
                fill_quad(window, mx + 4.0, sep_y, menu_w - 8.0, 1.0, theme.grid_line);
            }
            MenuItem::Action(action) => {
                let hovered = menu.hovered == Some(cur);
                if hovered {
                    fill_quad(window, mx + 2.0, iy, menu_w - 4.0, item_h, theme.selection_bg);
                }
                let label = menu_label(*action);
                let color = theme.text_fg;
                let run = mk_run(label, color);
                let shaped = text_system.shape_line(label.into(), font_size, &[run], None);
                let _ = shaped.paint(
                    point(px(mx + pad_x), px(iy + (item_h - fs) * 0.5)),
                    line_height,
                    window,
                    cx,
                );
                cur += 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_filter_prompt(
    window: &mut Window,
    cx: &mut App,
    prompt: &FilterPrompt,
    ox: f32,
    oy: f32,
    sw: f32,
    sh: f32,
    fs: f32,
    theme: &GridTheme,
    text_system: &WindowTextSystem,
    font_size: Pixels,
    line_height: Pixels,
) {
    let pad_x = 8.0;
    let pad_y = 6.0;
    let min_w: f32 = 220.0;
    let text = if prompt.input.is_empty() {
        "Type to filter...".to_string()
    } else {
        prompt.input.clone()
    };
    let preview_color = if prompt.input.is_empty() {
        theme.grid_line
    } else {
        theme.text_fg
    };
    let label_text = format!("Filter: {}", text);
    let label_w = label_text.len() as f32 * (fs * 0.6);
    let w = min_w.max(label_w + pad_x * 2.0);
    let h = fs + pad_y * 2.0;
    let ax = pf(prompt.anchor.x);
    let ay = pf(prompt.anchor.y);
    let mut mx = ax;
    let mut my = ay;
    if mx + w > ox + sw {
        mx = ox + sw - w - 4.0;
    }
    if my + h > oy + sh {
        my = oy + sh - h - 4.0;
    }
    let bg = hsla(0.0, 0.0, 1.0, 1.0);
    let border = theme.grid_line;
    fill_quad(window, mx, my, w, h, bg);
    fill_quad(window, mx, my, w, 1.0, border);
    fill_quad(window, mx, my + h - 1.0, w, 1.0, border);
    fill_quad(window, mx, my, 1.0, h, border);
    fill_quad(window, mx + w - 1.0, my, 1.0, h, border);

    let font = gpui::font("monospace");
    let run = TextRun {
        len: label_text.len(),
        color: preview_color,
        font: font.clone(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = text_system.shape_line(label_text.into(), font_size, &[run], None);
    let _ = shaped.paint(
        point(px(mx + pad_x), px(my + pad_y)),
        line_height,
        window,
        cx,
    );

    // Cursor
    if !prompt.input.is_empty() {
        let prefix = "Filter: ".to_string();
        let prefix_run = TextRun {
            len: prefix.len(),
            color: theme.text_fg,
            font: font.clone(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let prefix_shaped = text_system.shape_line(prefix.into(), font_size, &[prefix_run], None);
        let before_cursor = &prompt.input[..prompt.cursor.min(prompt.input.len())];
        let before_run = TextRun {
            len: before_cursor.len(),
            color: theme.text_fg,
            font: font.clone(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let before_shaped = text_system.shape_line(before_cursor.to_string().into(), font_size, &[before_run], None);
        let cur_x = mx + pad_x + f32::from(prefix_shaped.width) + f32::from(before_shaped.width);
        fill_quad(window, cur_x, my + pad_y, 1.0, fs + 2.0, theme.text_fg);
    } else {
        let cur_x = mx + pad_x + "Filter: ".len() as f32 * (fs * 0.6);
        fill_quad(window, cur_x, my + pad_y, 1.0, fs + 2.0, theme.text_fg);
    }
}

pub struct GridView {
    pub state: Entity<GridState>,
}

impl GridView {
    pub fn new(state: Entity<GridState>) -> Self {
        Self { state }
    }
}

impl Focusable for GridView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_canvas = self.state.clone();
        let state_status = self.state.clone();
        let state_mouse = self.state.clone();
        let state_move = self.state.clone();
        let state_up = self.state.clone();
        let state_scroll = self.state.clone();
        let state_key = self.state.clone();
        let state_edge = self.state.clone();
        let state_right = self.state.clone();
        let bg = self.state.read(cx).theme.bg;
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let status_h = self.state.read(cx).status_bar_height;

        // Process any pending menu action from a previous mouse-down on a
        // menu item (needs App access for clipboard).
        if let Some((action, col)) = self.state.read(cx).pending_action {
            self.state.update(cx, |s, cx| {
                s.execute_action(action, col, cx);
                s.pending_action = None;
            });
            // Don't return; allow notify to drive repaint.
        }

        if !self.state.read(cx).edge_scroll_started {
            self.state.update(cx, |s, _cx| {
                s.edge_scroll_started = true;
            });
            cx.spawn(async move |_weak, cx| {
                loop {
                    smol::Timer::after(Duration::from_millis(16)).await;
                    let _ = cx.update(|cx| {
                        let scrolled = state_edge.update(cx, |s, _cx| s.apply_edge_scroll());
                        if scrolled {
                            state_edge.update(cx, |_s, cx| {
                                cx.notify();
                            });
                        }
                    });
                }
            })
            .detach();
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .track_focus(&focus_handle)
            .child(
                canvas(
                    move |bounds, _window, cx| -> PaintData {
                        state_canvas.update(cx, |s, cx| {
                            s.bounds = bounds;
                            cx.notify();
                        });
                        let s = state_canvas.read(cx);
                        PaintData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_grid(&data, window, cx, bounds);
                    },
                )
                .flex_1(),
            )
            .child(
                canvas(
                    move |_bounds, _window, cx| -> StatusBarData {
                        let s = state_status.read(cx);
                        StatusBarData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_status_bar(&data, window, cx, bounds);
                    },
                )
                .h(px(status_h)),
            )
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _window, cx| {
                    state_mouse.update(cx, |s, cx| {
                        // Check if click hits the context menu overlay first
                        if let Some(menu) = s.context_menu.clone() {
                            if let Some((mx, my, mw, mh)) = s.menu_bounds(&menu) {
                                let x = pf(event.position.x) - pf(s.bounds.origin.x);
                                let y = pf(event.position.y) - pf(s.bounds.origin.y);
                                if x >= mx && x <= mx + mw && y >= my && y <= my + mh {
                                    if let Some(action_idx) = menu_hover_at(&menu, x, y) {
                                        let mut cur = 0;
                                        for item in &menu.items {
                                            if let MenuItem::Action(a) = item {
                                                if cur == action_idx {
                                                    s.pending_action = Some((*a, menu.col));
                                                    s.context_menu = None;
                                                    cx.notify();
                                                    return;
                                                }
                                                cur += 1;
                                            }
                                        }
                                    }
                                } else {
                                    // Click outside menu: dismiss it
                                    s.context_menu = None;
                                    s.filter_prompt = None;
                                }
                            }
                        }
                        s.handle_mouse_down(event.position, event.modifiers.shift);
                        cx.notify();
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                state_move.update(cx, |s, cx| {
                    s.handle_mouse_move(event.position, event.pressed_button);
                    cx.notify();
                });
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    state_up.update(cx, |s, cx| {
                        s.handle_mouse_up();
                        cx.notify();
                    });
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, _window, cx| {
                    state_right.update(cx, |s, cx| {
                        let pos = event.position;
                        let hit = s.hit_test(pos);
                        match hit {
                            HitResult::ColumnHeader(col) | HitResult::SortButton(col) => {
                                s.open_context_menu(col, pos);
                            }
                            _ => {
                                s.context_menu = None;
                                s.filter_prompt = None;
                            }
                        }
                        cx.notify();
                    });
                },
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                state_scroll.update(cx, |s, cx| {
                    let line_h = px(s.row_height);
                    let delta = event.delta.pixel_delta(line_h);
                    let scroll = s.scroll_handle.offset();
                    let (mx, my) = s.max_scroll();
                    let new_y = (pf(scroll.y) - pf(delta.y)).clamp(0.0, my);
                    let new_x = (pf(scroll.x) - pf(delta.x)).clamp(0.0, mx);
                    s.scroll_handle.set_offset(point(px(new_x), px(new_y)));
                    if s.drag_start.is_some() {
                        s.handle_scroll_drag();
                    }
                    cx.notify();
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                if event.keystroke.modifiers.platform && event.keystroke.key == "q" {
                    cx.quit();
                    return;
                }
                state_key.update(cx, |s, cx| {
                    s.handle_key(&event.keystroke);
                    cx.notify();
                });
            })
    }
}
