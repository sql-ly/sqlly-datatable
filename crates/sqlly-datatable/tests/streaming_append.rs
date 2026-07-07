//! Tests for `GridState::append_rows` — the streaming-results fast path used
//! by hosts that receive rows in batches while a query is still running. The
//! append must be visible on every surface the widget reads from: the
//! canonical `data.rows`, the display order, and the paint/context-menu row
//! snapshot (`data_rows`), which is exercised here through a real right-click
//! dispatch on an appended row.

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use gpui::{point, px, Modifiers, MouseButton, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnFilter, ColumnKind, ContextMenuItem, ContextMenuProvider,
    ContextMenuRequest, FilterPredicate, GridConfig, GridData, GridDataError, Selection,
    SortDirection, SqllyDataTable, TextOp,
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

fn initial(n: i64) -> GridData {
    GridData::new(columns(), (0..n).map(row).collect()).expect("rectangular data")
}

#[gpui::test]
fn append_extends_rows_and_display_indices_in_order(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(3))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.append_rows(vec![row(3), row(4)]).expect("rectangular");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.data.rows.len(), 5, "data.rows must grow");
        assert_eq!(
            s.display_indices.as_slice(),
            &[0, 1, 2, 3, 4],
            "unsorted/unfiltered append extends display order in place"
        );
        assert_eq!(s.data.rows[4][1], CellValue::Text("row4".into()));
    });
}

#[gpui::test]
fn append_respects_active_sort(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(3))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.sort = Some((0, SortDirection::Descending));
            s.recompute();
            // New row with the highest id must surface at the top.
            s.append_rows(vec![row(99)]).expect("rectangular");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(
            s.display_indices.as_slice(),
            &[3, 2, 1, 0],
            "descending sort must place the appended id=99 row first"
        );
    });
}

#[gpui::test]
fn append_respects_active_filter(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(3))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.filters[1] = ColumnFilter {
                predicate: FilterPredicate::Text {
                    op: TextOp::Contains,
                    operand: "row1".into(),
                },
                values: None,
            };
            s.recompute();
            assert_eq!(s.display_indices.as_slice(), &[1]);
            // row10 matches "row1", row5 does not.
            s.append_rows(vec![row(5), row(10)]).expect("rectangular");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.data.rows.len(), 5, "all rows are stored");
        assert_eq!(
            s.display_indices.as_slice(),
            &[1, 4],
            "the filter must apply to appended rows too"
        );
    });
}

#[gpui::test]
fn append_ragged_row_is_rejected_and_grid_unchanged(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(2))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            let err = s
                .append_rows(vec![row(2), vec![CellValue::Integer(3)]])
                .expect_err("ragged row must be rejected");
            assert_eq!(
                err,
                GridDataError::RaggedRow {
                    row_index: 3,
                    expected: 2,
                    actual: 1,
                },
                "error must report the absolute index of the offending row"
            );
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.data.rows.len(), 2, "a rejected append must not mutate");
        assert_eq!(s.display_indices.as_slice(), &[0, 1]);
    });
}

#[gpui::test]
fn append_empty_batch_is_noop(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(2))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.append_rows(Vec::new()).expect("empty append is fine");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(s.data.rows.len(), 2);
        assert_eq!(s.display_indices.as_slice(), &[0, 1]);
    });
}

#[gpui::test]
fn append_preserves_selection(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(3))
            .config(GridConfig::default())
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.selection = Selection::Cell(1, 1);
            s.append_rows(vec![row(3)]).expect("rectangular");
        });
    });

    view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        assert_eq!(
            s.selection,
            Selection::Cell(1, 1),
            "appending must not disturb the user's selection"
        );
    });
}

/// Captures the last context-menu request so the test can inspect the row
/// snapshot (`data_rows`) the widget hands to providers.
struct CapturingProvider {
    last: Arc<Mutex<Option<(String, CellValue)>>>,
}

impl ContextMenuProvider for CapturingProvider {
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
        if let Some(cell) = request.clicked_cell() {
            *self.last.lock().expect("lock") = Some((cell.column_name.clone(), cell.value.clone()));
        }
        vec![ContextMenuItem::action("noop", "Noop")]
    }
}

/// The paint/context-menu snapshot (`data_rows`) must include appended rows —
/// this drives a REAL right-click through GPUI dispatch on a row that only
/// exists because of `append_rows`, and asserts the provider sees its values.
#[gpui::test]
fn appended_rows_reach_the_paint_and_menu_snapshot(cx: &mut TestAppContext) {
    let captured: Arc<Mutex<Option<(String, CellValue)>>> = Arc::new(Mutex::new(None));
    let provider = CapturingProvider {
        last: captured.clone(),
    };

    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(initial(2))
            .config(GridConfig::default())
            .context_menu_provider(provider)
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |v, cx| {
        v.state.update(cx, |s, _cx| {
            s.append_rows(vec![row(2)]).expect("rectangular");
        });
        cx.notify();
    });
    cx.run_until_parked();

    // Right-click squarely inside the appended row (display row 2, column 1).
    let (origin, header_h, row_h) = view.read_with(cx, |v, cx| {
        let s = v.state.read(cx);
        (s.bounds.origin, s.header_height, s.row_height)
    });
    let x = f32::from(origin.x) + 150.0;
    let y = f32::from(origin.y) + header_h + row_h * 2.5;
    cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(point(px(x), px(y)), MouseButton::Right, Modifiers::none());
    cx.run_until_parked();

    let captured = captured.lock().expect("lock").clone();
    let (name, value) = captured.expect("right-click on the appended row must reach the provider");
    assert_eq!(name, "name");
    assert_eq!(
        value,
        CellValue::Text("row2".into()),
        "the provider snapshot must contain the appended row's value"
    );
}
