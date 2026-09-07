//! [`ChartConfig`] — the complete, GPUI-free description of a chart view.
//!
//! The struct is plain data, mirroring [`crate::pivot::PivotConfig`]:
//! reading it back *is* the "current configuration" API, and constructing or
//! mutating one is the programmatic preconfiguration API. The sidebar edits
//! the same struct the host passes in, so interactive changes and code paths
//! cannot diverge.

/// Most series a chart will draw. More than this and the legend/grouped bars
/// stop being readable; extra numeric columns are dropped with a visible note.
pub const MAX_SERIES: usize = 4;
/// Most categories (rows) a chart will draw. Past this a bar is thinner than
/// a pixel anyway; extra rows are dropped with a visible note.
pub const MAX_CATEGORIES: usize = 200;
/// Most visible source rows inspected for configuration and aggregation.
pub const MAX_SOURCE_ROWS: usize = 10_000;
/// Width of the exported standalone SVG document.
pub const SVG_EXPORT_WIDTH: f32 = 960.0;
/// Height of the exported standalone SVG document.
pub const SVG_EXPORT_HEIGHT: f32 = 540.0;

/// Why a result set cannot be charted right now. The message text is
/// user-facing — it is rendered verbatim in the chart's empty state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartError {
    /// The result set has no rows.
    NoRows,
    /// No column (of those eligible) is numeric, so there is nothing to plot.
    NoNumericColumns,
}

impl ChartError {
    /// The user-facing message for this error.
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            ChartError::NoRows => "No rows to chart",
            ChartError::NoNumericColumns => "No numeric columns to chart",
        }
    }
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// The chart presentation. Bars are the default because grouped bars stay
/// honest for categorical labels; radial kinds (pie, donut) place extra
/// constraints checked at paint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// Grouped bars around a zero baseline.
    Bar,
    /// Per-series polylines with gaps at NULLs.
    Line,
    /// Filled polylines on a zero-floored axis.
    Area,
    /// Per-category point markers.
    Scatter,
    /// Contiguous nice-edged bins of one numeric column.
    Histogram,
    /// Radial shares of a single non-negative series.
    Pie,
    /// [`ChartKind::Pie`] with the center hollowed out.
    Donut,
}

impl ChartKind {
    /// Every kind, in sidebar display order.
    #[must_use]
    pub fn all() -> &'static [ChartKind] {
        &[
            ChartKind::Bar,
            ChartKind::Line,
            ChartKind::Area,
            ChartKind::Scatter,
            ChartKind::Histogram,
            ChartKind::Pie,
            ChartKind::Donut,
        ]
    }

    /// The sidebar label for this kind.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ChartKind::Bar => "Bar",
            ChartKind::Line => "Line",
            ChartKind::Area => "Area",
            ChartKind::Scatter => "Scatter",
            ChartKind::Histogram => "Histogram",
            ChartKind::Pie => "Pie",
            ChartKind::Donut => "Donut",
        }
    }
}

/// How multiple source rows collapse into one category value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartAggregate {
    /// No collapsing: every source row is its own category.
    None,
    /// Sum of the category's numeric values.
    Sum,
    /// Mean of the category's numeric values.
    Average,
    /// Smallest value in the category.
    Minimum,
    /// Largest value in the category.
    Maximum,
    /// Count of non-null values in the category.
    Count,
}

impl ChartAggregate {
    /// Every aggregate, in sidebar display order.
    #[must_use]
    pub fn all() -> &'static [ChartAggregate] {
        &[
            ChartAggregate::None,
            ChartAggregate::Sum,
            ChartAggregate::Average,
            ChartAggregate::Minimum,
            ChartAggregate::Maximum,
            ChartAggregate::Count,
        ]
    }

    /// The sidebar label for this aggregate.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ChartAggregate::None => "None",
            ChartAggregate::Sum => "Sum",
            ChartAggregate::Average => "Average",
            ChartAggregate::Minimum => "Minimum",
            ChartAggregate::Maximum => "Maximum",
            ChartAggregate::Count => "Count",
        }
    }
}

