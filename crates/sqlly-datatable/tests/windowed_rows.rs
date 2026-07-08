//! Tests for windowed-row mode (`GridState::set_row_window`): the grid
//! presents a virtual total row count while holding only a resident window,
//! so hosts can page rows in/out of memory as the user scrolls arbitrarily
//! large spill-backed result sets.

#![allow(clippy::expect_used)]

use gpui::{px, Point, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, Selection, SqllyDataTable,
};

fn columns() -> Vec<Column> {
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
    ]
}

fn row(i: i64) -> Vec<CellValue> {
    vec![CellValue::Integer(i), CellValue::Text(format!("row{i}"))]
}

fn rows(range: std::ops::Range<i64>) -> Vec<Vec<CellValue>> {
    range.map(row).collect()
}

const TOTAL: usize = 1_000_000;

#[gpui::test]
fn window_presents_virtual_total_with_resident_slice(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            // One million virtual rows, resident window = [1000, 1200).
            s.set_row_window(TOTAL, 1000, rows(1000..1200))
                .expect("rectangular window");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.display_row_count(), TOTAL, "scroll space is the total");
        assert_eq!(s.data.rows.len(), 200, "only the window is resident");
        // Virtual row 1005 is resident index 5.
        assert_eq!(s.resident_row_for_display(1005), Some(5));
        assert_eq!(
            s.data.rows[5][1],
            CellValue::Text("row1005".into()),
            "mapping lands on the right data"
        );
        // Rows outside the window are known but not resident.
        assert_eq!(s.resident_row_for_display(0), None);
        assert_eq!(s.resident_row_for_display(500_000), None);
        // Past the virtual end is out of range entirely.
        assert_eq!(s.resident_row_for_display(TOTAL), None);
    });
}

#[gpui::test]
fn window_replacement_repositions_the_slice(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_window(TOTAL, 0, rows(0..100)).expect("window 1");
            // Paging: replace with a window further down.
            s.set_row_window(TOTAL, 5000, rows(5000..5100))
                .expect("window 2");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.resident_row_for_display(0), None, "old window evicted");
        assert_eq!(s.resident_row_for_display(5050), Some(50));
        assert_eq!(s.data.rows[50][0], CellValue::Integer(5050));
    });
}

#[gpui::test]
fn sort_and_filter_are_disabled_while_windowed(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_window(TOTAL, 0, rows(0..100)).expect("window");
            s.toggle_sort(0);
            assert_eq!(s.sort, None, "toggle_sort must be a no-op when windowed");
            s.recompute();
            assert_eq!(
                s.display_indices.as_slice(),
                (0..100).collect::<Vec<_>>().as_slice(),
                "recompute keeps the identity order over the resident window"
            );
        });
    });
}

#[gpui::test]
fn selection_and_copy_clamp_to_the_resident_window(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, cx| {
            s.set_row_window(TOTAL, 100, rows(100..200))
                .expect("window");
            // Select a virtual range that pokes past both window edges.
            s.selection = Selection::CellRange(50, 0, 250, 1);
            s.copy_selection(false, cx);
        });
    });
    cx.run_until_parked();

    // Only resident rows can be copied; non-resident ones are skipped, so
    // the clipboard holds exactly rows 100..=199.
    let text = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text()))
        .expect("clipboard has text");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 100, "one line per resident row, got {lines:?}");
    assert!(
        lines[0].ends_with("\trow100"),
        "first copied line must be resident row 100, got {:?}",
        lines[0]
    );
    assert!(
        lines[99].ends_with("\trow199"),
        "last copied line must be resident row 199, got {:?}",
        lines[99]
    );
}

#[gpui::test]
fn select_all_spans_the_virtual_total(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_window(TOTAL, 0, rows(0..100)).expect("window");
            s.select_all();
            assert_eq!(
                s.selection,
                Selection::CellRange(0, 0, TOTAL - 1, 1),
                "select-all covers the virtual set"
            );
        });
    });
}

/// Scrolling deep into non-resident territory must paint without panicking
/// (placeholder rows) and report the visible range in virtual space so the
/// host knows what to page in.
#[gpui::test]
fn scrolled_viewport_reports_virtual_visible_range_and_paints(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, cx| {
            s.set_row_window(TOTAL, 0, rows(0..100)).expect("window");
            // Jump the scroll position deep into the un-paged region.
            let row_h = s.row_height;
            s.scroll_handle.set_offset(Point {
                x: px(0.0),
                y: px(500_000.0 * row_h),
            });
            cx.notify();
        });
        cx.notify();
    });
    // Repaint with the viewport over non-resident rows — must not panic.
    cx.run_until_parked();

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        let (first, last) = s.visible_row_range();
        assert!(
            (499_000..=501_000).contains(&first),
            "visible range must be in virtual space near the scroll target, got {first}"
        );
        assert!(
            last > first,
            "range must be non-empty, got ({first},{last})"
        );
    });
}

#[gpui::test]
fn append_rows_extends_the_virtual_total_when_windowed(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(GridData::new(columns(), vec![]).expect("rectangular"))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.set_row_window(1000, 900, rows(900..1000))
                .expect("window");
            s.append_rows(rows(1000..1010)).expect("append");
            assert_eq!(s.display_row_count(), 1010, "total grows with appends");
            assert_eq!(s.resident_row_for_display(1005), Some(105));
        });
    });
}
