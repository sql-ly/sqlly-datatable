#![allow(clippy::expect_used)]

use gpui::{point, px, Keystroke, TestAppContext};
use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridData, MenuAction, Selection, SortDirection, SqllyDataTable,
};

fn grouped_data() -> GridData {
    GridData::new(
        vec![
            Column::new("region", ColumnKind::Text, 120.0),
            Column::new("amount", ColumnKind::Integer, 100.0),
        ],
        vec![
            vec![CellValue::Text("East".into()), CellValue::Integer(30)],
            vec![CellValue::Text("West".into()), CellValue::Integer(20)],
            vec![CellValue::Text("East".into()), CellValue::Integer(10)],
        ],
    )
    .expect("rectangular grouped data")
}

#[gpui::test]
fn grouping_builds_expandable_sections_and_survives_sorting(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(grouped_data())
            .group_by_column(0)
            .build(cx)
    });
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            assert_eq!(state.grouped_column(), Some(0));
            assert_eq!(state.display_row_count(), 5);
            assert_eq!(state.row_groups().len(), 2);
            assert_eq!(state.row_groups()[0].label, "East");
            assert_eq!(state.row_groups()[0].row_count, 2);
            assert!(!state.row_groups()[0].collapsed);

            state.selection = Selection::Cell(1, 1);
            state.set_group_collapsed(0, true);
            assert!(state.row_groups()[0].collapsed);
            assert_eq!(state.display_row_count(), 3);
            assert_eq!(state.selection, Selection::None);

            state.selection = Selection::Cell(1, 0);
            state.sort = Some((1, SortDirection::Ascending));
            state.recompute();
            assert_eq!(state.selection, Selection::None);
            let east = state
                .row_groups()
                .iter()
                .find(|group| group.label == "East")
                .expect("East group remains present");
            assert!(east.collapsed);
            assert_eq!(east.row_count, 2);
        });
    });
}

#[gpui::test]
fn keyboard_and_public_selection_apis_skip_group_headers(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(grouped_data())
            .group_by_column(0)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            state.selection = Selection::Cell(1, 0);
            state.handle_key(&Keystroke {
                key: "down".into(),
                ..Default::default()
            });
            assert_eq!(state.selection, Selection::Cell(2, 0));

            state.handle_key(&Keystroke {
                key: "down".into(),
                ..Default::default()
            });
            assert_eq!(state.selection, Selection::Cell(4, 0));

            state.selection = Selection::Column(0);
            assert_eq!(state.selected_cells(), vec![(1, 0), (2, 0), (4, 0)]);
        });
    });
}

#[gpui::test]
fn appending_while_grouped_rebuilds_sections(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(grouped_data())
            .group_by_column(0)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            state
                .append_rows(vec![
                    vec![CellValue::Text("North".into()), CellValue::Integer(40)],
                    vec![CellValue::Text("East".into()), CellValue::Integer(50)],
                ])
                .expect("valid appended rows");

            assert_eq!(state.row_groups().len(), 3);
            assert_eq!(state.row_groups()[0].row_count, 3);
            assert_eq!(state.display_row_count(), 8);
        });
    });
}

#[gpui::test]
fn grouped_column_copy_omits_section_headers(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        SqllyDataTable::builder(grouped_data())
            .group_by_column(0)
            .build(cx)
    });

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, cx| {
            state.selection = Selection::Column(1);
            state.copy_selection(false, cx);
        });
    });
    let text = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .expect("clipboard has grouped column text");

    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        ["30.00", "10.00", "20.00"]
    );
}

#[gpui::test]
fn group_header_click_toggles_and_menu_action_can_clear_grouping(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(grouped_data()).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, cx| {
            state.execute_action(MenuAction::GroupBy, 0, cx);
            assert_eq!(state.grouped_column(), Some(0));

            let group_header = point(px(60.0), px(state.header_height + state.row_height * 0.5));
            state.handle_mouse_down(group_header, false);
            assert!(state.row_groups()[0].collapsed);

            state.execute_action(MenuAction::ClearGrouping, 0, cx);
            assert_eq!(state.grouped_column(), None);
            assert!(state.row_groups().is_empty());
            assert_eq!(state.display_row_count(), 3);
        });
    });
}

#[gpui::test]
fn grouping_is_disabled_for_windowed_rows(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(grouped_data()).build(cx));

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            let resident = state.data.rows[..2].to_vec();
            state
                .set_row_window(100, 40, resident)
                .expect("valid resident rows");
            state.set_grouped_column(Some(0));

            assert_eq!(state.grouped_column(), None);
            assert_eq!(state.display_row_count(), 100);
            assert_eq!(state.resident_row_for_display(40), Some(0));
        });
    });
}
