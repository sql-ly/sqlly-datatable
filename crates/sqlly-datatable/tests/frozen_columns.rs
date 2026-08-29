//! Frozen leading columns: hit-testing ignores horizontal scroll for the
//! pinned prefix, `set_frozen_columns` clamps, and the header menu round-trips.

#![allow(clippy::expect_used)]

use gpui::{point, px, Modifiers, MouseButton, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, MenuAction, Selection, SqllyDataTable,
};

fn six_cols() -> GridData {
    GridData::new(
        (0..6)
            .map(|i| Column {
                name: format!("c{i}"),
                kind: ColumnKind::Integer,
                width: 100.0,
            })
            .collect(),
        (0..8)
            .map(|r| {
                (0..6)
                    .map(|c| CellValue::Integer(r * 10 + c))
                    .collect::<Vec<_>>()
            })
            .collect(),
    )
    .expect("rectangular data")
}

#[gpui::test]
fn frozen_column_hit_ignores_horizontal_scroll(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(six_cols())
            .config(GridConfig::default())
            .frozen_columns(2)
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.scroll_handle.set_offset(point(px(150.0), px(0.0)));
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

    // Frozen column 1 sits at rhw+100 regardless of sx=150.
    let x = f32::from(origin.x) + rhw + 150.0;
    let y = f32::from(origin.y) + header_h + row_h * 0.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    let selection = view.read_with(cx, |v, cx| v.state.read(cx).selection.clone());
    assert_eq!(
        selection,
        Selection::Cell(0, 1),
        "click on frozen column 1 after scrolling x by 150 should still hit source col 1, got {selection:?}"
    );
}

#[gpui::test]
fn set_frozen_columns_clamps_to_column_count(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(six_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    let clamped = view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_frozen_columns(99);
            s.frozen_columns()
        })
    });
    assert_eq!(clamped, 6);
}

#[gpui::test]
fn freeze_menu_actions_round_trip(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(six_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, app| {
            s.execute_action(MenuAction::FreezeToHere, 2, app);
            assert_eq!(s.frozen_columns(), 3);
            s.execute_action(MenuAction::UnfreezeColumns, 0, app);
            assert_eq!(s.frozen_columns(), 0);
        });
    });
}

#[gpui::test]
fn set_row_backgrounds_stores_per_source_row(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(six_cols())
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_backgrounds(vec![Some(gpui::hsla(0.0, 0.8, 0.5, 0.18)), None]);
            assert_eq!(s.row_backgrounds.len(), 2);
            assert!(s.row_backgrounds[0].is_some());
            assert!(s.row_backgrounds[1].is_none());
        });
    });
}
