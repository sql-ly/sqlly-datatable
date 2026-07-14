#![allow(clippy::expect_used)]

use gpui::{point, px, size, Bounds, TestAppContext};
use sqlly_datatable::pivot::PivotHitResult;
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridData, GridTab, PivotConfig, PivotSidebarPosition,
    SqllyDataTable,
};

#[gpui::test]
fn locked_pivot_stays_visible_but_rejects_activation(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .pivot(PivotConfig::default())
            .build(cx)
    });

    view.update(cx, |table, cx| {
        assert!(table.pivot_state().is_some(), "Pivot tab remains installed");
        table.set_pivot_locked(true, Some("Loading".to_string()));
        table.set_active_tab(GridTab::Pivot, cx);
        assert!(table.pivot_locked());
        assert_eq!(table.active_tab(), GridTab::Grid);

        table.set_pivot_locked(false, None);
        table.set_active_tab(GridTab::Pivot, cx);
        assert!(!table.pivot_locked());
        assert_eq!(table.active_tab(), GridTab::Pivot);
    });
}

#[gpui::test]
fn pivot_sidebar_layout_is_configurable(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .pivot(PivotConfig::default())
            .pivot_sidebar_position(PivotSidebarPosition::Right)
            .pivot_sidebar_collapsed(true)
            .pivot_sidebar_width(320.0)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        assert_eq!(table.pivot_sidebar_position(), PivotSidebarPosition::Right);
        assert!(table.pivot_sidebar_collapsed());
        assert_eq!(table.pivot_sidebar_width(), 320.0);

        table.set_pivot_sidebar_position(PivotSidebarPosition::Left);
        table.set_pivot_sidebar_collapsed(false);
        table.set_pivot_sidebar_width(0.0);

        assert_eq!(table.pivot_sidebar_width(), 180.0);

        table.set_pivot_sidebar_width(10_000.0);

        assert_eq!(table.pivot_sidebar_position(), PivotSidebarPosition::Left);
        assert!(!table.pivot_sidebar_collapsed());
        assert_eq!(table.pivot_sidebar_width(), 480.0);
        table.set_active_tab(GridTab::Pivot, cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn pivot_dimensions_are_configurable_readable_and_resizable(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(
            vec![
                Column::new("region", ColumnKind::Text, 100.0),
                Column::new("year", ColumnKind::Integer, 80.0),
                Column::new("sales", ColumnKind::Integer, 100.0),
            ],
            vec![
                vec![
                    CellValue::Text("East".into()),
                    CellValue::Integer(2025),
                    CellValue::Integer(10),
                ],
                vec![
                    CellValue::Text("West".into()),
                    CellValue::Integer(2025),
                    CellValue::Integer(20),
                ],
            ],
        )
        .expect("rectangular pivot data");
        let config = PivotConfig {
            row_fields: vec![0],
            column_fields: vec![1],
            value_field: Some(2),
            ..PivotConfig::default()
        };
        SqllyDataTable::builder(data)
            .pivot(config)
            .pivot_row_height(32.0)
            .pivot_column_width(180.0)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state");
        pivot.update(cx, |state, _cx| {
            assert_eq!(state.row_height(), 32.0);
            assert_eq!(state.column_width(), 180.0);

            state.set_row_height(1.0);
            state.set_column_width(1.0);
            assert_eq!(state.row_height(), 18.0);
            assert_eq!(state.column_width(), 40.0);

            state.set_row_height(32.0);
            state.set_column_width(180.0);
            state.bounds = Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            };

            let column_edge = point(px(state.row_header_width + 180.0), px(10.0));
            assert_eq!(
                state.hit_test(column_edge),
                PivotHitResult::ColBorder { col: 0 }
            );
            state.handle_mouse_down(column_edge, false);
            state.handle_mouse_move(point(px(f32::from(column_edge.x) + 20.0), px(10.0)), true);
            state.handle_mouse_up();
            assert_eq!(state.column_width(), 200.0);

            let row_edge = point(px(10.0), px(state.header_height() + 32.0));
            assert_eq!(
                state.hit_test(row_edge),
                PivotHitResult::RowBorder { row: 0 }
            );
            state.handle_mouse_down(row_edge, false);
            state.handle_mouse_move(point(px(10.0), px(f32::from(row_edge.y) + 8.0)), true);
            state.handle_mouse_up();
            assert_eq!(state.row_height(), 40.0);
        });
    });
}
