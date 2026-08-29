//! Column hide and reorder: display order drives paint/hit-test while
//! selection still uses source column indices.

#![allow(clippy::expect_used)]

use gpui::{point, px, Modifiers, MouseButton, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, Selection, SqllyDataTable,
};

fn three_cols() -> GridData {
    GridData::new(
        vec![
            Column {
                name: "a".into(),
                kind: ColumnKind::Integer,
                width: 100.0,
            },
            Column {
                name: "b".into(),
                kind: ColumnKind::Integer,
                width: 100.0,
            },
            Column {
                name: "c".into(),
                kind: ColumnKind::Integer,
                width: 100.0,
            },
        ],
        (0..4)
            .map(|r| {
                vec![
                    CellValue::Integer(r),
                    CellValue::Integer(r + 10),
                    CellValue::Integer(r + 20),
                ]
            })
            .collect(),
    )
    .expect("rectangular data")
}

fn click_cell(
    cx: &mut gpui::VisualTestContext,
    origin_x: f32,
    origin_y: f32,
    header_h: f32,
    row_h: f32,
    x_in_grid: f32,
) {
    let x = origin_x + x_in_grid;
    let y = origin_y + header_h + row_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn hide_column_shifts_hit_test_to_remaining_source(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(three_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.hide_column(1);
        });
    });
    cx.run_until_parked();

    let (origin, header_h, row_h, rhw) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (
            s.bounds.origin,
            s.header_height,
            s.row_height,
            s.row_header_width,
        )
    });

    // After hiding source col 1, source col 2 occupies the second display slot
    // (rhw+100 .. rhw+200). A click there must hit source col 2, not col 1.
    click_cell(
        cx,
        f32::from(origin.x),
        f32::from(origin.y),
        header_h,
        row_h,
        rhw + 150.0,
    );

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 2),
        "hidden col 1 should leave source col 2 under the old col-2 x, got {selection:?}"
    );
}

#[gpui::test]
fn set_column_order_paints_in_that_order(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(three_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_column_order(&[2, 0, 1]);
        });
    });
    cx.run_until_parked();

    let (origin, header_h, row_h, rhw) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (
            s.bounds.origin,
            s.header_height,
            s.row_height,
            s.row_header_width,
        )
    });

    click_cell(
        cx,
        f32::from(origin.x),
        f32::from(origin.y),
        header_h,
        row_h,
        rhw + 50.0,
    );

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 2),
        "order [2,0,1] should put source col 2 first, got {selection:?}"
    );
}

#[gpui::test]
fn show_all_columns_restores_identity(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(three_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let (order, hidden) = view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_column_order(&[2, 0]);
            assert_eq!(s.hidden_columns(), vec![1]);
            s.show_all_columns();
            (s.column_order().to_vec(), s.hidden_columns())
        })
    });
    assert_eq!(order, vec![0, 1, 2]);
    assert!(hidden.is_empty());
}

#[gpui::test]
fn invalid_column_order_is_ignored(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(three_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let order = view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_column_order(&[0, 9]);
            s.column_order().to_vec()
        })
    });
    assert_eq!(order, vec![0, 1, 2]);
}

#[gpui::test]
fn row_backgrounds_are_stored(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(three_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let len = view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_backgrounds(vec![None, Some(gpui::hsla(0.1, 0.5, 0.5, 0.2))]);
            s.row_backgrounds.len()
        })
    });
    assert_eq!(len, 2);
}
