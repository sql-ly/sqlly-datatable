//! Cells holding line breaks paint as one line with a visible break marker
//! (the text shaper takes a single line and asserts on `\n` in debug
//! builds), while copy keeps the real line breaks.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use gpui::TestAppContext;
use sqlly_datatable::grid::paint::{single_line, LINE_BREAK_MARKER};
use sqlly_datatable::{
    CellStyler, CellTextStyler, CellValue, Column, ColumnKind, GridConfig, GridData, RgbaColor,
    Selection, SqllyDataTable, TextSpan,
};

const MULTI: &str = "first line\nsecond\r\nthird\rfourth";

fn data() -> GridData {
    GridData::new(
        vec![
            Column::new("note", ColumnKind::Text, 160.0),
            Column::new("doc", ColumnKind::Text, 160.0),
        ],
        vec![vec![
            CellValue::Text(MULTI.into()),
            CellValue::Text("{\n  \"a\": 1\n}".into()),
        ]],
    )
    .expect("rectangular data")
}

/// Colors every character, so styled painting of multi-line text is
/// exercised too.
struct ColorAll;

impl CellTextStyler for ColorAll {
    fn styles_column(&self, column: &str) -> bool {
        column == "doc"
    }
    fn spans(&self, _column: &str, text: &str) -> Vec<TextSpan> {
        assert!(!text.contains('\n'), "the styler sees the painted line");
        vec![TextSpan::new(
            0..text.len(),
            RgbaColor::new(1.0, 0.0, 0.0, 1.0),
        )]
    }
}

#[test]
fn line_breaks_become_markers() {
    let m = LINE_BREAK_MARKER;
    assert_eq!(
        single_line(MULTI),
        format!("first line{m}second{m}third{m}fourth")
    );
    assert!(matches!(
        single_line("tab\tstays"),
        std::borrow::Cow::Borrowed("tab\tstays")
    ));
    assert_eq!(single_line("\n"), m.to_string());
}

#[gpui::test]
fn multi_line_cells_paint_and_copy_raw(cx: &mut TestAppContext) {
    let config = GridConfig {
        cell_text_styler: Some(CellStyler(Arc::new(ColorAll))),
        ..GridConfig::default()
    };
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(data()).config(config).build(cx));
    // Painting used to trip the shaper's single-line debug assertion.
    cx.update(|window, _cx| window.refresh());
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, cx| {
            state.selection = Selection::Cell(0, 0);
            state.copy_selection(false, cx);
        });
    });
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("clipboard text");
    assert_eq!(
        copied,
        format!("{MULTI}\n"),
        "copy keeps the real line breaks"
    );
}
