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

use super::config::{
    ChartConfig, ChartError, ChartKind, MAX_SOURCE_ROWS, SVG_EXPORT_HEIGHT, SVG_EXPORT_WIDTH,
};
use super::model::{ChartData, ChartSeries};
use super::paint::{series_color, ChartPaint, PaintSeries};
use super::stream::{extract, extract_rows, Extraction, HistogramData, HistogramError};
use super::svg::chart_svg;
use crate::source::{
    progress_percent, scan_with_progress, RowSource, RowSourceDrill, RowSourceError, ScanControl,
};

/// A background chart extraction over a [`RowSource`]: queued by
/// [`ChartState::recompute`], started by the canvas, and applied by
/// [`ChartState::apply_scan`] if nothing newer replaced it.
pub(crate) struct ChartScanJob {
    generation: u64,
    pub(crate) control: Arc<ScanControl>,
    source: Arc<dyn RowSource>,
    columns: Vec<Column>,
    config: ChartConfig,
    /// The first rows, which decide the column roles; read by this job when
    /// no earlier job has.
    sample: Option<Arc<Vec<Vec<CellValue>>>>,
}

/// What a finished background extraction produced.
pub(crate) struct ChartScanOutput {
    generation: u64,
    sample: Arc<Vec<Vec<CellValue>>>,
    outcome: Result<Extraction, RowSourceError>,
}

impl ChartScanJob {
    /// [`Self::run`] once the source's rows are ready (see
    /// [`RowSource::ready`]).
    pub(crate) async fn run_when_ready(self) -> Option<ChartScanOutput> {
        if let Some(ready) = self.source.ready() {
            if let Err(error) = ready.await {
                self.control.finish();
                return Some(ChartScanOutput {
                    generation: self.generation,
                    sample: self.sample.clone().unwrap_or_default(),
                    outcome: Err(error),
                });
            }
        }
        self.run()
    }

    /// Read the sample (first job only), then extract over every row.
    /// `None` when cancelled. Blocking; runs on the background executor.
    pub(crate) fn run(self) -> Option<ChartScanOutput> {
        struct FinishOnDrop(Arc<ScanControl>);
        impl Drop for FinishOnDrop {
            fn drop(&mut self) {
                self.0.finish();
            }
        }
        let _finish = FinishOnDrop(Arc::clone(&self.control));
        let sample = match self.sample {
            Some(sample) => sample,
            None => match read_sample(&*self.source) {
                Ok(sample) => Arc::new(sample),
                Err(error) => {
                    return Some(ChartScanOutput {
                        generation: self.generation,
                        sample: Arc::new(Vec::new()),
                        outcome: Err(error),
                    })
                }
            },
        };
        let source = &*self.source;
        let control = &*self.control;
        let mut scan = |pass: usize, passes: usize, visit: &mut dyn FnMut(usize, &[CellValue])| {
            let mut index = 0usize;
            scan_with_progress(source, control, pass, passes, &mut |row| {
                visit(index, row);
                index += 1;
            })
        };
        let outcome = match extract(
            &self.columns,
            &sample,
            source.row_count(),
            &self.config,
            false,
            &mut scan,
        ) {
            Ok(Some(extraction)) => Ok(extraction),
            Ok(None) => return None,
            Err(error) => Err(error),
        };
        Some(ChartScanOutput {
            generation: self.generation,
            sample,
            outcome,
        })
    }
}

/// The first [`MAX_SOURCE_ROWS`] rows of `source`.
fn read_sample(source: &dyn RowSource) -> Result<Vec<Vec<CellValue>>, RowSourceError> {
    let mut sample = Vec::with_capacity(source.row_count().min(MAX_SOURCE_ROWS));
    source.scan(&mut |row| {
        sample.push(row.to_vec());
        sample.len() < MAX_SOURCE_ROWS
    })?;
    Ok(sample)
}

