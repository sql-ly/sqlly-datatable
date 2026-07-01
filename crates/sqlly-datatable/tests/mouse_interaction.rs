//! Reproduction tests for mouse-driven selection and the right-click context
//! menu, exercised through GPUI's real event dispatch (window + hitbox +
//! listeners) via `test-support`. These assert the widget is self-contained:
//! a left click selects a cell, and a right click on a column header opens the
//! built-in menu — no host wiring required.

#![allow(clippy::expect_used)]

use gpui::{
    div, point, px, AppContext, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, Styled, TestAppContext, Window,
};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, ContextMenuItem, ContextMenuProvider, ContextMenuRequest,
    ContextMenuTarget, GridConfig, GridData, Selection, SqllyDataTable,
};

/// A wrapper view that renders the grid inset from the window's top-left by a
/// fixed padding, forcing the grid's painted `bounds.origin` to be NON-ZERO.
/// This reproduces the real app, where the results grid is nested deep in the
/// layout (origin ~ (350, 1800)) rather than being the window root (origin 0,
/// as in every other test here). Pointer coordinates arrive from GPUI in
/// absolute window space; the widget must translate them into its own frame.
struct Harness {
    grid: Entity<SqllyDataTable>,
}

/// Padding applied to the grid inside the harness. Kept well within the
/// 1920x1080 test window so the grid still has room for all rows/columns.
const PAD_LEFT: f32 = 120.0;
const PAD_TOP: f32 = 200.0;

impl Render for Harness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .pl(px(PAD_LEFT))
            .pt(px(PAD_TOP))
            .child(self.grid.clone())
    }
}

/// Minimal provider mirroring how the app (`ResultGridMenuProvider`) drives the
/// right-click menu: standard items for a column header, a custom action for a
/// cell/row. Exercises the provider branch of the widget's right-mouse handler.
struct TestProvider;

impl ContextMenuProvider for TestProvider {
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
        match request.target {
            ContextMenuTarget::ColumnHeader { .. } | ContextMenuTarget::SortButton { .. } => {
                ContextMenuItem::standard_column_header_items()
            }
            ContextMenuTarget::Cell { .. } | ContextMenuTarget::RowHeader { .. } => {
                vec![ContextMenuItem::action("copy", "Copy")]
            }
        }
    }
}

fn sample() -> GridData {
    GridData::new(
        vec![
            Column {
                name: "id".into(),
                kind: ColumnKind::Integer,
                width: 100.0,
            },
            Column {
                name: "name".into(),
                kind: ColumnKind::Text,
                width: 200.0,
            },
        ],
        (0..30)
            .map(|i| vec![CellValue::Integer(i), CellValue::Text(format!("row{i}"))])
            .collect(),
    )
    .expect("rectangular data")
}

#[gpui::test]
fn left_click_selects_single_cell(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    // Read the geometry the widget actually painted into.
    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Click squarely inside row 0, column 0 (first data row under the header,
    // x well within the 100px-wide first column).
    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 0),
        "left click inside first data cell should select Cell(0,0), got {selection:?}"
    );
}

#[gpui::test]
fn right_click_on_column_header_opens_menu(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height)
    });

    // Right-click inside the first column's header cell.
    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Right, Modifiers::none());
    cx.run_until_parked();

    let has_menu = view.read_with(cx, |v, cx| v.state.read(cx).context_menu.is_some());
    assert!(
        has_menu,
        "right click on a column header should open the built-in context menu"
    );
}

#[gpui::test]
fn right_click_column_header_with_provider_opens_menu(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .context_menu_provider(TestProvider)
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height)
    });

    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Right, Modifiers::none());
    cx.run_until_parked();

    let has_menu = view.read_with(cx, |v, cx| v.state.read(cx).context_menu.is_some());
    assert!(
        has_menu,
        "right click on a column header WITH a provider should open the provider menu"
    );
}

#[gpui::test]
fn right_click_cell_with_provider_opens_menu(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .context_menu_provider(TestProvider)
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Right, Modifiers::none());
    cx.run_until_parked();

    let has_menu = view.read_with(cx, |v, cx| v.state.read(cx).context_menu.is_some());
    assert!(
        has_menu,
        "right click on a cell WITH a provider should open the provider menu"
    );
}

/// After clicking into the grid, the keyboard must drive selection: this is
/// the "select ranges of cells" / select-all / copy path. It only works if the
/// widget participates in the focus tree (track_focus + focus-on-mouse-down).
/// Without that wiring `.on_key_down` never fires. Regression guard.
#[gpui::test]
fn keyboard_select_all_after_click(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Click a data cell first (this should focus the grid).
    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    // Cmd+A → select all.
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert!(
        matches!(selection, Selection::CellRange(0, 0, r, c) if r >= 29 && c >= 1),
        "cmd-a should select the whole grid, got {selection:?}"
    );
}

/// Shift+Arrow extends a cell selection into a range (the primary keyboard way
/// to select a range of cells). Requires the same focus wiring.
#[gpui::test]
fn keyboard_shift_arrow_extends_range(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    cx.simulate_keystrokes("shift-down shift-down");
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert!(
        matches!(selection, Selection::CellRange(0, 0, 2, 0)),
        "shift-down twice should extend to CellRange(0,0,2,0), got {selection:?}"
    );
}

