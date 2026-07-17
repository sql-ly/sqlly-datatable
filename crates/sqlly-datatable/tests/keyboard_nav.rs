#![allow(clippy::expect_used)]

//! Keyboard navigation: the full arrow / Home / End / PageUp / PageDown set
//! moves the active cell *and* scrolls it into view, so a keyboard user never
//! drives the selection off-screen. Exercises the public `handle_key` path and
//! asserts on the public scroll offset.

use gpui::{point, px, size, Bounds, Keystroke, TestAppContext};
use sqlly_datatable::{CellValue, Column, ColumnKind, GridData, Selection, SqllyDataTable};

fn tall_wide_data() -> GridData {
    let cols = vec![
        Column::new("a", ColumnKind::Integer, 200.0),
        Column::new("b", ColumnKind::Integer, 200.0),
        Column::new("c", ColumnKind::Integer, 200.0),
    ];
    let rows: Vec<Vec<CellValue>> = (0..200)
        .map(|r| {
            (0..3)
                .map(|c| CellValue::Integer((r * 3 + c) as i64))
                .collect()
        })
        .collect();
    GridData::new(cols, rows).expect("rectangular")
}

fn key(k: &str) -> Keystroke {
    Keystroke {
        key: k.into(),
        ..Default::default()
    }
}

fn selected_col(sel: Selection) -> usize {
    match sel {
        Selection::Cell(_, c) => c,
        other => panic!("expected a cell selection, got {other:?}"),
    }
}

#[gpui::test]
fn keyboard_navigation_scrolls_active_cell_into_view(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| SqllyDataTable::builder(tall_wide_data()).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |s, _cx| {
            // Small viewport: 400x200. Content is 4800px tall and 600px wide,
            // so both scrollbars are live and every axis can scroll.
            s.bounds = Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(400.0), px(200.0)),
            };
            s.selection = Selection::Cell(0, 0);

            // Arrowing down past the fold scrolls vertically to follow the cell.
            for _ in 0..20 {
                s.handle_key(&key("down"));
            }
            assert_eq!(s.selection, Selection::Cell(20, 0));
            assert!(
                f32::from(s.scroll_handle.offset().y) > 0.0,
                "arrowing the active cell below the fold must scroll it into view"
            );

            // PageDown advances by a viewport of rows and stays visible.
            let before_y = f32::from(s.scroll_handle.offset().y);
            s.handle_key(&key("pagedown"));
            assert!(matches!(s.selection, Selection::Cell(r, 0) if r > 20));
            assert!(f32::from(s.scroll_handle.offset().y) >= before_y);

            // End jumps to the last column and scrolls right to reveal it.
            s.handle_key(&key("end"));
            assert_eq!(selected_col(s.selection.clone()), 2);
            assert!(
                f32::from(s.scroll_handle.offset().x) > 0.0,
                "End must scroll right to reveal the last column"
            );

            // Home returns to the first column and scrolls fully back.
            s.handle_key(&key("home"));
            assert_eq!(selected_col(s.selection.clone()), 0);
            assert_eq!(
                f32::from(s.scroll_handle.offset().x),
                0.0,
                "Home must scroll back to the first column"
            );

            // Escape clears the selection.
            s.handle_key(&key("escape"));
            assert_eq!(s.selection, Selection::None);
        });
    });
}

#[gpui::test]
fn shift_navigation_extends_range_to_row_and_column_extremes(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| SqllyDataTable::builder(tall_wide_data()).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |s, _cx| {
            s.bounds = Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(400.0), px(200.0)),
            };
            s.selection = Selection::Cell(0, 0);

            // Shift+End extends the selection across the row to the last column.
            s.handle_key(&Keystroke {
                key: "end".into(),
                modifiers: gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
                ..Default::default()
            });
            assert_eq!(s.selection, Selection::CellRange(0, 0, 0, 2));
            // The moving corner (last column) is scrolled into view.
            assert!(f32::from(s.scroll_handle.offset().x) > 0.0);
        });
    });
}
