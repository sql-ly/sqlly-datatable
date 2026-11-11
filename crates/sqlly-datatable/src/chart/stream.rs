//! Chart extraction over rows that stream by once, so a chart can cover a
//! result of any size with memory bounded by what it draws.
//!
//! The column roles (which columns are numeric, which label the categories)
//! come from a sample: the first [`MAX_SOURCE_ROWS`] rows. Every row then
//! feeds the chart, kept in bounded state per ordering:
//!
//! * aggregated, source order — the first `top_n` categories seen;
//! * aggregated, by category — the `top_n` smallest labels (a label pushed
//!   out can never come back, because the bound only tightens);
//! * aggregated, by value — every category up to [`MAX_TRACKED_CATEGORIES`];
//! * per row, source order — the first `top_n` rows, except line, area and
//!   scatter charts of more rows than that, which keep each row bucket's
//!   minimum and maximum so the drawn shape keeps its spikes and dips;
//! * per row, by value or by category — the best `top_n` rows.
//!
//! Histograms read the rows twice: once for the range, once to count bins.

use std::collections::{BTreeMap, HashMap};

use crate::data::{CellValue, Column};
use crate::source::RowSourceError;

use super::config::{
    ChartAggregate, ChartConfig, ChartError, ChartKind, ChartOrder, MAX_CATEGORIES, MAX_SERIES,
    MAX_SOURCE_ROWS,
};
use super::model::{
    classify_column, histogram_layout, label_text, numeric_cell, ChartData, ChartSeries,
    ColumnRole, HistogramBins,
};

/// Most distinct categories tracked when ordering aggregated categories by
/// value; past this, later categories are left out and the note says so.
pub const MAX_TRACKED_CATEGORIES: usize = 100_000;

/// One pass over the rows: `(pass, passes, visit)`. `visit` receives each
/// row with its position. Returns `Ok(false)` when the pass was cancelled.
pub(crate) type Scan<'a> = dyn FnMut(usize, usize, &mut dyn FnMut(usize, &[CellValue])) -> Result<bool, RowSourceError>
    + 'a;

/// Why a histogram cannot be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistogramError {
    /// No numeric column to bin.
    NoNumericColumn,
    /// The numeric column holds no values.
    NoNumericValues,
}

impl HistogramError {
    pub(crate) fn message(self) -> String {
        match self {
            HistogramError::NoNumericColumn => {
                crate::localization::text("datatable.chart.error.no_numeric_column_to_histogram")
            }
            HistogramError::NoNumericValues => {
                crate::localization::text("datatable.chart.error.no_numeric_values_to_histogram")
            }
        }
    }
}

/// A histogram ready to paint: the binned column's name, the bins, and (for
/// in-memory rows) the rows inside each bin.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistogramData {
    /// Source column the bins count.
    pub(crate) column: usize,
    pub(crate) name: String,
    pub(crate) bins: HistogramBins,
    pub(crate) bin_rows: Vec<Vec<usize>>,
}

/// Everything one extraction produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Extraction {
    pub(crate) data: Result<ChartData, ChartError>,
    /// Present only for [`ChartKind::Histogram`].
    pub(crate) histogram: Option<Result<HistogramData, HistogramError>>,
    /// Source column labelling the categories; `None` when categories are
    /// row numbers.
    pub(crate) label_index: Option<usize>,
}

/// Which columns play which part, decided from the sample.
struct Plan {
    value_columns: Vec<usize>,
    series_truncated: bool,
    label_index: Option<usize>,
    histogram_column: Option<usize>,
}

