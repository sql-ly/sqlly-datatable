//! Native entry point for the sample app. The whole application lives in the
//! library crate (`lib.rs`); this binary just bootstraps GPUI.

fn main() {
    // `gpui_platform::application()` selects the OS windowing backend; the
    // registry gpui bundled the backends behind `Application::new()`, but the
    // zed git tree splits them into per-platform crates behind this facade.
    gpui_platform::application()
        // Lucide icon SVGs for the grid's chrome (embedded in the binary).
        .with_assets(gpui_component_assets::Assets)
        .run(sqlly_datatable_sample::init_and_open);
}
