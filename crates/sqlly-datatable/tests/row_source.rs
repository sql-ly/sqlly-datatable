//! Pivot and chart over a [`RowSource`]: a windowed grid holds only the rows
//! near the viewport, so both tabs read the complete result from the host's
//! source on the background executor instead of aggregating resident rows.

#![allow(clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::TestAppContext;
use sqlly_datatable::{
    AggregationFn, CellValue, ChartAggregate, ChartConfig, ChartKind, ChartOrder, Column,
    ColumnKind, GridData, GridTab, PivotConfig, RowSource, RowSourceDrill, RowSourceError,
    Selection, SqllyDataTable,
};

const TOTAL: usize = 200_000;
const GROUPS: usize = 7;

fn columns() -> Vec<Column> {
    vec![
        Column::new("amount", ColumnKind::Integer, 100.0),
        Column::new("region", ColumnKind::Text, 100.0),
    ]
}

fn row(i: usize) -> Vec<CellValue> {
    vec![
        CellValue::Integer(i64::try_from(i % 10).expect("small")),
        CellValue::Text(format!("region-{}", i % GROUPS)),
    ]
}

fn full_result() -> Arc<dyn RowSource> {
    Arc::new((0..TOTAL).map(row).collect::<Vec<_>>())
}

/// A windowed grid over the first 100 of `TOTAL` rows.
fn windowed_table(
    cx: &mut TestAppContext,
) -> (gpui::Entity<SqllyDataTable>, &mut gpui::VisualTestContext) {
    // The tabs draw gpui-component widgets, which need its theme.
    cx.update(sqlly_datatable::init);
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .pivot(PivotConfig {
                row_fields: vec![1],
                value_field: Some(0),
                aggregation: AggregationFn::Sum,
                ..PivotConfig::default()
            })
            .chart(ChartConfig {
                kind: ChartKind::Bar,
                label_column: Some(1),
                value_columns: vec![0],
                aggregate: ChartAggregate::Count,
                order: ChartOrder::CategoryAscending,
                ..ChartConfig::default()
            })
            .build(cx)
    });
    view.update(cx, |table, cx| {
        table.state.update(cx, |s, _cx| {
            s.set_row_window(TOTAL, 0, (0..100).map(row).collect())
                .expect("window");
        });
    });
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
fn pivot_reads_every_row_of_the_source_not_the_window(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    let source = full_result();
    view.update(cx, |table, cx| {
        table.set_row_source(Some(Arc::clone(&source)), cx);
        table.set_active_tab(GridTab::Pivot, cx);
        let pivot = table.pivot_state().expect("pivot").read(cx);
        assert!(
            pivot.is_scanning(),
            "the pivot is queued, not computed inline"
        );
        assert!(pivot.scan_status().is_some(), "a progress label is shown");
    });
    cx.run_until_parked();

    view.read_with(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot").read(cx);
        assert!(!pivot.is_scanning());
        assert_eq!(pivot.scan_status(), None);
        assert_eq!(pivot.result.source_row_count, TOTAL);
        let expected: i64 = (0..TOTAL)
            .map(|i| i64::try_from(i % 10).expect("small"))
            .sum();
        assert_eq!(pivot.result.grand_total, CellValue::Integer(expected));
        assert_eq!(pivot.result.row_roots.len(), GROUPS);
    });

    // Drill-through cannot filter a windowed grid: it does nothing.
    view.update(cx, |table, cx| {
        table
            .pivot_state()
            .expect("pivot")
            .update(cx, |p, _cx| p.request_drill_down(0, 0));
    });
    cx.run_until_parked();
    view.read_with(cx, |table, _cx| {
        assert_eq!(table.active_tab(), GridTab::Pivot);
    });
}