fn plan(columns: &[Column], sample: &[Vec<CellValue>], config: &ChartConfig) -> Plan {
    let sample = &sample[..sample.len().min(MAX_SOURCE_ROWS)];
    let column_count = columns
        .len()
        .max(sample.iter().map(Vec::len).max().unwrap_or(0));
    let roles: Vec<ColumnRole> = (0..column_count)
        .map(|i| classify_column(i, sample))
        .collect();
    let numeric: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, r)| **r == ColumnRole::Numeric)
        .map(|(i, _)| i)
        .collect();
    let value_columns: Vec<usize> = if config.value_columns.is_empty() {
        numeric.clone()
    } else {
        config
            .value_columns
            .iter()
            .copied()
            .filter(|index| numeric.contains(index))
            .collect()
    };
    let series_truncated = value_columns.len() > MAX_SERIES;
    let label_index = config
        .label_column
        .filter(|index| *index < column_count)
        .or_else(|| roles.iter().position(|role| *role == ColumnRole::Label));
    let histogram_column = config
        .value_columns
        .iter()
        .copied()
        .find(|index| numeric.contains(index))
        .or_else(|| numeric.first().copied());
    Plan {
        value_columns: value_columns.into_iter().take(MAX_SERIES).collect(),
        series_truncated,
        label_index,
        histogram_column,
    }
}

/// Run one extraction. `sample` holds the first rows (at least the first
/// [`MAX_SOURCE_ROWS`] when there are that many); `total_rows` is how many
/// rows a pass visits. `track_rows` keeps the source rows behind each mark
/// for click-to-navigate — only sensible for in-memory rows. `Ok(None)`
/// when cancelled.
pub(crate) fn extract(
    columns: &[Column],
    sample: &[Vec<CellValue>],
    total_rows: usize,
    config: &ChartConfig,
    track_rows: bool,
    scan: &mut Scan<'_>,
) -> Result<Option<Extraction>, RowSourceError> {
    let histogram_kind = config.kind == ChartKind::Histogram;
    if total_rows == 0 {
        return Ok(Some(Extraction {
            data: Err(ChartError::NoRows),
            histogram: histogram_kind.then_some(Err(HistogramError::NoNumericColumn)),
            label_index: None,
        }));
    }
    let plan = plan(columns, sample, config);
    let label_index = plan.label_index;
    let label_for_row = |row_index: usize, row: &[CellValue]| match label_index {
        Some(column) => label_text(row.get(column)),
        None => (row_index + 1).to_string(),
    };

    let mut collector = (!plan.value_columns.is_empty())
        .then(|| Collector::new(config, plan.value_columns.len(), total_rows, track_rows));
    let mut range = histogram_kind
        .then_some(plan.histogram_column)
        .flatten()
        .map(|column| (column, Range::default()));
    let passes = if range.is_some() { 2 } else { 1 };

    let complete = scan(0, passes, &mut |row_index, row| {
        if let Some(collector) = collector.as_mut() {
            let values: Vec<Option<f64>> = plan
                .value_columns
                .iter()
                .map(|column| row.get(*column).and_then(numeric_cell))
                .collect();
            collector.push(row_index, || label_for_row(row_index, row), values);
        }
        if let Some((column, range)) = range.as_mut() {
            if let Some(value) = row.get(*column).and_then(numeric_cell) {
                range.add(value);
            }
        }
    })?;
    if !complete {
        return Ok(None);
    }

    let data = match collector {
        None => Err(ChartError::NoNumericColumns),
        Some(collector) => {
            let (categories, values, category_rows, notes) = collector.finish();
            let series = plan
                .value_columns
                .iter()
                .zip(values)
                .map(|(&col, values)| ChartSeries {
                    name: series_name(columns, col),
                    values,
                })
                .collect();
            Ok(ChartData {
                label_column: label_index.and_then(|i| columns.get(i).map(|c| c.name.clone())),
                categories,
                series,
                category_rows,
                categories_truncated: notes.categories_truncated,
                series_truncated: plan.series_truncated,
                categories_capped: notes.categories_capped,
                downsampled_rows: notes.downsampled_rows,
            })
        }
    };

    let histogram = if !histogram_kind {
        None
    } else {
        match range {
            None => Some(Err(HistogramError::NoNumericColumn)),
            Some((column, range)) => match bin_rows(column, &range, track_rows, passes, scan)? {
                None => return Ok(None),
                Some(Some((bins, bin_rows))) => Some(Ok(HistogramData {
                    column,
                    name: series_name(columns, column),
                    bins,
                    bin_rows,
                })),
                Some(None) => Some(Err(HistogramError::NoNumericValues)),
            },
        }
    };
    Ok(Some(Extraction {
        data,
        histogram,
        label_index,
    }))
}

