/// Optional macOS bundling helper.
///
/// The previous version of this file silently spawned `bundle.sh` in the
/// background, which made `cargo build` surprises look like ghost processes.
/// The script is still useful but only when the user explicitly wants an
/// `.app` directory (e.g. before running `open SqllyDataTableSample.app`).
/// To trigger it, run `sh bundle.sh` from this directory after building.
fn main() {
    // Intentionally does nothing. See `bundle.sh` for the explicit invocation.
}