#[gpui::test]
fn reconfiguring_rescans_and_only_the_newest_scan_lands(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.set_active_tab(GridTab::Pivot, cx);
    });
    cx.run_until_parked();
    // Two layout changes back to back: the first scan is superseded.
    view.update(cx, |table, cx| {
        table.pivot_state().expect("pivot").update(cx, |p, cx| {
            p.config.aggregation = AggregationFn::Max;
            p.recompute();
            p.config.aggregation = AggregationFn::Count;
            p.recompute();
            cx.notify();
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot").read(cx);
        assert_eq!(
            pivot.result.grand_total,
            CellValue::Integer(i64::try_from(TOTAL).expect("fits"))
        );
    });
}

#[gpui::test]
fn chart_aggregates_every_row_of_the_source(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.set_active_tab(GridTab::Chart, cx);
        assert!(table.chart_state().expect("chart").read(cx).is_scanning());
    });
    cx.run_until_parked();

    view.read_with(cx, |table, cx| {
        let chart = table.chart_state().expect("chart").read(cx);
        assert!(!chart.is_scanning());
        assert!(chart.is_chartable(), "{}", chart.empty_message());
        assert_eq!(chart.category_count(), GROUPS);
        assert!(chart.paint_for().is_ok(), "paints");
        let data = chart.data().expect("data");
        let counted: f64 = data.series[0].values.iter().flatten().sum();
        assert_eq!(counted, TOTAL as f64, "every row is counted once");
    });
}

#[gpui::test]
fn clearing_the_source_returns_to_resident_rows(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.set_active_tab(GridTab::Pivot, cx);
    });
    cx.run_until_parked();
    view.update(cx, |table, cx| {
        table.set_row_source(None, cx);
        let pivot = table.pivot_state().expect("pivot").read(cx);
        assert!(!pivot.has_row_source());
        assert!(!pivot.is_scanning());
        assert_eq!(pivot.result.source_row_count, 100, "the resident window");
    });
}

struct FailingSource;

impl RowSource for FailingSource {
    fn row_count(&self) -> usize {
        10
    }

    fn scan(&self, _visit: &mut dyn FnMut(&[CellValue]) -> bool) -> Result<(), RowSourceError> {
        Err(RowSourceError("unreadable".into()))
    }
}

#[gpui::test]
fn unreadable_source_says_so_instead_of_showing_a_partial_pivot(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(Arc::new(FailingSource)), cx);
        table.set_active_tab(GridTab::Pivot, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |table, cx| {
        let pivot = table.pivot_state().expect("pivot").read(cx);
        assert_eq!(pivot.result.source_row_count, 0);
        assert_eq!(
            pivot.scan_status().as_deref(),
            Some("Couldn't read all of the result rows, so the pivot can't be shown.")
        );
    });
}

/// Collects every drill the grid hands to its host.
fn capture_drills(
    view: &gpui::Entity<SqllyDataTable>,
    cx: &mut gpui::VisualTestContext,
) -> Rc<RefCell<Vec<RowSourceDrill>>> {
    let drills = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&drills);
    view.update(cx, |table, cx| {
        table.set_row_source_drill_handler(
            Some(Rc::new(move |drill, _cx| sink.borrow_mut().push(drill))),
            cx,
        );
    });
    drills
}

fn matching_rows(drill: &RowSourceDrill) -> usize {
    (0..TOTAL).map(row).filter(|r| drill.matches(r)).count()
}

/// Repaint the window and settle: the chart records its clickable marks
/// while painting, and the grid applies a click on its next render.
fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, _cx| window.refresh());
    cx.run_until_parked();
}

/// Click the chart canvas until a mark answers (the plot layout is the
/// painter's business; any mark will do).
fn click_any_mark(view: &gpui::Entity<SqllyDataTable>, cx: &mut gpui::VisualTestContext) -> bool {
    view.update(cx, |table, cx| {
        let chart = table.chart_state().expect("chart").clone();
        chart.update(cx, |c, _cx| {
            for y in (0..800).step_by(4) {
                for x in (0..1600).step_by(4) {
                    if c.request_navigate_at(x as f32, y as f32) {
                        return true;
                    }
                }
            }
            false
        })
    })
}