/// The second histogram pass: count each value into the bins laid out from
/// `range`. `Some(None)` when the column held no numeric values.
#[allow(clippy::type_complexity)]
fn bin_rows(
    column: usize,
    range: &Range,
    track_rows: bool,
    passes: usize,
    scan: &mut Scan<'_>,
) -> Result<Option<Option<(HistogramBins, Vec<Vec<usize>>)>>, RowSourceError> {
    let Some(layout) = histogram_layout(range.min, range.max, range.count) else {
        return Ok(Some(None));
    };
    let mut bins = HistogramBins {
        edges: layout.edges.clone(),
        counts: vec![0; layout.edges.len() - 1],
    };
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); bins.counts.len()];
    let complete = scan(1, passes, &mut |row_index, row| {
        if let Some(value) = row.get(column).and_then(numeric_cell) {
            let bin = layout.bin_for(value);
            bins.counts[bin] += 1;
            if track_rows {
                rows[bin].push(row_index);
            }
        }
    })?;
    Ok(complete.then_some(Some((bins, rows))))
}

fn series_name(columns: &[Column], col: usize) -> String {
    columns
        .get(col)
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            crate::localization::format(
                "datatable.chart.series.fallback_name",
                &[("column_number", &(col + 1).to_string())],
            )
        })
}

/// Running range of a histogram column's finite values.
#[derive(Debug)]
struct Range {
    min: f64,
    max: f64,
    count: usize,
}

impl Default for Range {
    fn default() -> Self {
        Self {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            count: 0,
        }
    }
}

impl Range {
    fn add(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.count += 1;
    }
}

// ---------------------------------------------------------------------------
// Collectors
// ---------------------------------------------------------------------------

/// What the drawn data left out, for the "Showing …" note.
#[derive(Debug, Default)]
struct Notes {
    categories_truncated: bool,
    categories_capped: bool,
    downsampled_rows: Option<usize>,
}

/// Categories, per-series values, rows behind each category, and notes.
type Finished = (Vec<String>, Vec<Vec<Option<f64>>>, Vec<Vec<usize>>, Notes);

enum Collector {
    Aggregate(AggregateCollector),
    Rows(RowCollector),
    Downsample(DownsampleCollector),
}

impl Collector {
    fn new(config: &ChartConfig, series: usize, total_rows: usize, track_rows: bool) -> Self {
        let top_n = config.top_n.clamp(1, MAX_CATEGORIES);
        if config.aggregate != ChartAggregate::None {
            return Collector::Aggregate(AggregateCollector {
                order: config.order,
                aggregate: config.aggregate,
                top_n,
                series,
                track_rows,
                groups: Vec::new(),
                index: HashMap::new(),
                by_label: BTreeMap::new(),
                truncated: false,
                capped: false,
            });
        }
        let shape_kind = matches!(
            config.kind,
            ChartKind::Line | ChartKind::Area | ChartKind::Scatter
        );
        if config.order == ChartOrder::Source && shape_kind && total_rows > top_n {
            // Per-row marks keep their row however rows arrive: one number
            // per drawn point, and it lets a click jump to the row.
            return Collector::Downsample(DownsampleCollector::new(top_n, total_rows, true));
        }
        Collector::Rows(RowCollector {
            order: config.order,
            top_n,
            track_rows: true,
            kept: Vec::new(),
            seen: 0,
        })
    }

    fn push(&mut self, row: usize, label: impl FnOnce() -> String, values: Vec<Option<f64>>) {
        match self {
            Collector::Aggregate(c) => c.push(row, label, &values),
            Collector::Rows(c) => c.push(row, label, values),
            Collector::Downsample(c) => c.push(row, label, values),
        }
    }

