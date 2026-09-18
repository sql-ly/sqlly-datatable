//! Header tooltips: hovering a column header for the process-wide tooltip
//! delay surfaces host-configured text (`ColumnOverride::header_tooltip`) —
//! e.g. the source-column provenance of a result column.

#![allow(clippy::expect_used)]

use std::time::Duration;

use gpui::{point, px, TestAppContext};
use sqlly_datatable::{
    set_tooltip_show_delay, tooltip_show_delay, CellValue, Column, ColumnKind, ColumnOverride,
    GridConfig, GridData, SqllyDataTable,
};

fn data() -> GridData {
    GridData::new(
        vec![
            Column::new("PermissionName", ColumnKind::Text, 120.0),
            Column::new("ModelName", ColumnKind::Text, 120.0),
        ],
        vec![vec![
            CellValue::Text("CREATE".into()),
            CellValue::Text("Reference".into()),
        ]],
    )
    .expect("rectangular data")
}

#[gpui::test]
fn header_dwell_surfaces_the_configured_tooltip(cx: &mut TestAppContext) {
    let config = GridConfig {
        column_overrides: vec![ColumnOverride {
            header_tooltip: Some("Source: p.Name from Shared.Permission p".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (view, cx) =
        cx.add_window_view(|_window, cx| SqllyDataTable::builder(data()).config(config).build(cx));
    cx.run_until_parked();

    view.update(cx, |table, cx| {
        table.state.update(cx, |state, _cx| {
            // Process-global delay: capture and restore so parallel tests are
            // unaffected.
            let original = tooltip_show_delay();

            // Resting on the first header starts the dwell clock; nothing
            // shows before the delay elapses.
            set_tooltip_show_delay(Duration::from_secs(3600));
            state.handle_mouse_move(point(px(80.0), px(10.0)), None);
            assert!(state.header_hover.is_some(), "dwell clock must start");
            assert!(state.header_tooltip_pending());
            assert!(state.header_tooltip_ready().is_none());

            // Once the delay is met (zero here), the tooltip is ready with
            // the host's text.
            set_tooltip_show_delay(Duration::ZERO);
            let (col, text) = state
                .header_tooltip_ready()
                .expect("ready once the dwell delay is met");
            assert_eq!(col, 0);
            assert_eq!(text, "Source: p.Name from Shared.Permission p");
            assert!(!state.header_tooltip_pending());

            // A column without configured text never becomes ready, and never
            // asks for a dwell timer.
            state.handle_mouse_move(point(px(200.0), px(10.0)), None);
            assert!(state.header_hover.is_some());
            assert!(state.header_tooltip_ready().is_none());
            assert!(!state.header_tooltip_pending());

            // Moving between headers restarts the clock on the new column.
            set_tooltip_show_delay(Duration::from_secs(3600));
            state.handle_mouse_move(point(px(80.0), px(10.0)), None);
            let (col, started) = state.header_hover.expect("hover tracked");
            assert_eq!(col, 0);
            state.handle_mouse_move(point(px(200.0), px(10.0)), None);
            let (col, restarted) = state.header_hover.expect("hover tracked");
            assert_eq!(col, 1);
            assert!(restarted >= started);

            // Leaving the header row clears the dwell entirely.
            state.handle_mouse_move(point(px(80.0), px(200.0)), None);
            assert!(state.header_hover.is_none());
            assert!(!state.header_tooltip_pending());

            set_tooltip_show_delay(original);
        });
    });
}
