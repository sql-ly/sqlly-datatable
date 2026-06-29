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