    fn finish(self) -> Finished {
        match self {
            Collector::Aggregate(c) => c.finish(),
            Collector::Rows(c) => c.finish(),
            Collector::Downsample(c) => c.finish(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct AggregateCell {
    count: usize,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
}

impl AggregateCell {
    fn add(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;
        self.min = Some(self.min.map_or(value, |minimum| minimum.min(value)));
        self.max = Some(self.max.map_or(value, |maximum| maximum.max(value)));
    }

    fn finish(self, aggregate: ChartAggregate) -> Option<f64> {
        match aggregate {
            ChartAggregate::None => None,
            ChartAggregate::Sum => (self.count > 0).then_some(self.sum),
            ChartAggregate::Average => (self.count > 0).then(|| self.sum / self.count as f64),
            ChartAggregate::Minimum => self.min,
            ChartAggregate::Maximum => self.max,
            ChartAggregate::Count => Some(self.count as f64),
        }
    }
}

struct Group {
    label: String,
    cells: Vec<AggregateCell>,
    rows: Vec<usize>,
}

impl Group {
    fn new(label: String, series: usize) -> Self {
        Self {
            label,
            cells: vec![AggregateCell::default(); series],
            rows: Vec::new(),
        }
    }

    fn add(&mut self, row: usize, values: &[Option<f64>], track_rows: bool) {
        if track_rows {
            self.rows.push(row);
        }
        for (cell, value) in self.cells.iter_mut().zip(values) {
            if let Some(value) = value {
                cell.add(*value);
            }
        }
    }
}

/// Aggregated categories (every row collapses into its label's group).
struct AggregateCollector {
    order: ChartOrder,
    aggregate: ChartAggregate,
    top_n: usize,
    series: usize,
    track_rows: bool,
    /// Source and by-value orders: groups in first-seen order.
    groups: Vec<Group>,
    index: HashMap<String, usize>,
    /// By-category order: the `top_n` smallest labels.
    by_label: BTreeMap<String, Group>,
    truncated: bool,
    capped: bool,
}

impl AggregateCollector {
    fn push(&mut self, row: usize, label: impl FnOnce() -> String, values: &[Option<f64>]) {
        let label = label();
        if self.order == ChartOrder::CategoryAscending {
            if let Some(group) = self.by_label.get_mut(&label) {
                group.add(row, values, self.track_rows);
                return;
            }
            if self.by_label.len() >= self.top_n {
                self.truncated = true;
                let largest_is_bigger = self
                    .by_label
                    .last_key_value()
                    .is_some_and(|(largest, _)| *largest > label);
                if !largest_is_bigger {
                    return;
                }
                self.by_label.pop_last();
            }
            let mut group = Group::new(label.clone(), self.series);
            group.add(row, values, self.track_rows);
            self.by_label.insert(label, group);
            return;
        }
        if let Some(&index) = self.index.get(&label) {
            self.groups[index].add(row, values, self.track_rows);
            return;
        }
        let limit = match self.order {
            ChartOrder::Source => self.top_n,
            _ => MAX_TRACKED_CATEGORIES,
        };
        if self.groups.len() >= limit {
            self.truncated = true;
            self.capped |= self.order != ChartOrder::Source;
            return;
        }
        let mut group = Group::new(label.clone(), self.series);
        group.add(row, values, self.track_rows);
        self.index.insert(label, self.groups.len());
        self.groups.push(group);
    }

    fn finish(self) -> Finished {
        let aggregate = self.aggregate;
        let mut groups: Vec<Group> = if self.order == ChartOrder::CategoryAscending {
            self.by_label.into_values().collect()
        } else {
            self.groups
        };
        if self.order == ChartOrder::ValueDescending {
            groups.sort_by(|left, right| {
                let left = left.cells.first().and_then(|cell| cell.finish(aggregate));
                let right = right.cells.first().and_then(|cell| cell.finish(aggregate));
                right
                    .partial_cmp(&left)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let truncated = self.truncated || groups.len() > self.top_n;
        groups.truncate(self.top_n);
        let values = (0..self.series)
            .map(|series| {
                groups
                    .iter()
                    .map(|group| group.cells[series].finish(aggregate))
                    .collect()
            })
            .collect();
        let categories = groups.iter().map(|group| group.label.clone()).collect();
        let rows = groups.into_iter().map(|group| group.rows).collect();
        (
            categories,
            values,
            rows,
            Notes {
                categories_truncated: truncated,
                categories_capped: self.capped,
                downsampled_rows: None,
            },
        )
    }
}

/// One source row kept as its own category.
struct KeptRow {
    row: usize,
    label: String,
    values: Vec<Option<f64>>,
}

/// Per-row categories (no aggregation), keeping the best `top_n` rows.
struct RowCollector {
    order: ChartOrder,
    top_n: usize,
    track_rows: bool,
    kept: Vec<KeptRow>,
    seen: usize,
}

impl RowCollector {
    fn push(&mut self, row: usize, label: impl FnOnce() -> String, values: Vec<Option<f64>>) {
        self.seen += 1;
        if self.order == ChartOrder::Source && self.kept.len() >= self.top_n {
            return;
        }
        self.kept.push(KeptRow {
            row,
            label: label(),
            values,
        });
        // Keep at most twice the bound between compactions: sorting and
        // cutting back to `top_n` keeps the work O(rows · log top_n).
        if self.kept.len() >= self.top_n * 2 {
            self.compact();
        }
    }

    fn compact(&mut self) {
        match self.order {
            ChartOrder::Source => {}
            // Stable sorts keep source order among ties, as a full sort would.
            ChartOrder::CategoryAscending => self.kept.sort_by(|a, b| a.label.cmp(&b.label)),
            ChartOrder::ValueDescending => self.kept.sort_by(|a, b| {
                let left = a.values.first().copied().flatten();
                let right = b.values.first().copied().flatten();
                right
                    .partial_cmp(&left)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        self.kept.truncate(self.top_n);
    }

    fn finish(mut self) -> Finished {
        self.compact();
        let series = self.kept.first().map_or(0, |row| row.values.len());
        let values = (0..series)
            .map(|s| self.kept.iter().map(|row| row.values[s]).collect())
            .collect();
        let rows = self
            .kept
            .iter()
            .map(|row| {
                if self.track_rows {
                    vec![row.row]
                } else {
                    Vec::new()
                }
            })
            .collect();
        let categories = self.kept.into_iter().map(|row| row.label).collect();
        (
            categories,
            values,
            rows,
            Notes {
                categories_truncated: self.seen > self.top_n,
                categories_capped: false,
                downsampled_rows: None,
            },
        )
    }
}

/// Line/area/scatter shape of more rows than can be drawn: rows split into
/// `top_n / 2` consecutive buckets, each drawn as its minimum and maximum
/// (by the first series) in row order. A bucket without a numeric value
/// keeps its first row, drawn as a gap.
struct DownsampleCollector {
    buckets: usize,
    total_rows: usize,
    track_rows: bool,
    current: Option<Bucket>,
    out: Vec<KeptRow>,
    seen: usize,
}

struct Bucket {
    index: usize,
    first: Option<KeptRow>,
    min: Option<(f64, KeptRow)>,
    max: Option<(f64, KeptRow)>,
}

impl DownsampleCollector {
    fn new(top_n: usize, total_rows: usize, track_rows: bool) -> Self {
        Self {
            buckets: (top_n / 2).max(1),
            total_rows: total_rows.max(1),
            track_rows,
            current: None,
            out: Vec::new(),
            seen: 0,
        }
    }

    fn push(&mut self, row: usize, label: impl FnOnce() -> String, values: Vec<Option<f64>>) {
        self.seen += 1;
        // Bucket by the row's position among `total_rows`; past the expected
        // count (a source that grew), everything joins the last bucket.
        let position = self.seen - 1;
        let index = (position.min(self.total_rows - 1) * self.buckets / self.total_rows)
            .min(self.buckets - 1);
        if self.current.as_ref().is_some_and(|b| b.index != index) {
            self.flush();
        }
        let bucket = self.current.get_or_insert(Bucket {
            index,
            first: None,
            min: None,
            max: None,
        });
        let key = values.first().copied().flatten();
        let lower = key.is_some_and(|v| bucket.min.as_ref().is_none_or(|(m, _)| v < *m));
        let higher = key.is_some_and(|v| bucket.max.as_ref().is_none_or(|(m, _)| v > *m));
        if !(lower || higher || bucket.first.is_none()) {
            return;
        }
        let kept = KeptRow {
            row,
            label: label(),
            values,
        };
        match key {
            Some(v) => {
                if lower && higher {
                    bucket.max = Some((
                        v,
                        KeptRow {
                            row: kept.row,
                            label: kept.label.clone(),
                            values: kept.values.clone(),
                        },
                    ));
                    bucket.min = Some((v, kept));
                } else if lower {
                    bucket.min = Some((v, kept));
                } else if higher {
                    bucket.max = Some((v, kept));
                } else {
                    bucket.first = Some(kept);
                }
            }
            None => bucket.first = Some(kept),
        }
    }

    fn flush(&mut self) {
        let Some(bucket) = self.current.take() else {
            return;
        };
        match (bucket.min, bucket.max) {
            (Some((_, min)), Some((_, max))) => {
                if min.row == max.row {
                    self.out.push(min);
                } else if min.row < max.row {
                    self.out.push(min);
                    self.out.push(max);
                } else {
                    self.out.push(max);
                    self.out.push(min);
                }
            }
            (Some((_, only)), None) | (None, Some((_, only))) => self.out.push(only),
            (None, None) => self.out.extend(bucket.first),
        }
    }

    fn finish(mut self) -> Finished {
        self.flush();
        let series = self.out.first().map_or(0, |row| row.values.len());
        let values = (0..series)
            .map(|s| self.out.iter().map(|row| row.values[s]).collect())
            .collect();
        let rows = self
            .out
            .iter()
            .map(|row| {
                if self.track_rows {
                    vec![row.row]
                } else {
                    Vec::new()
                }
            })
            .collect();
        let categories = self.out.into_iter().map(|row| row.label).collect();
        (
            categories,
            values,
            rows,
            Notes {
                categories_truncated: false,
                categories_capped: false,
                downsampled_rows: Some(self.seen),
            },
        )
    }
}

/// [`extract`] over rows already in memory, tracking the rows behind each
/// mark. Never cancelled.
pub(crate) fn extract_rows(
    columns: &[Column],
    rows: &[Vec<CellValue>],
    config: &ChartConfig,
) -> Extraction {
    let mut scan = |_pass: usize, _passes: usize, visit: &mut dyn FnMut(usize, &[CellValue])| {
        for (index, row) in rows.iter().enumerate() {
            visit(index, row);
        }
        Ok::<bool, RowSourceError>(true)
    };
    match extract(columns, rows, rows.len(), config, true, &mut scan) {
        Ok(Some(extraction)) => extraction,
        // In-memory rows neither fail nor cancel.
        _ => Extraction {
            data: Err(ChartError::NoRows),
            histogram: None,
            label_index: None,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::data::ColumnKind;

    fn columns() -> Vec<Column> {
        vec![
            Column::new("label", ColumnKind::Text, 80.0),
            Column::new("value", ColumnKind::Decimal, 80.0),
        ]
    }

    fn rows(
        n: usize,
        label: impl Fn(usize) -> String,
        value: impl Fn(usize) -> f64,
    ) -> Vec<Vec<CellValue>> {
        (0..n)
            .map(|i| vec![CellValue::Text(label(i)), CellValue::Decimal(value(i))])
            .collect()
    }

    fn config(kind: ChartKind, aggregate: ChartAggregate, order: ChartOrder) -> ChartConfig {
        ChartConfig {
            kind,
            aggregate,
            order,
            ..ChartConfig::default()
        }
    }

    /// The pre-streaming algorithm (aggregate everything, sort, truncate),
    /// as the reference the bounded collectors must match.
    fn reference_aggregate(
        rows: &[Vec<CellValue>],
        config: &ChartConfig,
    ) -> (Vec<String>, Vec<Option<f64>>) {
        let mut order: Vec<String> = Vec::new();
        let mut cells: HashMap<String, AggregateCell> = HashMap::new();
        for row in rows {
            let label = label_text(row.first());
            let entry = cells.entry(label.clone()).or_insert_with(|| {
                order.push(label);
                AggregateCell::default()
            });
            if let Some(v) = row.get(1).and_then(numeric_cell) {
                entry.add(v);
            }
        }
        let value = |label: &String| cells[label].finish(config.aggregate);
        match config.order {
            ChartOrder::Source => {}
            ChartOrder::CategoryAscending => order.sort(),
            ChartOrder::ValueDescending => order.sort_by(|a, b| {
                value(b)
                    .partial_cmp(&value(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        order.truncate(config.top_n);
        let values = order.iter().map(value).collect();
        (order, values)
    }

    #[test]
    fn bounded_aggregation_matches_full_aggregation_for_every_order() {
        // 5,000 rows over 700 labels in scrambled first-seen order.
        let data = rows(
            5_000,
            |i| format!("k{:03}", (i * 7919) % 700),
            |i| ((i * 31) % 97) as f64 - 40.0,
        );
        for order in [
            ChartOrder::Source,
            ChartOrder::CategoryAscending,
            ChartOrder::ValueDescending,
        ] {
            for aggregate in [
                ChartAggregate::Sum,
                ChartAggregate::Average,
                ChartAggregate::Count,
            ] {
                let mut cfg = config(ChartKind::Bar, aggregate, order);
                cfg.top_n = 25;
                let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
                let (labels, values) = reference_aggregate(&data, &cfg);
                assert_eq!(got.categories, labels, "{order:?} {aggregate:?}");
                assert_eq!(got.series[0].values, values, "{order:?} {aggregate:?}");
                assert!(got.categories_truncated);
            }
        }
    }

    #[test]
    fn category_rows_cover_each_kept_category() {
        let data = rows(300, |i| format!("g{}", i % 3), |i| i as f64);
        let cfg = config(
            ChartKind::Bar,
            ChartAggregate::Sum,
            ChartOrder::CategoryAscending,
        );
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert_eq!(got.categories, vec!["g0", "g1", "g2"]);
        assert_eq!(got.category_rows[1].len(), 100);
        assert!(got.category_rows[1].iter().all(|r| r % 3 == 1));
    }

    #[test]
    fn per_row_orders_keep_the_best_rows_stably() {
        let data = rows(1_000, |i| format!("r{:04}", 999 - i), |i| (i % 10) as f64);
        let mut cfg = config(
            ChartKind::Bar,
            ChartAggregate::None,
            ChartOrder::ValueDescending,
        );
        cfg.top_n = 10;
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert_eq!(got.series[0].values, vec![Some(9.0); 10]);
        // Ties keep source order: rows 9, 19, 29, …
        assert_eq!(got.category_rows[0], vec![9]);
        assert_eq!(got.category_rows[1], vec![19]);

        cfg.order = ChartOrder::CategoryAscending;
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert_eq!(got.categories[0], "r0000");
        assert_eq!(got.categories[9], "r0009");
        assert!(got.categories_truncated);
    }

    #[test]
    fn long_line_keeps_its_spikes_in_bounded_points() {
        let n = 100_000;
        let data = rows(
            n,
            |i| format!("r{i}"),
            |i| {
                if i == 73_123 {
                    1_000.0
                } else if i == 12_345 {
                    -1_000.0
                } else {
                    (i % 50) as f64
                }
            },
        );
        let cfg = config(ChartKind::Line, ChartAggregate::None, ChartOrder::Source);
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert!(got.categories.len() <= MAX_CATEGORIES);
        assert_eq!(got.downsampled_rows, Some(n));
        assert!(!got.categories_truncated);
        let values: Vec<f64> = got.series[0].values.iter().flatten().copied().collect();
        assert!(values.contains(&1_000.0), "the spike survives");
        assert!(values.contains(&-1_000.0), "the dip survives");
        // Points stay in row order.
        let order: Vec<usize> = got.category_rows.iter().map(|r| r[0]).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn short_line_and_long_bar_are_not_downsampled() {
        let data = rows(150, |i| i.to_string(), |i| i as f64);
        let cfg = config(ChartKind::Line, ChartAggregate::None, ChartOrder::Source);
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert_eq!(got.categories.len(), 150);
        assert_eq!(got.downsampled_rows, None);

        let data = rows(1_000, |i| i.to_string(), |i| i as f64);
        let cfg = config(ChartKind::Bar, ChartAggregate::None, ChartOrder::Source);
        let got = extract_rows(&columns(), &data, &cfg).data.unwrap();
        assert_eq!(got.categories.len(), MAX_CATEGORIES);
        assert!(got.categories_truncated);
        assert_eq!(got.downsampled_rows, None);
    }

    #[test]
    fn histogram_bins_every_row_in_two_passes() {
        let data = rows(50_000, |i| format!("r{i}"), |i| (i % 1_000) as f64);
        let cfg = config(
            ChartKind::Histogram,
            ChartAggregate::None,
            ChartOrder::Source,
        );
        let histogram = extract_rows(&columns(), &data, &cfg)
            .histogram
            .unwrap()
            .unwrap();
        assert_eq!(histogram.bins.counts.iter().sum::<usize>(), 50_000);
        assert_eq!(histogram.name, "value");
        assert_eq!(
            histogram.bin_rows.iter().map(Vec::len).sum::<usize>(),
            50_000
        );
    }

    #[test]
    fn streamed_extraction_without_row_tracking_matches_in_memory() {
        let data = rows(20_000, |i| format!("g{}", i % 40), |i| (i % 13) as f64);
        let cfg = config(
            ChartKind::Bar,
            ChartAggregate::Average,
            ChartOrder::ValueDescending,
        );
        let in_memory = extract_rows(&columns(), &data, &cfg).data.unwrap();
        let mut passes_seen = Vec::new();
        let mut scan = |pass: usize, passes: usize, visit: &mut dyn FnMut(usize, &[CellValue])| {
            passes_seen.push((pass, passes));
            for (i, row) in data.iter().enumerate() {
                visit(i, row);
            }
            Ok::<bool, RowSourceError>(true)
        };
        let sample = &data[..MAX_SOURCE_ROWS.min(data.len())];
        let streamed = extract(&columns(), sample, data.len(), &cfg, false, &mut scan)
            .unwrap()
            .unwrap()
            .data
            .unwrap();
        assert_eq!(streamed.categories, in_memory.categories);
        assert_eq!(streamed.series, in_memory.series);
        assert!(streamed.category_rows.iter().all(Vec::is_empty));
        assert_eq!(passes_seen, vec![(0, 1)]);
    }

    #[test]
    fn cancelled_scan_yields_nothing() {
        let data = rows(10, |i| i.to_string(), |i| i as f64);
        let cfg = ChartConfig::default();
        let mut scan = |_: usize, _: usize, _: &mut dyn FnMut(usize, &[CellValue])| {
            Ok::<bool, RowSourceError>(false)
        };
        assert_eq!(
            extract(&columns(), &data, data.len(), &cfg, false, &mut scan),
            Ok(None)
        );
    }

    #[test]
    fn failed_scan_is_an_error() {
        let data = rows(10, |i| i.to_string(), |i| i as f64);
        let cfg = ChartConfig::default();
        let mut scan = |_: usize, _: usize, _: &mut dyn FnMut(usize, &[CellValue])| {
            Err::<bool, RowSourceError>(RowSourceError("disk gone".into()))
        };
        assert!(extract(&columns(), &data, data.len(), &cfg, false, &mut scan).is_err());
    }
}
