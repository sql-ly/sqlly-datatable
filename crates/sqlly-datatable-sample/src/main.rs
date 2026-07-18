//! Native entry point for the sample app. The whole application lives in the
//! library crate (`lib.rs`); this binary just bootstraps GPUI.

fn main() {
    gpui::Application::new()
        // Lucide icon SVGs for the grid's chrome (embedded in the binary).
        .with_assets(gpui_component_assets::Assets)
        .run(sqlly_datatable_sample::init_and_open);
}
