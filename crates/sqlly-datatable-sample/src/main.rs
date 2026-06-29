use gpui::prelude::*;
use gpui::{px, size, App, Bounds, WindowBounds, WindowOptions};
use sqlly_datatable::{
    ColumnKind, ColumnOverride, GridConfig, GridData, NumberFormat, SqllyDataTable,
};
use sqlly_datatable::data::sample_data;

fn main() {
    let application = gpui::Application::new();
    application.run(move |cx: &mut App| {
        cx.activate(true);

        let data = sample_data();
        let config = sample_config(&data);
        let view = SqllyDataTable::builder(data)
            .config(config)
            .build(cx);

        let focus = view.state.read(cx).focus_handle.clone();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1200.0), px(700.0)),
                cx,
            ))),
            titlebar: Some(Default::default()),
            is_movable: true,
            is_resizable: true,
            window_min_size: Some(size(px(600.0), px(400.0))),
            ..Default::default()
        };

        let state = view.state.clone();
        let window = cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| SqllyDataTable::new(state.clone()))
        });
        if let Ok(window) = window {
            window.update(cx, |_view, window, _cx| {
                window.focus(&focus);
                window.on_window_should_close(_cx, |_window, cx| {
                    cx.quit();
                    true
                });
            }).ok();
        }
    });
}

fn sample_config(data: &GridData) -> GridConfig {
    let mut config = GridConfig::default();
    let mut overrides = vec![ColumnOverride::default(); data.columns.len()];

    for (i, col) in data.columns.iter().enumerate() {
        match col.kind {
            ColumnKind::Integer => {
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 0,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            ColumnKind::Decimal => {
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 4,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            _ => {}
        }
    }

    config.column_overrides = overrides;
    config
}
