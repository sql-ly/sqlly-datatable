//! Pure selection / hit-testing types and helpers. Kept separate from the
//! stateful widget so paint, input, and copy code can all use the same
//! predicates without circular dependencies.

use gpui::Point;

/// What is currently selected. Stores display-row indices; after a sort the
/// "same row" might live at a different position, so callers needing stable
/// identities should track source rows separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    Cell(usize, usize),
    Row(usize),
    Column(usize),
    /// Inclusive `(r1, c1)` to `(r2, c2)`. Always `r1 <= r2 && c1 <= c2`.
    CellRange(usize, usize, usize, usize),
    /// Inclusive `[r1, r2]`.
    RowRange(usize, usize),
}

impl Selection {
    /// Returns `(min_row, min_col, max_row, max_col)`. `Selection::None`
    /// returns `None`.
    #[must_use]
    pub fn normalized_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        match *self {
            Selection::None => None,
            Selection::Cell(r, c) => Some((r, c, r, c)),
            Selection::Row(r) => Some((r, 0, r, usize::MAX)),
            Selection::Column(c) => Some((0, c, usize::MAX, c)),
            Selection::CellRange(r1, c1, r2, c2) => {
                Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
            }
            Selection::RowRange(r1, r2) => Some((r1.min(r2), 0, r1.max(r2), usize::MAX)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// What a mouse hit-test resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

/// `true` if the selection visually highlights the given cell.
#[must_use]
pub fn is_cell_selected(sel: &Selection, row: usize, col: usize) -> bool {
    match *sel {
        Selection::None => false,
        Selection::Cell(r, c) => r == row && c == col,
        Selection::CellRange(r1, c1, r2, c2) => {
            let (rmin, cmin, rmax, cmax) = (r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2));
            row >= rmin && row <= rmax && col >= cmin && col <= cmax
        }
        Selection::Row(r) => r == row,
        Selection::RowRange(r1, r2) => {
            let (rmin, rmax) = (r1.min(r2), r1.max(r2));
            row >= rmin && row <= rmax
        }
        Selection::Column(c) => c == col,
    }
}

#[must_use]
pub fn is_row_selected(sel: &Selection, row: usize) -> bool {
    match *sel {
        Selection::Row(r) => r == row,
        Selection::RowRange(r1, r2) => {
            let (rmin, rmax) = (r1.min(r2), r1.max(r2));
            row >= rmin && row <= rmax
        }
        _ => false,
    }
}

#[must_use]
pub fn is_column_selected(sel: &Selection, col: usize) -> bool {
    matches!(*sel, Selection::Column(c) if c == col)
}

/// Convert a screen pointer (in window coordinates) to its corresponding
/// content-space (i.e. bounds-relative plus scroll offset) coordinates.
#[must_use]
pub fn screen_to_content(
    pos: Point<gpui::Pixels>,
    bounds_origin: Point<gpui::Pixels>,
    scroll: Point<gpui::Pixels>,
) -> (f32, f32) {
    let sx: f32 = scroll.x.into();
    let sy: f32 = scroll.y.into();
    let ox: f32 = bounds_origin.x.into();
    let oy: f32 = bounds_origin.y.into();
    let px: f32 = pos.x.into();
    let py: f32 = pos.y.into();
    (px - ox + sx, py - oy + sy)
}

/// Translate an absolute window-space pointer into the grid's OWN coordinate
/// frame by subtracting the grid's painted `bounds.origin`. Every pointer
/// value the widget hands to [`GridState`] is normalized through this at the
/// event boundary, so all stored positions (`click_pos`, `drag_start`,
/// `last_mouse_pos`, menu/prompt anchors) live in one consistent grid-relative
/// frame regardless of where the widget is nested in the window. A grid at
/// window origin (as in the sample app and older tests) is the identity case.
#[must_use]
pub fn to_grid_relative(
    pos: Point<gpui::Pixels>,
    bounds_origin: Point<gpui::Pixels>,
) -> Point<gpui::Pixels> {
    Point {
        x: pos.x - bounds_origin.x,
        y: pos.y - bounds_origin.y,
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
    use gpui::{px, Pixels};

    fn p(x: f32, y: f32) -> Point<Pixels> {
        Point { x: px(x), y: px(y) }
    }

    #[test]
    fn normalized_bounds_none_is_none() {
        assert_eq!(Selection::None.normalized_bounds(), None);
    }

    #[test]
    fn normalized_bounds_cell_folds_to_single_point() {
        assert_eq!(
            Selection::Cell(2, 3).normalized_bounds(),
            Some((2, 3, 2, 3))
        );
    }

    #[test]
    fn normalized_bounds_row_spans_all_columns() {
        let (r0, c0, r1, c1) = Selection::Row(4).normalized_bounds().unwrap();
        assert_eq!(r0, 4);
        assert_eq!(r1, 4);
        assert_eq!(c0, 0);
        assert_eq!(c1, usize::MAX);
    }

    #[test]
    fn normalized_bounds_column_spans_all_rows() {
        let (r0, c0, r1, c1) = Selection::Column(5).normalized_bounds().unwrap();
        assert_eq!(r0, 0);
        assert_eq!(r1, usize::MAX);
        assert_eq!(c0, 5);
        assert_eq!(c1, 5);
    }

    #[test]
    fn normalized_bounds_cell_range_handles_reversed() {
        assert_eq!(
            Selection::CellRange(5, 4, 1, 2).normalized_bounds(),
            Some((1, 2, 5, 4)),
        );
    }

    #[test]
    fn normalized_bounds_row_range_handles_reversed() {
        let (r0, _c0, r1, c1) = Selection::RowRange(9, 3).normalized_bounds().unwrap();
        assert_eq!(r0, 3);
        assert_eq!(r1, 9);
        assert_eq!(c1, usize::MAX);
    }

    #[test]
    fn is_cell_selected_for_all_variants() {
        assert!(!is_cell_selected(&Selection::None, 0, 0));
        assert!(is_cell_selected(&Selection::Cell(2, 3), 2, 3));
        assert!(!is_cell_selected(&Selection::Cell(2, 3), 3, 2));

        assert!(is_cell_selected(&Selection::CellRange(1, 1, 3, 3), 2, 2));
        assert!(is_cell_selected(&Selection::CellRange(3, 3, 1, 1), 2, 2));
        assert!(!is_cell_selected(&Selection::CellRange(1, 1, 3, 3), 4, 4));

        assert!(is_cell_selected(&Selection::Row(2), 2, 0));
        assert!(is_cell_selected(&Selection::Row(2), 2, 99));
        assert!(!is_cell_selected(&Selection::Row(2), 3, 0));

        assert!(is_cell_selected(&Selection::RowRange(1, 3), 2, 5));
        assert!(!is_cell_selected(&Selection::RowRange(1, 3), 4, 5));
        assert!(is_cell_selected(&Selection::RowRange(3, 1), 2, 0));

        assert!(is_cell_selected(&Selection::Column(5), 0, 5));
        assert!(is_cell_selected(&Selection::Column(5), 99, 5));
        assert!(!is_cell_selected(&Selection::Column(5), 0, 4));
    }

    #[test]
    fn is_row_selected_only_for_row_and_row_range() {
        assert!(is_row_selected(&Selection::Row(3), 3));
        assert!(!is_row_selected(&Selection::Row(3), 4));
        assert!(is_row_selected(&Selection::RowRange(2, 5), 4));
        assert!(is_row_selected(&Selection::RowRange(5, 2), 4));
        assert!(!is_row_selected(&Selection::RowRange(2, 5), 6));

        assert!(!is_row_selected(&Selection::Cell(1, 2), 1));
        assert!(!is_row_selected(&Selection::CellRange(0, 0, 9, 9), 5));
        assert!(!is_row_selected(&Selection::Column(0), 5));
        assert!(!is_row_selected(&Selection::None, 0));
    }

    #[test]
    fn is_column_selected_only_for_column_variant() {
        assert!(is_column_selected(&Selection::Column(7), 7));
        assert!(!is_column_selected(&Selection::Column(7), 8));
        assert!(!is_column_selected(&Selection::Row(0), 0));
        assert!(!is_column_selected(&Selection::None, 0));
        assert!(!is_column_selected(&Selection::CellRange(0, 2, 9, 2), 2));
    }

    #[test]
    fn screen_to_content_applies_origin_and_scroll() {
        let pos = p(50.0, 60.0);
        let origin = p(10.0, 20.0);
        let scroll = p(5.0, 7.0);
        let (cx, cy) = screen_to_content(pos, origin, scroll);
        assert_eq!(cx, 45.0);
        assert_eq!(cy, 47.0);
    }

    #[test]
    fn screen_to_content_no_offset() {
        let (cx, cy) = screen_to_content(p(0.0, 0.0), p(0.0, 0.0), p(0.0, 0.0));
        assert_eq!(cx, 0.0);
        assert_eq!(cy, 0.0);
    }

    #[test]
    fn screen_to_content_handles_negative_above_origin() {
        // Above-origin and negative-axis positions happen during drag-scroll
        // and should not panic.
        let (_, _) = screen_to_content(p(-30.0, -30.0), p(0.0, 0.0), p(0.0, 0.0));
    }
}
