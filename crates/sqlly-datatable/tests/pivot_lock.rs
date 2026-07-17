#![allow(clippy::expect_used)]

use gpui::{point, px, size, Bounds, TestAppContext};
use sqlly_datatable::pivot::PivotHitResult;
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, GridTab, PivotConfig,
    PivotSidebarPosition, SqllyDataTable,
};

#[gpui::test]
fn animations_flag_threads_from_config_into_pivot(cx: &mut TestAppContext) {
    // The pivot renders its own transient surfaces (menu, popover, dialog,
    // drag ghost, accordion bodies) from `PivotState`, which is built from a
    // *snapshot* of the grid — so the motion opt-out has to be copied across at
    // build time. This asserts that wiring: config off -> pivot off.
    let mut cfg = GridConfig::default();
    assert!(cfg.animations, "grid config defaults motion on");
    cfg.animations = false;

    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .config(cfg)
            .pivot(PivotConfig::default())
            .build(cx)
    });

    view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot enabled");
        assert!(
            !pivot.read(cx).animations,
            "pivot mirrors the grid config's animations flag"
        );
    });
}

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
fn pivot_value_cells_are_right_aligned_with_red_negatives(cx: &mut TestAppContext) {
    use sqlly_datatable::pivot::AggregationFn;
    use sqlly_datatable::TextAlignment;

    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(
            vec![
                Column::new("region", ColumnKind::Text, 100.0),
                Column::new("label", ColumnKind::Text, 100.0),
            ],
            vec![vec![
                CellValue::Text("East".into()),
                CellValue::Text("alpha".into()),
            ]],
        )
        .expect("rectangular pivot data");
        // Min over a Text column keeps kind Text — exactly the case that used
        // to fall back to the string format's left alignment.
        let config = PivotConfig {
            row_fields: vec![0],
            value_field: Some(1),
            aggregation: AggregationFn::Min,
            ..PivotConfig::default()
        };
        SqllyDataTable::builder(data).pivot(config).build(cx)
    });

    view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state");
        let fmt = pivot.read(cx).value_format().clone();
        assert_eq!(fmt.alignment(), TextAlignment::Right);
        assert!(fmt.number.show_negative_red);
    });
}

#[gpui::test]
fn pivot_format_dialog_edits_persist_and_round_trip_config(cx: &mut TestAppContext) {
    use sqlly_datatable::{PivotZone, TextAlignment};

    let data = || {
        GridData::new(
            vec![
                Column::new("year", ColumnKind::Integer, 100.0),
                Column::new("sales", ColumnKind::Integer, 100.0),
            ],
            vec![
                vec![CellValue::Integer(2025), CellValue::Integer(10)],
                vec![CellValue::Integer(2026), CellValue::Integer(-20)],
            ],
        )
        .expect("rectangular pivot data")
    };
    let config = PivotConfig {
        row_fields: vec![0],
        value_field: Some(1),
        ..PivotConfig::default()
    };

    let (view, cx) = cx.add_window_view({
        let config = config.clone();
        move |_window, cx| SqllyDataTable::builder(data()).pivot(config).build(cx)
    });

    let saved = view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state").clone();
        pivot.update(cx, |state, _cx| {
            // Integer row groups format with the resolved default.
            assert_eq!(state.result.row_nodes[0].label, "2,025.00");

            // Double-click on the row chip → dialog → uncheck separator,
            // drop decimals, center-align, negatives red.
            state.open_format_dialog(0, PivotZone::Rows, point(px(0.0), px(0.0)));
            state.update_format_dialog(|f| {
                f.thousands_separator = false;
                f.decimals = 0;
                f.alignment = TextAlignment::Center;
                f.show_negative_red = true;
            });
            assert_eq!(state.result.row_nodes[0].label, "2025");
            let fmt = state.label_format(0).expect("label format");
            assert_eq!(fmt.alignment(), TextAlignment::Center);
            assert!(state.config.field_formats.contains_key(&0));

            // Values chip dialog edits the value-format override instead.
            state.close_format_dialog();
            state.open_format_dialog(1, PivotZone::Values, point(px(0.0), px(0.0)));
            state.update_format_dialog(|f| {
                f.decimals = 3;
                f.negative_parentheses = true;
            });
            let vf = state.value_format().number;
            assert_eq!(vf.decimals, 3);
            assert!(vf.negative_parentheses);
            assert_eq!(state.config.value_format, Some(vf));

            // Reset drops the values override; the field override stays.
            state.reset_format_dialog();
            assert_eq!(state.config.value_format, None);
            assert!(state.config.field_formats.contains_key(&0));
            state.close_format_dialog();
        });
        pivot.read(cx).config.clone()
    });

    // A host can read the config back, persist it, and hand it to a fresh
    // widget over a fresh data load: the field formats come back with it.
    let (view2, cx) = cx
        .add_window_view(move |_window, cx| SqllyDataTable::builder(data()).pivot(saved).build(cx));
    view2.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state");
        let state = pivot.read(cx);
        assert_eq!(state.result.row_nodes[0].label, "2025");
        assert_eq!(
            state.label_format(0).expect("label format").alignment(),
            TextAlignment::Center
        );
    });
}

