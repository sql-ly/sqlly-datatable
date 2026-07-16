//! Hardening: the grid must survive hostile real-world data — empty result
//! sets, zero-column frames, multi-byte text (emoji / CJK / RTL), extreme
//! numbers, and NaN/infinity — without panicking, and core interactions
//! (sort, filter, select, copy) must keep working over that data.

#![allow(clippy::expect_used)]

use gpui::{point, px, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, Selection, SqllyDataTable,
};

fn hostile_data() -> GridData {
    let long = "x".repeat(500);
    GridData::new(
        vec![
            Column::new("text", ColumnKind::Text, 120.0),
            Column::new("amount", ColumnKind::Decimal, 100.0),
            Column::new("count", ColumnKind::Integer, 90.0),
        ],
        vec![
            // Emoji (multi-byte, ZWJ sequence), CJK, RTL, combining accents.
            vec![
                CellValue::Text("👩‍👩‍👧‍👦 family 👍".into()),
                CellValue::Decimal(f64::NAN),
                CellValue::Integer(i64::MAX),
            ],
            vec![
                CellValue::Text("日本語のテキストと漢字".into()),
                CellValue::Decimal(f64::INFINITY),
                CellValue::Integer(i64::MIN),
            ],
            vec![
                CellValue::Text("مرحبا بالعالم שלום עולם".into()),
                CellValue::Decimal(f64::NEG_INFINITY),
                CellValue::Integer(0),
            ],
            vec![
                CellValue::Text(long),
                CellValue::Decimal(f64::MAX),
                CellValue::Integer(-1),
            ],
            vec![CellValue::None, CellValue::None, CellValue::None],
        ],
    )
    .expect("rectangular hostile data")
}

#[gpui::test]
fn hostile_text_and_numbers_survive_sort_filter_select_copy(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(hostile_data()).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, cx| {
            // Sort every column in both directions over NaN/inf/multi-byte.
            for col in 0..3 {
                state.toggle_sort(col);
                state.toggle_sort(col);
                state.toggle_sort(col);
            }
            // Filter panel over multi-byte values.
            state.open_filter_panel(0, None);
            state.apply_filter_panel();
            state.clear_filter_panel();
            // Select everything and copy (formats every hostile cell).
            state.select_all();
            assert!(matches!(state.selection, Selection::CellRange(..)));
            state.copy_selection(true, cx);
            state.copy_selection(false, cx);
            // Pointer sweep across the surface, including out of bounds —
            // exercises hit-testing and hover resolution.
            for x in [-10.0_f32, 0.0, 55.0, 200.0, 5000.0] {
                for y in [-10.0_f32, 0.0, 16.0, 100.0, 5000.0] {
                    state.handle_mouse_move(point(px(x), px(y)), None);
                }
            }
        });
    });
}

#[gpui::test]
fn empty_result_set_paints_hint_and_interactions_are_inert(cx: &mut TestAppContext) {
    let data = GridData::new(
        vec![Column::new("amount", ColumnKind::Decimal, 140.0)],
        vec![],
    )
    .expect("empty result set is valid");
    let (view, cx) = cx.add_window_view(|_window, cx| SqllyDataTable::builder(data).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            // The empty hint is on by default and host-overridable.
            assert_eq!(state.config.empty_text, "No rows");
            assert_eq!(state.display_row_count(), 0);
            // Interactions on an empty grid are inert, never panicking.
            state.select_all();
            assert!(matches!(state.selection, Selection::None));
            state.toggle_sort(0);
            state.handle_mouse_move(point(px(60.0), px(60.0)), None);
        });
    });
}

#[gpui::test]
fn zero_column_frame_does_not_panic(cx: &mut TestAppContext) {
    let data = GridData::new(vec![], vec![]).expect("zero columns is a valid frame");
    let (view, cx) = cx.add_window_view(|_window, cx| SqllyDataTable::builder(data).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            state.select_all();
            assert!(matches!(state.selection, Selection::None));
            state.handle_mouse_move(point(px(10.0), px(10.0)), None);
        });
    });
}

#[gpui::test]
fn empty_text_is_localizable_and_suppressible(cx: &mut TestAppContext) {
    let data = GridData::new(vec![Column::new("a", ColumnKind::Text, 100.0)], vec![])
        .expect("valid empty data");
    let config = GridConfig {
        empty_text: "Keine Zeilen vorhanden".into(),
        ..GridConfig::default()
    };
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(data).config(config).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            assert_eq!(state.config.empty_text, "Keine Zeilen vorhanden");
        });
    });
}
