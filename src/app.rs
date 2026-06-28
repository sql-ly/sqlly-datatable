use crate::data::sample_data;
use crate::grid::{GridState, GridView};
use gpui::prelude::*;
use gpui::{px, size, App, Bounds, WindowBounds, WindowOptions};

pub fn run() {
    let application = gpui::Application::new();
    application.run(move |cx: &mut App| {
        cx.activate(true);

        let focus = cx.focus_handle();
        let data = sample_data();
        let state = cx.new(|_cx| GridState::new(data, focus.clone()));

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

        let state_for_window = state.clone();
        let window = cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| GridView::new(state_for_window.clone()))
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
