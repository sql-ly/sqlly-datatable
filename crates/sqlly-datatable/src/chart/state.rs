//! `ChartState` — runtime state for the chart tab: the extracted
//! [`ChartData`], the user's config, legend visibility, the last painted
//! snapshot (for hit testing), and the click-to-navigate request queue.
//!
//! The state holds an `Arc` snapshot of the source rows shared with the flat
//! grid, exactly like [`crate::pivot::PivotState`]. The source is never
//! mutated; every interaction re-runs the pure extraction in
//! [`super::model`].

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Bounds, FocusHandle, Pixels};

use crate::data::{CellValue, Column};
use crate::grid::theme::GridTheme;

use super::config::{ChartConfig, ChartError, ChartKind, SVG_EXPORT_HEIGHT, SVG_EXPORT_WIDTH};
use super::model::{histogram_bins, histogram_source, ChartData, ChartSeries};
use super::paint::{series_color, ChartPaint, PaintSeries};
use super::svg::chart_svg;

/// Callback invoked when the user clicks the sidebar's save-configuration
/// button. Receives the live [`ChartConfig`] to persist.
pub type ChartSaveConfigHandler = Rc<dyn Fn(&ChartConfig, &mut App)>;

/// Runtime state for one chart tab. Created by the host
/// [`crate::SqllyDataTable`] when a chart config is supplied; the chart
/// canvas and sidebar both render from this entity.
pub struct ChartState {
    /// User-controlled chart configuration. Mutating it directly does NOT
    /// recompute — use [`Self::set_config`] (or the `set_*` helpers) so the
    /// extracted data stays in sync.
    pub config: ChartConfig,
    /// Latest extraction outcome. `Err` carries the honest empty-state
    /// message rendered instead of a plot.
    pub(crate) outcome: Result<ChartData, ChartError>,
    /// Shared snapshot of the grid's source rows (never mutated).
    pub(crate) source_rows: Arc<Vec<Vec<CellValue>>>,
    /// Source column metadata.
    pub(crate) source_columns: Vec<Column>,
    /// Legend entries the user has hidden, by name. For radial kinds these
    /// are category names; otherwise series names.
    pub(crate) hidden: HashSet<String>,
    /// Registered save-configuration action. The sidebar's save button only
    /// renders while this is `Some`.
    pub(crate) save_config_handler: Option<ChartSaveConfigHandler>,
    /// Source-row indices a chart mark click resolved to. Drained by
    /// `SqllyDataTable::render`, which selects/reveals those rows on the
    /// Grid tab.
    pub(crate) pending_navigate: Option<Vec<usize>>,
    /// The paint snapshot from the most recent layout pass — the hit-testing
    /// surface for click-to-navigate.
    pub(crate) last_paint: Option<ChartPaint>,
    /// Painted canvas bounds, updated each layout pass.
    pub(crate) bounds: Bounds<Pixels>,
    /// Theme shared with the host grid.
    pub theme: GridTheme,
    /// Mirrors [`crate::GridConfig::animations`] (informational for the
    /// sidebar's transient surfaces).
    pub animations: bool,
    /// Font size for chart text (matches the grid's cell font).
    pub font_size: f32,
    /// Focus handle for keyboard input.
    pub focus_handle: FocusHandle,
    /// Current width of the chart controls sidebar, kept in sync by the host
    /// widget.
    pub(crate) sidebar_width: f32,
}

impl ChartState {
    /// Build a chart state over a shared source snapshot and run the initial
    /// extraction.
    #[must_use]
    pub fn new(
        source_columns: Vec<Column>,
        source_rows: Arc<Vec<Vec<CellValue>>>,
        config: ChartConfig,
        theme: GridTheme,
        animations: bool,
        font_size: f32,
        focus_handle: FocusHandle,
    ) -> Self {
        let mut state = Self {
            config,
            outcome: Err(ChartError::NoRows),
            source_rows,
            source_columns,
            hidden: HashSet::new(),
            save_config_handler: None,
            pending_navigate: None,
            last_paint: None,
            bounds: Bounds::default(),
            theme,
            animations,
            font_size,
            focus_handle,
            sidebar_width: super::widget::DEFAULT_CHART_SIDEBAR_WIDTH,
        };
        state.recompute();
        state
    }