/// The background extraction currently queued or running.
struct ChartScan {
    control: Arc<ScanControl>,
    total: usize,
    /// Taken by the canvas when it starts the job.
    job: Option<ChartScanJob>,
}

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
    /// Shared snapshot of the grid's source rows (never mutated). Unused
    /// while [`Self::row_source`] is set.
    pub(crate) source_rows: Arc<Vec<Vec<CellValue>>>,
    /// The complete result when the grid holds only a window of it; every
    /// recompute then extracts from it on the background executor.
    row_source: Option<Arc<dyn RowSource>>,
    /// The first rows of `row_source`, which decide the column roles (and
    /// the sidebar's numeric columns). Read once per source.
    source_sample: Arc<Vec<Vec<CellValue>>>,
    /// The background extraction queued or running over `row_source`.
    scan: Option<ChartScan>,
    /// Bumped per queued scan; a finished scan applies only if it matches.
    scan_generation: u64,
    /// The last background extraction could not read the rows.
    scan_failed: bool,
    /// The histogram for the current source and config (histogram kind
    /// only), computed with the rest of the extraction rather than per paint.
    histogram: Option<Result<HistogramData, HistogramError>>,
    /// Source column labelling the categories in the current extraction.
    label_index: Option<usize>,
    /// The host handles drill-through over the row source; set by the host
    /// grid.
    pub(crate) source_drill_enabled: bool,
    /// A clicked mark's rows as a drill over the row source, drained by
    /// `SqllyDataTable::render` and handed to the host.
    pending_source_drill: Option<RowSourceDrill>,
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

impl Drop for ChartState {
    /// A chart that goes away stops its background read of the source.
    fn drop(&mut self) {
        self.cancel_scan();
    }
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
            row_source: None,
            source_sample: Arc::new(Vec::new()),
            scan: None,
            scan_generation: 0,
            scan_failed: false,
            histogram: None,
            label_index: None,
            source_drill_enabled: false,
            pending_source_drill: None,
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

    /// Chart `source` — the complete result — instead of the in-memory rows,
    /// or go back to the in-memory rows with `None`. Every recompute then
    /// reads `source` on the background executor while the previous chart
    /// stays up with a progress label. Click-to-navigate is unavailable
    /// meanwhile: the grid only holds a window of the rows.
    pub fn set_row_source(&mut self, columns: Vec<Column>, source: Option<Arc<dyn RowSource>>) {
        self.source_columns = columns;
        self.row_source = source;
        self.source_sample = Arc::new(Vec::new());
        self.recompute();
    }

    /// Whether this chart reads a [`RowSource`] rather than in-memory rows.
    #[must_use]
    pub fn has_row_source(&self) -> bool {
        self.row_source.is_some()
    }