#[gpui::test]
fn pivot_save_config_button_is_wired_only_when_registered(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;

    // Without a handler the save action is absent (the sidebar hides its
    // save button).
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .pivot(PivotConfig::default())
            .build(cx)
    });
    view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state").clone();
        assert!(!pivot.read(cx).has_save_config_handler());

        // Wire at runtime, invoke, then clear.
        let saved: Rc<RefCell<Option<PivotConfig>>> = Rc::new(RefCell::new(None));
        let sink = saved.clone();
        table.set_pivot_save_config(
            move |config, _cx| {
                *sink.borrow_mut() = Some(config.clone());
            },
            cx,
        );
        assert!(pivot.read(cx).has_save_config_handler());

        pivot.update(cx, |state, cx| {
            state.config.row_fields = vec![0];
            let handler = state
                .save_config_handler()
                .expect("registered save handler");
            handler(&state.config.clone(), cx);
        });
        assert_eq!(
            saved.borrow().as_ref().map(|c| c.row_fields.clone()),
            Some(vec![0])
        );

        table.clear_pivot_save_config(cx);
        assert!(!pivot.read(cx).has_save_config_handler());
    });

    // Builder registration wires the handler from the start.
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .pivot(PivotConfig::default())
            .pivot_save_config(|_config, _cx| {})
            .build(cx)
    });
    view.update(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot state");
        assert!(pivot.read(cx).has_save_config_handler());
    });
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

            // The row-label area's right edge drags to resize its width.
            let start_width = state.row_header_area_width();
            let label_edge = point(px(start_width), px(state.header_height() + 10.0));
            assert_eq!(state.hit_test(label_edge), PivotHitResult::RowHeaderBorder);
            state.handle_mouse_down(label_edge, false);
            state.handle_mouse_move(point(px(start_width + 40.0), px(10.0)), true);
            state.handle_mouse_up();
            assert_eq!(state.row_header_area_width(), start_width + 40.0);

            // Clamped to the supported minimum.
            state.set_row_header_width(1.0);
            assert_eq!(
                state.row_header_area_width(),
                sqlly_datatable::MIN_PIVOT_ROW_HEADER_WIDTH
            );
        });
    });
}