    /// Replace the source snapshot (e.g. after the flat grid appended rows)
    /// and recompute. O(1) extra memory when `rows` shares the grid's Arc.
    pub fn set_source(&mut self, columns: Vec<Column>, rows: Arc<Vec<Vec<CellValue>>>) {
        self.source_columns = columns;
        self.source_rows = rows;
        self.recompute();
    }

    /// Whether `rows` is a different snapshot than the current source.
    pub(crate) fn source_differs(&self, rows: &Arc<Vec<Vec<CellValue>>>) -> bool {
        !Arc::ptr_eq(&self.source_rows, rows)
    }

    /// Re-run extraction against the current source + config. Called by
    /// every mutating entry point; invalidates the cached paint.
    pub(crate) fn recompute(&mut self) {
        self.config.clamp_to_columns(self.source_columns.len());
        self.outcome =
            ChartData::extract_with_config(&self.source_columns, &self.source_rows, &self.config);
        self.last_paint = None;
    }

    /// Replace the whole config and recompute.
    pub fn set_config(&mut self, config: ChartConfig) {
        self.config = config;
        self.recompute();
    }

    /// Switch the chart kind and recompute.
    pub fn set_kind(&mut self, kind: ChartKind) {
        if self.config.kind != kind {
            self.config.kind = kind;
            self.recompute();
        }
    }

    // ------------------------------------------------------------------
    // Legend
    // ------------------------------------------------------------------

    /// Show/hide one legend entry (a series name, or a category name for
    /// radial kinds). Refuses to hide the last visible entry.
    pub fn toggle_legend_entry(&mut self, label: &str) {
        let visible = self.visible_legend_count();
        if self.hidden.contains(label) {
            self.hidden.remove(label);
        } else if visible > 1 {
            self.hidden.insert(label.to_string());
        }
    }

    /// How many legend entries are currently visible (categories for radial
    /// kinds, series otherwise).
    #[must_use]
    pub fn visible_legend_count(&self) -> usize {
        let radial = matches!(self.config.kind, ChartKind::Pie | ChartKind::Donut);
        match self.outcome {
            Ok(ref data) if radial => data
                .categories
                .iter()
                .filter(|c| !self.hidden.contains(*c))
                .count(),
            Ok(ref data) => data
                .series
                .iter()
                .filter(|s| !self.hidden.contains(&s.name))
                .count(),
            Err(_) => 0,
        }
    }

    // ------------------------------------------------------------------
    // Paint / export
    // ------------------------------------------------------------------

