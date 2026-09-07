#![allow(clippy::expect_used, clippy::field_reassign_with_default)]

use gpui::TestAppContext;
use sqlly_datatable::{
    CellValue, ChartConfig, ChartKind, Column, ColumnKind, GridData, GridTab, SqllyDataTable,
};

fn sample_data() -> GridData {
    let columns = vec![
        Column::new("name", ColumnKind::Text, 120.0),
        Column::new("score", ColumnKind::Integer, 90.0),
        Column::new("delta", ColumnKind::Decimal, 90.0),
    ];
    let rows = vec![
        vec![
            CellValue::Text("a".into()),
            CellValue::Integer(40),
            CellValue::Decimal(1.5),
        ],
        vec![
            CellValue::Text("b".into()),
            CellValue::Integer(25),
            CellValue::Decimal(-0.5),
        ],
        vec![
            CellValue::Text("c".into()),
            CellValue::Integer(25),
            CellValue::Decimal(2.0),
        ],
        vec![
            CellValue::Text("d".into()),
            CellValue::Integer(10),
            CellValue::Decimal(0.5),
        ],
    ];
    GridData::new(columns, rows).expect("sample grid")
}

#[gpui::test]
fn chart_tab_builder_enables_chart(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample_data())
            .chart(ChartConfig::default())
            .build(cx)
    });

    view.update(cx, |table, cx| {
        assert!(table.chart_state().is_some(), "chart tab installed");
        assert_eq!(table.active_tab(), GridTab::Grid);

        let chart = table.chart_state().expect("chart state");
        assert!(chart.read(cx).is_chartable());
        chart.update(cx, |s, _cx| {
            assert_eq!(s.config.kind, ChartKind::Bar);
            assert_eq!(s.series_count(), 2, "score + delta");
            assert_eq!(s.category_count(), 4);
            assert_eq!(s.empty_message(), "", "chartable data has no empty message");
        });
    });
}

#[gpui::test]
fn chart_tab_activation_and_lock(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample_data())
            .chart(ChartConfig::default())
            .build(cx)
    });

    view.update(cx, |table, cx| {
        table.set_active_tab(GridTab::Chart, cx);
        assert_eq!(table.active_tab(), GridTab::Chart);

        // Locking returns to the grid and rejects activation; the tab
        // itself stays installed.
        table.set_chart_locked(true, Some("Loading".to_string()));
        assert!(table.chart_locked());
        assert_eq!(table.active_tab(), GridTab::Grid);
        table.set_active_tab(GridTab::Chart, cx);
        assert_eq!(table.active_tab(), GridTab::Grid, "locked rejects");

        table.set_chart_locked(false, None);
        table.set_active_tab(GridTab::Chart, cx);
        assert_eq!(table.active_tab(), GridTab::Chart);
    });
}

#[gpui::test]
fn chart_tab_activation_without_chart_is_noop(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(sample_data()).build(cx));

    view.update(cx, |table, cx| {
        assert!(table.chart_state().is_none());
        table.set_active_tab(GridTab::Chart, cx);
        assert_eq!(table.active_tab(), GridTab::Grid);
    });
}

#[gpui::test]
fn chart_enable_disable_runtime_roundtrip(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(sample_data()).build(cx));

    view.update(cx, |table, cx| {
        assert!(table.chart_state().is_none());
        table.enable_chart(ChartConfig::default(), cx);
        assert!(table.chart_state().is_some());
        table.set_active_tab(GridTab::Chart, cx);
        assert_eq!(table.active_tab(), GridTab::Chart);

        // Re-enabling reconfigures instead of duplicating.
        let mut config = ChartConfig::default();
        config.kind = ChartKind::Line;
        table.enable_chart(config, cx);
        let chart = table.chart_state().expect("chart state");
        chart.update(cx, |s, _cx| {
            assert_eq!(s.config.kind, ChartKind::Line);
        });

        // Disabling returns to the grid.
        table.disable_chart();
        assert!(table.chart_state().is_none());
        assert_eq!(table.active_tab(), GridTab::Grid);
    });
}

#[gpui::test]
fn chart_config_roundtrip_external(cx: &mut TestAppContext) {
    // The externally supplied config drives extraction; the state reports
    // the same config back (the host persistence contract).
    let mut configured = ChartConfig::default();
    configured.label_column = Some(0);
    configured.value_columns = vec![1];
    configured.top_n = 2;

    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample_data())
            .chart(configured)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        let chart = table.chart_state().expect("chart state");
        chart.update(cx, |s, _cx| {
            assert_eq!(s.config.label_column, Some(0));
            assert_eq!(s.config.value_columns, vec![1]);
            assert_eq!(s.config.top_n, 2);
            assert_eq!(s.category_count(), 2, "top_n clamps the categories");
        });

        // Runtime reconfiguration through the same external API.
        let mut next = ChartConfig::default();
        next.kind = ChartKind::Pie;
        next.value_columns = vec![1]; // score: non-negative → valid pie
        chart.update(cx, |s, _cx| s.set_config(next));
        chart.update(cx, |s, _cx| {
            assert_eq!(s.config.kind, ChartKind::Pie);
            assert_eq!(s.series_count(), 1);
            assert!(s.paint_for().is_ok(), "pie over one non-negative series");
        });
    });
}

#[gpui::test]
fn chart_histogram_over_numeric_source(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(sample_data())
            .chart(ChartConfig {
                kind: ChartKind::Histogram,
                ..ChartConfig::default()
            })
            .build(cx)
    });

    view.update(cx, |table, cx| {
        let chart = table.chart_state().expect("chart state");
        chart.update(cx, |s, _cx| {
            // The histogram paints from the source columns directly (the
            // extraction outcome is kind-independent), so it resolves even
            // though the default extraction also succeeded.
            assert!(s.paint_for().is_ok(), "histogram over score/delta");
            assert!(s.svg().is_some(), "histogram exports SVG");
        });
    });
}