#[gpui::test]
fn pivot_drill_over_a_source_hands_the_host_an_exact_row_test(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    let drills = capture_drills(&view, cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.set_active_tab(GridTab::Pivot, cx);
    });
    cx.run_until_parked();
    // Visible row 0 is the first region group; column 0 its value.
    view.update(cx, |table, cx| {
        table.pivot_state().expect("pivot").update(cx, |p, cx| {
            p.request_drill_down(0, 0);
            cx.notify();
        });
        cx.notify();
    });
    cx.run_until_parked();
    let drills = drills.borrow();
    assert_eq!(drills.len(), 1, "the host received the drill");
    assert_eq!(drills[0].label(), "region = region-0");
    let expected = (0..TOTAL).filter(|i| i % GROUPS == 0).count();
    assert_eq!(matching_rows(&drills[0]), expected);
}

#[gpui::test]
fn aggregated_chart_mark_drills_to_its_category(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    let drills = capture_drills(&view, cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.set_active_tab(GridTab::Chart, cx);
    });
    cx.run_until_parked();
    redraw(cx);
    assert!(click_any_mark(&view, cx), "a bar answers the click");
    redraw(cx);
    let drills = drills.borrow();
    assert_eq!(drills.len(), 1);
    assert!(drills[0].label().starts_with("region = region-"));
    assert_eq!(
        matching_rows(&drills[0]),
        TOTAL.div_ceil(GROUPS).min(TOTAL / GROUPS + 1)
    );
}

#[gpui::test]
fn histogram_bin_drills_to_the_values_inside_it(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    let drills = capture_drills(&view, cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.chart_state().expect("chart").update(cx, |c, _cx| {
            c.set_config(ChartConfig {
                kind: ChartKind::Histogram,
                value_columns: vec![0],
                ..ChartConfig::default()
            });
        });
        table.set_active_tab(GridTab::Chart, cx);
    });
    cx.run_until_parked();
    redraw(cx);
    assert!(click_any_mark(&view, cx), "a bin answers the click");
    redraw(cx);
    let drills = drills.borrow();
    assert_eq!(drills.len(), 1);
    assert!(
        drills[0].label().starts_with("amount = "),
        "{}",
        drills[0].label()
    );
    let matched = matching_rows(&drills[0]);
    assert!(
        matched > 0 && matched < TOTAL,
        "one bin of amounts, got {matched}"
    );
}

#[gpui::test]
fn per_row_chart_mark_selects_its_row_in_the_windowed_grid(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    view.update(cx, |table, cx| {
        table.set_row_source(Some(full_result()), cx);
        table.chart_state().expect("chart").update(cx, |c, _cx| {
            c.set_config(ChartConfig {
                kind: ChartKind::Line,
                label_column: Some(1),
                value_columns: vec![0],
                aggregate: ChartAggregate::None,
                order: ChartOrder::Source,
                ..ChartConfig::default()
            });
        });
        table.set_active_tab(GridTab::Chart, cx);
    });
    cx.run_until_parked();
    redraw(cx);
    assert!(click_any_mark(&view, cx), "a point answers the click");
    redraw(cx);
    view.read_with(cx, |table, cx| {
        assert_eq!(table.active_tab(), GridTab::Grid);
        match &table.state.read(cx).selection {
            Selection::Rows(rows) => {
                assert_eq!(rows.len(), 1);
                assert!(rows[0] < TOTAL);
            }
            other => panic!("expected a row selection, got {other:?}"),
        }
    });
}

#[gpui::test]
fn host_drill_banner_clear_calls_the_host(cx: &mut TestAppContext) {
    let (view, cx) = windowed_table(cx);
    let cleared = Rc::new(RefCell::new(0));
    let count = Rc::clone(&cleared);
    view.update(cx, |table, cx| {
        table.show_row_source_drill(
            "region = region-3".into(),
            Rc::new(move |_cx| *count.borrow_mut() += 1),
        );
        assert_eq!(table.drill_filter(), Some("region = region-3"));
        assert!(table.has_row_source_drill());
        table.clear_drill_filter(cx);
        assert_eq!(table.drill_filter(), None);
    });
    cx.run_until_parked();
    assert_eq!(
        *cleared.borrow(),
        1,
        "Clear hands the view back to the host"
    );
}
