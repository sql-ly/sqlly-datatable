//! Charting for result grids — a dedicated tab beside Grid and Pivot that
//! plots the grid's own rows, configured from a pivot-style sidebar.
//!
//! The module mirrors [`crate::pivot`]'s shape: a pure data engine
//! ([`model`]), a state entity ([`ChartState`]), a canvas widget
//! ([`ChartCanvas`]), a configuration sidebar ([`ChartSidebar`]), and an SVG
//! exporter ([`chart_svg`]). Click-to-navigate resolves a mark back to the
//! source rows behind it, which the host grid selects and reveals on the
//! Grid tab (see [`ChartState::request_navigate_at`]).

pub mod config;
pub mod model;
mod paint;
pub(crate) mod sidebar;
pub(crate) mod state;
mod svg;
pub(crate) mod widget;

pub use config::{
    ChartAggregate, ChartConfig, ChartError, ChartKind, ChartOrder, MAX_CATEGORIES, MAX_SERIES,
    MAX_SOURCE_ROWS, SVG_EXPORT_HEIGHT, SVG_EXPORT_WIDTH,
};
pub use model::{AxisScale, ChartData, ChartSeries, HistogramBins};
pub use paint::ChartPaint;
pub use sidebar::ChartSidebar;
pub use state::{ChartSaveConfigHandler, ChartState};
pub use svg::chart_svg;
pub use widget::{ChartCanvas, DEFAULT_CHART_SIDEBAR_WIDTH};