/// How categories are ordered along the axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartOrder {
    /// Keep the source order.
    Source,
    /// Sort by category label, ascending.
    CategoryAscending,
    /// Sort by the first series' value, highest first.
    ValueDescending,
}

impl ChartOrder {
    /// Every order, in sidebar display order.
    #[must_use]
    pub fn all() -> &'static [ChartOrder] {
        &[
            ChartOrder::Source,
            ChartOrder::CategoryAscending,
            ChartOrder::ValueDescending,
        ]
    }

    /// The sidebar label for this order.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ChartOrder::Source => "Source",
            ChartOrder::CategoryAscending => "Category A–Z",
            ChartOrder::ValueDescending => "Value high–low",
        }
    }
}

/// Field assignments plus display options for a chart view.
///
/// All field references are **source column indices** into the grid's
/// `GridData::columns`, exactly like [`crate::pivot::PivotConfig`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChartConfig {
    /// The chart presentation.
    pub kind: ChartKind,
    /// The column whose values label the categories. `None` auto-detects the
    /// first string-ish column, falling back to 1-based row indices.
    pub label_column: Option<usize>,
    /// The numeric columns to plot as series, in draw order. Empty
    /// auto-selects every numeric column (capped at [`MAX_SERIES`]).
    pub value_columns: Vec<usize>,
    /// How multiple source rows collapse into one category value.
    pub aggregate: ChartAggregate,
    /// How categories are ordered along the axis.
    pub order: ChartOrder,
    /// Most categories drawn, after ordering.
    pub top_n: usize,
    /// Whether clicking a mark in the chart navigates to the flat grid rows
    /// that drive it (switching to the Grid tab with those rows selected).
    /// The sidebar exposes this as a checkbox.
    pub navigate_on_click: bool,
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            kind: ChartKind::Bar,
            label_column: None,
            value_columns: Vec::new(),
            aggregate: ChartAggregate::None,
            order: ChartOrder::Source,
            top_n: MAX_CATEGORIES,
            navigate_on_click: true,
        }
    }
}

impl ChartConfig {
    /// The next [`top_n`] value in the sidebar's limit cycle.
    #[must_use]
    pub fn next_top_n(&self) -> usize {
        match self.top_n {
            10 => 25,
            25 => 50,
            50 => 100,
            100 => MAX_CATEGORIES,
            _ => 10,
        }
    }

    /// Drop every field reference that points outside `0..column_count`.
    /// Call after the source schema changes.
    pub fn clamp_to_columns(&mut self, column_count: usize) {
        if self.label_column.is_some_and(|c| c >= column_count) {
            self.label_column = None;
        }
        self.value_columns.retain(|c| *c < column_count);
        self.top_n = self.top_n.clamp(1, MAX_CATEGORIES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_navigates_and_autodetects() {
        let config = ChartConfig::default();
        assert!(config.navigate_on_click, "click-to-navigate defaults on");
        assert_eq!(config.kind, ChartKind::Bar);
        assert_eq!(config.label_column, None);
        assert!(config.value_columns.is_empty());
        assert_eq!(config.top_n, MAX_CATEGORIES);
    }

    #[test]
    fn clamp_to_columns_drops_out_of_range_fields() {
        let mut config = ChartConfig {
            label_column: Some(4),
            value_columns: vec![1, 9, 2],
            top_n: 500,
            ..ChartConfig::default()
        };
        config.clamp_to_columns(3);
        assert_eq!(config.label_column, None, "out-of-range label is cleared");
        assert_eq!(config.value_columns, vec![1, 2]);
        assert_eq!(config.top_n, MAX_CATEGORIES, "top_n clamps to the cap");
    }

    #[test]
    fn top_n_cycles_through_the_ladder() {
        let mut config = ChartConfig::default();
        let ladder = [MAX_CATEGORIES, 10, 25, 50, 100, MAX_CATEGORIES, 10];
        for expected in ladder {
            assert_eq!(config.top_n, expected);
            config.top_n = config.next_top_n();
        }
    }
}
