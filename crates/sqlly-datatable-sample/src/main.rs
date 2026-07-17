//! Native entry point for the sample app. The whole application lives in the
//! library crate (`lib.rs`) so the identical app can also be built for the
//! web via `build-wasm.sh`; this binary just supplies the OS platform.

#[cfg(not(target_family = "wasm"))]
fn main() {
    gpui_platform::application()
        // Lucide icon SVGs for the grid's chrome (embedded in the binary).
        .with_assets(gpui_component_assets::Assets)
        .run(sqlly_datatable_sample::init_and_open);
}

/// The web build enters through `sqlly_datatable_sample::web::run` (see
/// `lib.rs`); this stub only keeps `--all-targets` compiles green on wasm,
/// where the native `Assets` embed above does not exist.
#[cfg(target_family = "wasm")]
fn main() {}