/// Shift+click extends a rectangular cell range from the first-clicked cell
/// (anchor) to the shift-clicked cell (extent) — mirrors the Swift grid's
/// drag/shift range selection.
#[gpui::test]
fn shift_click_selects_cell_range(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Anchor click at row 0, col 0.
    let x0 = f32::from(origin.x) + 90.0;
    let y0 = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_click(point(px(x0), px(y0)), Modifiers::none());
    cx.run_until_parked();

    // Shift-click at row 2, col 1 (x inside the 200px-wide second column,
    // which starts after the 100px first column + 50px row-header gutter).
    let x1 = f32::from(origin.x) + 50.0 + 100.0 + 90.0;
    let y1 = f32::from(origin.y) + header_h + row_h * 2.5;
    cx.simulate_mouse_down(point(px(x1), px(y1)), MouseButton::Left, Modifiers::shift());
    cx.simulate_mouse_up(point(px(x1), px(y1)), MouseButton::Left, Modifiers::shift());
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::CellRange(0, 0, 2, 1),
        "shift-click should extend to CellRange(0,0,2,1), got {selection:?}"
    );
}

/// Clicking a column header selects the whole column.
#[gpui::test]
fn click_column_header_selects_column(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height)
    });

    // Click the second column's header (after gutter + first column).
    let x = f32::from(origin.x) + 50.0 + 100.0 + 90.0;
    let y = f32::from(origin.y) + header_h * 0.5;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Column(1),
        "clicking a column header should select that column, got {selection:?}"
    );
}

/// Clicking the row-header gutter selects the whole row.
#[gpui::test]
fn click_row_header_selects_row(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Click inside the 50px row-header gutter (x < 50), at row 2.
    let x = f32::from(origin.x) + 25.0;
    let y = f32::from(origin.y) + header_h + row_h * 2.5;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Row(2),
        "clicking the row-header gutter should select that row, got {selection:?}"
    );
}

// ----------------------------------------------------------------------------
// Coordinate-frame tests: the grid may be nested anywhere in the window, so all
// stored pointer positions (click_pos, drag_start, ...) MUST be expressed
// relative to the grid's own top-left, not in absolute window coordinates.
// These use the `Harness` wrapper to force a non-zero grid origin.
// ----------------------------------------------------------------------------

/// The reported bug: clicking 8px from the grid's top-left must record a
/// click position of (8, 8) — grid-relative — NOT the absolute window/monitor
/// coordinate (~128, 208). This is exactly what the status bar prints as
/// `Click: (x, y)`.
#[gpui::test]
fn click_position_is_relative_to_grid_container(cx: &mut TestAppContext) {
    let (harness, cx) = cx.add_window_view(|_window, cx| {
        let grid = cx.new(|cx| {
            SqllyDataTable::builder(sample())
                .config(GridConfig::default())
                .build(cx)
        });
        Harness { grid }
    });
    cx.run_until_parked();

    let grid = harness.read_with(cx, |h, _cx| h.grid.clone());
    let origin = grid.read_with(cx, |g, cx| g.state.read(cx).bounds.origin);

    // Sanity: the grid really is offset from the window origin.
    assert!(
        f32::from(origin.x) >= PAD_LEFT && f32::from(origin.y) >= PAD_TOP,
        "grid should be inset; origin was {origin:?}"
    );

    // Click 8px in from the grid's own top-left corner.
    let x = f32::from(origin.x) + 8.0;
    let y = f32::from(origin.y) + 8.0;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    let click_pos = grid.read_with(cx, |g, cx| g.state.read(cx).click_pos);
    assert_eq!(
        click_pos,
        Some(point(px(8.0), px(8.0))),
        "click 8px from the grid's top-left should record grid-relative (8,8), got {click_pos:?}"
    );
}

/// Cell hit-testing must remain correct at a non-zero origin: clicking inside
/// the first data cell selects Cell(0,0) even when the grid is inset.
#[gpui::test]
fn cell_selection_correct_with_nonzero_origin(cx: &mut TestAppContext) {
    let (harness, cx) = cx.add_window_view(|_window, cx| {
        let grid = cx.new(|cx| {
            SqllyDataTable::builder(sample())
                .config(GridConfig::default())
                .build(cx)
        });
        Harness { grid }
    });
    cx.run_until_parked();

    let grid = harness.read_with(cx, |h, _cx| h.grid.clone());
    let (origin, header_h, row_h) = grid.read_with(cx, |g, cx| {
        let s = g.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Row 0, column 0: past the 50px row-header gutter, first data row.
    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
    cx.run_until_parked();

    let selection = grid.read_with(cx, |g, cx| g.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 0),
        "cell hit-test must map correctly at a non-zero grid origin, got {selection:?}"
    );
}

/// A drag begun on a cell must store its start position in the grid's own
/// frame, so drag-range math stays correct regardless of nesting.
#[gpui::test]
fn drag_start_is_relative_to_grid_container(cx: &mut TestAppContext) {
    let (harness, cx) = cx.add_window_view(|_window, cx| {
        let grid = cx.new(|cx| {
            SqllyDataTable::builder(sample())
                .config(GridConfig::default())
                .build(cx)
        });
        Harness { grid }
    });
    cx.run_until_parked();

    let grid = harness.read_with(cx, |h, _cx| h.grid.clone());
    let (origin, header_h, row_h) = grid.read_with(cx, |g, cx| {
        let s = g.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Press (mouse-down, no release) inside row 0 / col 0.
    let rel_x = 90.0;
    let rel_y = header_h + row_h * 0.5;
    let x = f32::from(origin.x) + rel_x;
    let y = f32::from(origin.y) + rel_y;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    let drag_start = grid.read_with(cx, |g, cx| g.state.read(cx).drag_start);
    assert_eq!(
        drag_start,
        Some(point(px(rel_x), px(rel_y))),
        "drag_start should be grid-relative ({rel_x},{rel_y}), got {drag_start:?}"
    );
}