#[gpui::test]
fn pivot_tab_draws_in_every_sidebar_state(cx: &mut TestAppContext) {
    // The pivot split is a `gpui-component` resizable panel group whose
    // resize handle reads the toolkit's global theme — installed by
    // `sqlly_datatable::init`. This drives a real draw of the pivot tab in
    // each sidebar state (expanded left/right, collapsed) to catch a missing
    // init or a broken split layout at render time, which the state-only
    // tests above never exercise.
    use gpui::{Modifiers, MouseButton};

    cx.update(sqlly_datatable::init);

    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(
            vec![
                Column::new("region", ColumnKind::Text, 100.0),
                Column::new("n", ColumnKind::Integer, 80.0),
            ],
            vec![
                vec![CellValue::Text("East".into()), CellValue::Integer(1)],
                vec![CellValue::Text("West".into()), CellValue::Integer(2)],
            ],
        )
        .expect("rectangular pivot data");
        let config = PivotConfig {
            row_fields: vec![0],
            value_field: Some(1),
            ..PivotConfig::default()
        };
        SqllyDataTable::builder(data).pivot(config).build(cx)
    });

    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.simulate_mouse_move(
            point(px(300.0), px(300.0)),
            Option::<MouseButton>::None,
            Modifiers::none(),
        );
        cx.run_until_parked();
    };

    view.update(cx, |table, cx| {
        table.set_active_tab(GridTab::Pivot, cx);
    });
    draw(cx); // expanded, sidebar left

    view.update(cx, |table, cx| {
        table.set_pivot_sidebar_collapsed(true);
        cx.notify();
    });
    draw(cx); // collapsed rail

    view.update(cx, |table, cx| {
        table.set_pivot_sidebar_collapsed(false);
        table.set_pivot_sidebar_position(PivotSidebarPosition::Right);
        table.set_pivot_sidebar_width(240.0);
        cx.notify();
    });
    draw(cx); // expanded, sidebar right, programmatic width

    view.update(cx, |table, _cx| {
        assert!(!table.pivot_sidebar_collapsed());
        assert_eq!(table.pivot_sidebar_position(), PivotSidebarPosition::Right);
        assert_eq!(table.pivot_sidebar_width(), 240.0);
    });
}

#[gpui::test]
fn drill_through_shows_and_clears_grid_filter_banner(cx: &mut TestAppContext) {
    // A pivot drill-through silently replaces the flat grid's column
    // filters; the Grid tab must name the applied filter (banner state) and
    // clear it in one step.
    use gpui::{Modifiers, MouseButton};

    cx.update(sqlly_datatable::init);

    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(
            vec![
                Column::new("region", ColumnKind::Text, 100.0),
                Column::new("n", ColumnKind::Integer, 80.0),
            ],
            vec![
                vec![CellValue::Text("East".into()), CellValue::Integer(1)],
                vec![CellValue::Text("West".into()), CellValue::Integer(2)],
            ],
        )
        .expect("rectangular pivot data");
        let config = PivotConfig {
            row_fields: vec![0],
            value_field: Some(1),
            ..PivotConfig::default()
        };
        SqllyDataTable::builder(data).pivot(config).build(cx)
    });

    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.simulate_mouse_move(
            point(px(300.0), px(300.0)),
            Option::<MouseButton>::None,
            Modifiers::none(),
        );
        cx.run_until_parked();
    };

    view.update(cx, |table, cx| {
        assert_eq!(table.drill_filter(), None);
        let pivot = table.pivot_state().expect("pivot enabled").clone();
        pivot.update(cx, |state, _cx| {
            // Drill on the first visible value cell ("East" row).
            state.request_drill_down(0, 0);
        });
        cx.notify();
    });
    draw(cx); // render pass applies the queued drill

    view.update(cx, |table, cx| {
        assert_eq!(table.active_tab(), GridTab::Grid, "drill lands on Grid tab");
        let label = table.drill_filter().expect("drill banner state set");
        assert!(
            label.contains("region") && label.contains("East"),
            "label names the filtered column and value: {label}"
        );
        let filtered: Vec<_> = table
            .state
            .read(cx)
            .filters
            .iter()
            .filter(|f| f.values.is_some())
            .collect();
        assert_eq!(filtered.len(), 1, "exactly the drill filter is active");

        table.clear_drill_filter(cx);
        assert_eq!(table.drill_filter(), None);
        assert!(
            table
                .state
                .read(cx)
                .filters
                .iter()
                .all(|f| f.values.is_none()),
            "clearing the banner resets every column filter"
        );
    });
}
