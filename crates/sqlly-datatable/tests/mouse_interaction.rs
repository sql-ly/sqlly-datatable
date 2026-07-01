//! Reproduction tests for mouse-driven selection and the right-click context
//! menu, exercised through GPUI's real event dispatch (window + hitbox +
//! listeners) via `test-support`. These assert the widget is self-contained:
//! a left click selects a cell, and a right click on a column header opens the
//! built-in menu — no host wiring required.

#![allow(clippy::expect_used)]

use gpui::{point, px, Modifiers, MouseButton, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, ContextMenuItem, ContextMenuProvider, ContextMenuRequest,
    ContextMenuTarget, GridConfig, GridData, Selection, SqllyDataTable,
};

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