    /// The theme-resolved paint snapshot for the current kind, with
    /// legend-hidden entries excluded. `Err` carries the message to render
    /// instead of a plot.
    pub fn paint_for(&self) -> Result<ChartPaint, &'static str> {
        if self.config.kind == ChartKind::Histogram {
            return self.paint_for_histogram();
        }
        let data = self.outcome.as_ref().map_err(|e| e.message())?;
        if matches!(self.config.kind, ChartKind::Pie | ChartKind::Donut) {
            self.paint_for_radial(data)
        } else {
            self.paint_for_cartesian(data)
        }
    }

    fn paint_for_histogram(&self) -> Result<ChartPaint, &'static str> {
        let (name, pairs) = histogram_source(&self.source_columns, &self.source_rows, &self.config)
            .ok_or("No numeric column to histogram")?;
        let bins = histogram_bins(&pairs.iter().map(|(v, _)| *v).collect::<Vec<_>>())
            .ok_or("No numeric values to histogram")?;
        let mut bin_rows: Vec<Vec<usize>> = vec![Vec::new(); bins.counts.len()];
        for (value, row) in &pairs {
            bin_rows[bins.bin_for(*value)].push(*row);
        }
        Ok(ChartPaint::from_histogram(
            &name,
            &bins,
            bin_rows,
            &self.theme,
            self.font_size,
        ))
    }

    fn paint_for_radial(&self, data: &ChartData) -> Result<ChartPaint, &'static str> {
        if let Some(message) = data.radial_error() {
            return Err(message);
        }
        // Hidden categories are excluded; slice colors stay keyed to the
        // ORIGINAL category index so hiding a slice never recolors the rest.
        let mut categories = Vec::new();
        let mut values = Vec::new();
        let mut slice_colors = Vec::new();
        let mut category_rows = Vec::new();
        {
            let series = &data.series[0];
            for (index, (category, value)) in data.categories.iter().zip(&series.values).enumerate()
            {
                if self.hidden.contains(category) {
                    continue;
                }
                categories.push(category.clone());
                values.push(*value);
                slice_colors.push(series_color(&self.theme, index));
                category_rows.push(data.category_rows[index].clone());
            }
        }
        let series = vec![PaintSeries {
            name: data
                .series
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            color: series_color(&self.theme, 0),
            values,
        }];
        Ok(ChartPaint::new(
            self.config.kind,
            categories,
            series,
            slice_colors,
            category_rows,
            &self.theme,
            self.font_size,
        ))
    }

    fn paint_for_cartesian(&self, data: &ChartData) -> Result<ChartPaint, &'static str> {
        // Hidden series are excluded; the rest keep their original colors.
        let visible: Vec<(usize, &ChartSeries)> = data
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| !self.hidden.contains(&s.name))
            .collect();
        let visible = if visible.is_empty() {
            // Defensive: the toggle guards against hiding the last series,
            // but a renamed column could leave the set stale.
            data.series.iter().enumerate().take(1).collect()
        } else {
            visible
        };
        let series = visible
            .iter()
            .map(|(index, s)| PaintSeries {
                name: s.name.clone(),
                color: series_color(&self.theme, *index),
                values: s.values.clone(),
            })
            .collect();
        Ok(ChartPaint::new(
            self.config.kind,
            data.categories.clone(),
            series,
            Vec::new(),
            data.category_rows.clone(),
            &self.theme,
            self.font_size,
        ))
    }

    /// The chart as a standalone SVG document at the standard export size,
    /// or `None` when there is nothing to chart.
    #[must_use]
    pub fn svg(&self) -> Option<String> {
        self.paint_for()
            .ok()
            .map(|paint| chart_svg(&paint, SVG_EXPORT_WIDTH, SVG_EXPORT_HEIGHT))
    }

    /// Whether the current source has anything chartable.
    #[must_use]
    pub fn is_chartable(&self) -> bool {
        self.outcome.is_ok()
    }

    /// The human-facing message shown instead of a plot (empty when the
    /// chart is renderable).
    #[must_use]
    pub fn empty_message(&self) -> String {
        match self.outcome {
            Ok(_) => String::new(),
            Err(ref error) => error.message().to_string(),
        }
    }

    /// How many series the extraction produced (before legend hiding).
    #[must_use]
    pub fn series_count(&self) -> usize {
        self.outcome.as_ref().map_or(0, |d| d.series.len())
    }

    /// How many categories the extraction produced (before legend hiding).
    #[must_use]
    pub fn category_count(&self) -> usize {
        self.outcome.as_ref().map_or(0, |d| d.categories.len())
    }

    /// Whether any truncation flag is set (drives the "Showing …" note).
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.outcome
            .as_ref()
            .is_ok_and(|d| d.categories_truncated || d.series_truncated || d.source_truncated)
    }

    // ------------------------------------------------------------------
    // Click-to-navigate
    // ------------------------------------------------------------------

    /// Resolve a click at canvas-relative `(x, y)` against the last painted
    /// snapshot and queue the driving source rows for the host widget to
    /// select/reveal on the Grid tab. Returns whether a mark was hit.
    /// Inert while [`ChartConfig::navigate_on_click`] is off.
    pub fn request_navigate_at(&mut self, x: f32, y: f32) -> bool {
        if !self.config.navigate_on_click {
            return false;
        }
        let Some(paint) = self.last_paint.clone() else {
            return false;
        };
        let width = f32::from(self.bounds.size.width);
        let height = f32::from(self.bounds.size.height);
        let Some((category, _)) = paint.hit_test(width, height, x, y) else {
            return false;
        };
        let rows = paint
            .category_rows
            .get(category)
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            return false;
        }
        self.pending_navigate = Some(rows);
        true
    }

    /// Take the queued navigation rows, if any. Called by the host widget.
    pub(crate) fn take_pending_navigate(&mut self) -> Option<Vec<usize>> {
        self.pending_navigate.take()
    }

    // ------------------------------------------------------------------
    // Save configuration
    // ------------------------------------------------------------------

    /// Register (or replace) the save-configuration action. While registered,
    /// the sidebar renders a save button that invokes the handler with the
    /// live [`ChartConfig`].
    pub fn on_save_config(&mut self, handler: impl Fn(&ChartConfig, &mut App) + 'static) {
        self.save_config_handler = Some(Rc::new(handler));
    }

    /// Remove the save-configuration action; the sidebar's save button
    /// disappears.
    pub fn clear_save_config_handler(&mut self) {
        self.save_config_handler = None;
    }

    /// Whether a save-configuration action is currently registered.
    #[must_use]
    pub fn has_save_config_handler(&self) -> bool {
        self.save_config_handler.is_some()
    }

    /// The registered save-configuration action, if any.
    #[must_use]
    pub fn save_config_handler(&self) -> Option<ChartSaveConfigHandler> {
        self.save_config_handler.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::data::{Column, ColumnKind};

    fn sample_columns() -> Vec<Column> {
        vec![
            Column::new("name", ColumnKind::Text, 120.0),
            Column::new("score", ColumnKind::Integer, 90.0),
        ]
    }

    fn sample_rows() -> Vec<Vec<CellValue>> {
        ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                vec![
                    CellValue::Text((*name).to_string()),
                    CellValue::Integer((i * 10 + 10) as i64),
                ]
            })
            .collect()
    }

    fn state(cx: &gpui::TestAppContext, config: ChartConfig) -> ChartState {
        let focus = cx.update(|cx| cx.focus_handle());
        ChartState::new(
            sample_columns(),
            Arc::new(sample_rows()),
            config,
            crate::grid::theme::GridTheme::default(),
            false,
            14.0,
            focus,
        )
    }

    #[gpui::test]
    fn navigate_click_gated_by_config(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        let paint = s.paint_for().expect("chartable");
        s.bounds = Bounds {
            origin: gpui::point(px0(), px0()),
            size: gpui::size(pxv(400.0), pxv(300.0)),
        };
        s.last_paint = Some(paint);

        // Disabled by config: clicks resolve nothing.
        s.config.navigate_on_click = false;
        assert!(!s.request_navigate_at(200.0, 150.0));
        assert!(s.take_pending_navigate().is_none());

        // Enabled: a click in the middle of the plot hits a category slot
        // and queues exactly its source rows.
        s.config.navigate_on_click = true;
        assert!(s.request_navigate_at(200.0, 150.0));
        let rows = s.take_pending_navigate().expect("navigate queued");
        assert_eq!(rows.len(), 1, "per-row categories carry one row each");
        assert!(rows[0] < 4);

        // The queue drains — a second take is empty.
        assert!(s.take_pending_navigate().is_none());
    }

    #[gpui::test]
    fn navigate_click_outside_plot_misses(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        let paint = s.paint_for().expect("chartable");
        s.bounds = Bounds {
            origin: gpui::point(px0(), px0()),
            size: gpui::size(pxv(400.0), pxv(300.0)),
        };
        s.last_paint = Some(paint);
        // Far left gutter tick area and far below the plot: no hit.
        assert!(!s.request_navigate_at(2.0, 150.0));
        assert!(!s.request_navigate_at(200.0, 299.0));
        assert!(s.take_pending_navigate().is_none());
    }

    #[gpui::test]
    fn navigate_without_paint_snapshot_is_inert(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        assert!(s.last_paint.is_none());
        assert!(!s.request_navigate_at(200.0, 150.0));
        assert!(s.take_pending_navigate().is_none());
    }

    #[gpui::test]
    fn histogram_navigates_to_bin_rows(cx: &mut gpui::TestAppContext) {
        let mut s = state(
            cx,
            ChartConfig {
                kind: ChartKind::Histogram,
                ..ChartConfig::default()
            },
        );
        let paint = s.paint_for().expect("histogram over score");
        assert_eq!(paint.categories.len(), paint.category_rows.len());
        let total: usize = paint.category_rows.iter().map(Vec::len).sum();
        assert_eq!(total, 4, "every source row lands in exactly one bin");
        s.bounds = Bounds {
            origin: gpui::point(px0(), px0()),
            size: gpui::size(pxv(400.0), pxv(300.0)),
        };
        s.last_paint = Some(paint);
        assert!(s.request_navigate_at(200.0, 150.0));
        let rows = s.take_pending_navigate().expect("bin navigate queued");
        assert!(!rows.is_empty() && rows.len() <= 4);
    }

    #[gpui::test]
    fn radial_hit_test_resolves_slices(cx: &mut gpui::TestAppContext) {
        let mut s = state(
            cx,
            ChartConfig {
                kind: ChartKind::Pie,
                value_columns: vec![1],
                ..ChartConfig::default()
            },
        );
        let paint = s.paint_for().expect("pie over one non-negative series");
        s.bounds = Bounds {
            origin: gpui::point(px0(), px0()),
            size: gpui::size(pxv(400.0), pxv(300.0)),
        };
        s.last_paint = Some(paint);
        // Values 10/20/30/40: slices at 12, 3, 6 (ish), and 9 o'clock.
        // Clicking just above center hits the first (12 o'clock) slice.
        assert!(s.request_navigate_at(200.0, 120.0));
        let rows = s.take_pending_navigate().expect("slice hit");
        assert_eq!(rows, vec![0]);
        // The donut hole is a miss on a donut chart.
        s.config.kind = ChartKind::Donut;
        let paint = s.paint_for().expect("donut");
        s.last_paint = Some(paint);
        assert!(!s.request_navigate_at(200.0, 150.0), "hole misses");
    }

    #[gpui::test]
    fn legend_toggle_guards_last_visible_entry(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        s.set_kind(ChartKind::Pie);
        s.set_config(ChartConfig {
            kind: ChartKind::Pie,
            value_columns: vec![1],
            ..ChartConfig::default()
        });
        let radial_count = s.visible_legend_count();
        assert!(radial_count >= 2);
        // Hide all but one; the last one refuses to hide.
        for _ in 1..radial_count {
            let name = s
                .outcome
                .as_ref()
                .expect("outcome")
                .categories
                .iter()
                .find(|c| !s.hidden.contains(*c))
                .cloned()
                .expect("a visible category");
            s.toggle_legend_entry(&name);
        }
        assert_eq!(s.visible_legend_count(), 1);
        let last = s
            .outcome
            .as_ref()
            .expect("outcome")
            .categories
            .iter()
            .find(|c| !s.hidden.contains(*c))
            .cloned()
            .expect("the last visible category");
        s.toggle_legend_entry(&last);
        assert_eq!(s.visible_legend_count(), 1, "last entry stays visible");
    }

    #[gpui::test]
    fn empty_message_reflects_extraction_error(cx: &mut gpui::TestAppContext) {
        let focus = cx.update(|cx| cx.focus_handle());
        let s = ChartState::new(
            vec![Column::new("name", ColumnKind::Text, 120.0)],
            Arc::new(vec![vec![CellValue::Text("only text".into())]]),
            ChartConfig::default(),
            crate::grid::theme::GridTheme::default(),
            false,
            14.0,
            focus,
        );
        assert!(!s.is_chartable());
        assert_eq!(s.empty_message(), "No numeric columns to chart");
        assert!(s.paint_for().is_err());
        assert!(s.svg().is_none());
    }

    fn px0() -> gpui::Pixels {
        gpui::px(0.0)
    }
    fn pxv(v: f32) -> gpui::Pixels {
        gpui::px(v)
    }
}