    /// Whether `source` is the row source already installed.
    #[must_use]
    pub fn row_source_is(&self, source: &Arc<dyn RowSource>) -> bool {
        self.row_source
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, source))
    }

    /// Whether a background extraction is queued or running.
    #[must_use]
    pub fn is_scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// The label shown over the chart while a background extraction runs,
    /// or after one failed; `None` otherwise.
    #[must_use]
    pub fn scan_status(&self) -> Option<String> {
        if let Some(scan) = &self.scan {
            let percent = progress_percent(scan.control.scanned(), scan.total);
            return Some(crate::localization::format(
                "datatable.chart.scan.progress",
                &[
                    ("percent", &percent.to_string()),
                    ("total", &scan.total.to_string()),
                ],
            ));
        }
        self.scan_failed
            .then(|| crate::localization::text("datatable.chart.scan.failed"))
    }

    /// Take the queued background extraction for the canvas to run.
    pub(crate) fn take_pending_scan(&mut self) -> Option<ChartScanJob> {
        self.scan.as_mut().and_then(|scan| scan.job.take())
    }

    /// Install a finished background extraction, unless a newer one
    /// replaced it.
    pub(crate) fn apply_scan(&mut self, output: ChartScanOutput) {
        if output.generation != self.scan_generation {
            return;
        }
        self.scan = None;
        if !output.sample.is_empty() {
            self.source_sample = output.sample;
        }
        match output.outcome {
            Ok(extraction) => {
                self.scan_failed = false;
                self.outcome = extraction.data;
                self.histogram = extraction.histogram;
                self.label_index = extraction.label_index;
            }
            Err(_) => {
                self.scan_failed = true;
                self.outcome = Err(ChartError::NoRows);
                self.histogram = None;
            }
        }
        self.last_paint = None;
    }

    /// The rows that decide which columns are numeric or labels: the first
    /// [`MAX_SOURCE_ROWS`] rows of whichever source the chart reads.
    pub(crate) fn classification_rows(&self) -> &[Vec<CellValue>] {
        let rows: &[Vec<CellValue>] = if self.row_source.is_some() {
            &self.source_sample
        } else {
            &self.source_rows
        };
        &rows[..rows.len().min(MAX_SOURCE_ROWS)]
    }

    /// Re-run extraction against the current source + config. Called by
    /// every mutating entry point; invalidates the cached paint.
    pub(crate) fn recompute(&mut self) {
        self.config.clamp_to_columns(self.source_columns.len());
        self.last_paint = None;
        self.scan_failed = false;
        if let Some(source) = self.row_source.clone() {
            self.queue_scan(source);
            return;
        }
        self.cancel_scan();
        let extraction = extract_rows(&self.source_columns, &self.source_rows, &self.config);
        self.outcome = extraction.data;
        self.histogram = extraction.histogram;
        self.label_index = extraction.label_index;
    }

    /// Queue a background extraction over `source` for the current config,
    /// cancelling any earlier one. The current chart stays until it lands.
    fn queue_scan(&mut self, source: Arc<dyn RowSource>) {
        self.cancel_scan();
        self.scan_generation += 1;
        let control = ScanControl::new();
        let total = source.row_count();
        let job = ChartScanJob {
            generation: self.scan_generation,
            control: Arc::clone(&control),
            source,
            columns: self.source_columns.clone(),
            config: self.config.clone(),
            sample: (!self.source_sample.is_empty()).then(|| Arc::clone(&self.source_sample)),
        };
        self.scan = Some(ChartScan {
            control,
            total,
            job: Some(job),
        });
    }

    /// Stop the queued or running background extraction, if any.
    fn cancel_scan(&mut self) {
        if let Some(scan) = self.scan.take() {
            scan.control.cancel();
        }
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
    pub fn paint_for(&self) -> Result<ChartPaint, String> {
        if self.config.kind == ChartKind::Histogram {
            return self.paint_for_histogram();
        }
        let data = match &self.outcome {
            Ok(data) => data,
            // Nothing drawn yet: say what the background read is doing
            // instead of a "no rows" message.
            Err(error) => return Err(self.scan_status().unwrap_or_else(|| error.message())),
        };
        if matches!(self.config.kind, ChartKind::Pie | ChartKind::Donut) {
            self.paint_for_radial(data)
        } else {
            self.paint_for_cartesian(data)
        }
    }

    fn paint_for_histogram(&self) -> Result<ChartPaint, String> {
        if let Some(status) = self.scan_status().filter(|_| self.histogram.is_none()) {
            return Err(status);
        }
        let histogram = match &self.histogram {
            Some(Ok(histogram)) => histogram,
            Some(Err(error)) => return Err(error.message()),
            None => return Err(HistogramError::NoNumericColumn.message()),
        };
        Ok(ChartPaint::from_histogram(
            &histogram.name,
            &histogram.bins,
            histogram.bin_rows.clone(),
            &self.theme,
            self.font_size,
        ))
    }

    fn paint_for_radial(&self, data: &ChartData) -> Result<ChartPaint, String> {
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

    fn paint_for_cartesian(&self, data: &ChartData) -> Result<ChartPaint, String> {
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

    /// The extracted categories and series behind the chart, when there is
    /// anything to chart.
    #[must_use]
    pub fn data(&self) -> Option<&ChartData> {
        self.outcome.as_ref().ok()
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
        self.outcome.as_ref().is_ok_and(|d| {
            d.categories_truncated
                || d.series_truncated
                || d.categories_capped
                || d.downsampled_rows.is_some()
        })
    }

    /// The "Showing …" note describing every active cap, or `None` when
    /// nothing was dropped. Rendered above the canvas so a capped chart is
    /// never mistaken for the whole result set.
    #[must_use]
    pub fn truncation_note(&self) -> Option<String> {
        let data = self.outcome.as_ref().ok()?;
        let mut parts = Vec::new();
        if data.categories_truncated {
            let top_n = self.config.top_n.clamp(1, super::config::MAX_CATEGORIES);
            parts.push(crate::localization::format(
                "datatable.chart.truncation.top_categories",
                &[("count", &top_n.to_string())],
            ));
        }
        if data.series_truncated {
            parts.push(crate::localization::format(
                "datatable.chart.truncation.first_numeric_columns",
                &[("count", &super::config::MAX_SERIES.to_string())],
            ));
        }
        if data.categories_capped {
            parts.push(crate::localization::format(
                "datatable.chart.truncation.first_distinct_categories",
                &[("count", &super::stream::MAX_TRACKED_CATEGORIES.to_string())],
            ));
        }
        if let Some(rows) = data.downsampled_rows {
            parts.push(crate::localization::format(
                "datatable.chart.truncation.downsampled_shape",
                &[
                    ("rows", &rows.to_string()),
                    ("points", &data.categories.len().to_string()),
                ],
            ));
        }
        if parts.is_empty() {
            return None;
        }
        let details = parts.join(&crate::localization::text(
            "datatable.chart.truncation.component_separator",
        ));
        Some(crate::localization::format(
            "datatable.chart.truncation.showing",
            &[("details", &details)],
        ))
    }

    /// The value columns the chart actually plots: the configured list, or —
    /// when it is empty — the auto-selected set (every numeric column, capped
    /// at [`super::config::MAX_SERIES`]), mirroring extraction. The sidebar
    /// checks these rows so "auto" never looks like "nothing selected".
    #[must_use]
    pub fn effective_value_columns(&self) -> Vec<usize> {
        if !self.config.value_columns.is_empty() {
            return self.config.value_columns.clone();
        }
        let rows = self.classification_rows();
        (0..self.source_columns.len())
            .filter(|&index| super::model::is_numeric_column(index, rows))
            .take(super::config::MAX_SERIES)
            .collect()
    }

    /// The legend rows for the current kind: `(label, swatch color, hidden)`
    /// in display order — categories for radial kinds, series otherwise.
    /// Hidden entries are included (rendered dimmed/struck) so they can be
    /// toggled back on. Histograms label their bins on the axis and have no
    /// legend.
    #[must_use]
    pub(crate) fn legend_entries(&self) -> Vec<(String, gpui::Hsla, bool)> {
        if self.config.kind == ChartKind::Histogram {
            return Vec::new();
        }
        let Ok(data) = &self.outcome else {
            return Vec::new();
        };
        if matches!(self.config.kind, ChartKind::Pie | ChartKind::Donut) {
            data.categories
                .iter()
                .enumerate()
                .map(|(index, category)| {
                    (
                        category.clone(),
                        series_color(&self.theme, index),
                        self.hidden.contains(category),
                    )
                })
                .collect()
        } else {
            data.series
                .iter()
                .enumerate()
                .map(|(index, series)| {
                    (
                        series.name.clone(),
                        series_color(&self.theme, index),
                        self.hidden.contains(&series.name),
                    )
                })
                .collect()
        }
    }

    /// Name of the column labelling the categories, if extraction found one
    /// (the legend strip's "by …" tag).
    #[must_use]
    pub(crate) fn label_column_name(&self) -> Option<String> {
        self.outcome
            .as_ref()
            .ok()
            .and_then(|data| data.label_column.clone())
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
            // An aggregated mark over a row source: its rows were never
            // collected, so hand the host a test that finds them.
            if self.row_source.is_some() && self.source_drill_enabled {
                if let Some(drill) = self.mark_drill(&paint, category) {
                    self.pending_source_drill = Some(drill);
                    return true;
                }
            }
            return false;
        }
        self.pending_navigate = Some(rows);
        true
    }

    /// The rows behind mark `category` of `paint` as a test over full rows:
    /// the label column equals the category (bars, points, slices), or the
    /// value falls in the bin (histograms). `None` when the categories are
    /// row numbers, which have no column to test.
    fn mark_drill(&self, paint: &ChartPaint, category: usize) -> Option<RowSourceDrill> {
        let field_label = |column: usize, value: &str| {
            let name = self
                .source_columns
                .get(column)
                .map_or("?", |c| c.name.as_str());
            crate::localization::format(
                "datatable.grid.drill_filter.field_equals",
                &[("field_name", name), ("values", value)],
            )
        };
        if self.config.kind == ChartKind::Histogram {
            let Some(Ok(histogram)) = &self.histogram else {
                return None;
            };
            if category >= histogram.bins.counts.len() {
                return None;
            }
            let column = histogram.column;
            let bins = histogram.bins.clone();
            let label = field_label(column, &bins.label(category));
            return Some(RowSourceDrill::new(label, move |row| {
                row.get(column)
                    .and_then(super::model::numeric_cell)
                    .is_some_and(|value| bins.bin_for(value) == category)
            }));
        }
        let column = self.label_index?;
        let wanted = paint.categories.get(category)?.clone();
        let label = field_label(column, &wanted);
        Some(RowSourceDrill::new(label, move |row| {
            super::model::label_text(row.get(column)) == wanted
        }))
    }

    /// Take the queued drill over the row source, if any.
    pub(crate) fn take_pending_source_drill(&mut self) -> Option<RowSourceDrill> {
        self.pending_source_drill.take()
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
    fn truncation_note_names_the_active_caps(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        assert_eq!(s.truncation_note(), None, "nothing dropped, no note");
        s.set_config(ChartConfig {
            top_n: 2,
            ..ChartConfig::default()
        });
        assert_eq!(
            s.truncation_note().as_deref(),
            Some("Showing top 2 categories"),
            "the note reports the configured limit, not the hard cap"
        );
    }

    #[gpui::test]
    fn effective_value_columns_reflect_auto_selection(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        assert_eq!(
            s.effective_value_columns(),
            vec![1],
            "auto mode reports the numeric columns the extraction plots"
        );
        s.set_config(ChartConfig {
            value_columns: vec![1],
            ..ChartConfig::default()
        });
        assert_eq!(s.effective_value_columns(), vec![1]);
    }

    #[gpui::test]
    fn legend_entries_follow_kind_and_hidden_state(cx: &mut gpui::TestAppContext) {
        let mut s = state(cx, ChartConfig::default());
        let entries = s.legend_entries();
        assert_eq!(entries.len(), 1, "one series → one legend row");
        assert_eq!(entries[0].0, "score");
        assert!(!entries[0].2);
        assert_eq!(s.label_column_name().as_deref(), Some("name"));

        // Radial kinds list categories instead, and hidden entries stay
        // listed (dimmed) so they can be re-enabled.
        s.set_config(ChartConfig {
            kind: ChartKind::Pie,
            value_columns: vec![1],
            ..ChartConfig::default()
        });
        let entries = s.legend_entries();
        assert_eq!(entries.len(), 4, "four categories → four legend rows");
        s.toggle_legend_entry("a");
        let entries = s.legend_entries();
        assert_eq!(entries.len(), 4);
        assert!(entries
            .iter()
            .any(|(name, _, hidden)| name == "a" && *hidden));

        // Histograms have no legend — bins label themselves on the axis.
        s.set_config(ChartConfig {
            kind: ChartKind::Histogram,
            ..ChartConfig::default()
        });
        assert!(s.legend_entries().is_empty());
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
