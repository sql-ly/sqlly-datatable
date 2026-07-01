//! Reproduce the *app's* nesting around the grid to see whether nesting the
//! `SqllyDataTable` entity inside sized / flex / overflow_hidden / scroll-wheel
//! wrappers (as `sqlly-gpui`'s ResultsView + AppView do) breaks mouse routing
//! to the grid. If a click here fails to select, the nesting is the culprit.

use gpui::{
    div, point, px, AppContext, Context, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, ScrollWheelEvent, Styled, TestAppContext, Window,
};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, Selection, SqllyDataTable,
};

fn sample() -> GridData {
    GridData::new(
        vec![
            Column { name: "id".into(), kind: ColumnKind::Integer, width: 100.0 },
            Column { name: "name".into(), kind: ColumnKind::Text, width: 200.0 },
        ],
        (0..30)
            .map(|i| vec![CellValue::Integer(i), CellValue::Text(format!("row{i}"))])
            .collect(),
    )
    .expect("rectangular data")
}

struct Harness {
    grid: gpui::Entity<SqllyDataTable>,
}

impl Render for Harness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let grid = self.grid.clone();
        // Outer app-like frame: vertical stack, a fixed-height results pane
        // near the bottom holding the ResultsView-equivalent tree.
        div()
            .flex()
            .flex_col()
            .size_full()
            // editor area (flex_1) above the results pane
            .child(div().flex_1().child("editor"))
            // results pane: fixed height, like app.rs `div().h(px(results_height))`
            .child(
                div().h(px(300.0)).child(
                    // ResultsView tree
                    div()
                        .flex()
                        .flex_col()
                        .size_full()
                        // tab bar
                        .child(div().h(px(28.0)).child("tabs"))
                        // content area (results.rs:1272)
                        .child(
                            div()
                                .id("results_content")
                                .flex_1()
                                .overflow_hidden()
                                // tabbed-results wrapper (results.rs:1337)
                                .child(
                                    div()
                                        .id("tabbed-results")
                                        .size_full()
                                        .on_scroll_wheel(|_e: &ScrollWheelEvent, _w, _c| {})
                                        .child(grid),
                                ),
                        )
                        // status bar
                        .child(div().h(px(22.0)).child("status")),
                ),
            )
    }
}

#[gpui::test]
fn nested_like_app_left_click_selects_cell(cx: &mut TestAppContext) {
    let (harness, cx) = cx.add_window_view(|_window, cx| {
        let grid = SqllyDataTable::builder(sample())
            .config(GridConfig::default())
            .build(cx);
        let grid = cx.new(|_| grid);
        Harness { grid }
    });
    cx.run_until_parked();

    let (origin, header_h, row_h) = harness.read_with(cx, |h, cx| {
        let s = h.grid.read(cx).state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });

    // Click inside row 0, column 0 (x past the 50px row-header gutter).
    let x = f32::from(origin.x) + 90.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    let selection = harness.read_with(cx, |h, cx| h.grid.read(cx).state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 0),
        "nested-like-app click should still select Cell(0,0), got {selection:?} (bounds origin {origin:?})"
    );
}
