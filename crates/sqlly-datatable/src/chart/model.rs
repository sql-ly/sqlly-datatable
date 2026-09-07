//! Chart data extraction and numeric scaffolding: turning a rectangular
//! result set into categories, series, a nice-number value axis, and
//! histogram bins.
//!
//! Everything in this module is pure — no gpui, no theme, no window — so it is
//! trivially testable and shared by the painter, the SVG exporter, and the
//! click-to-navigate hit-testing path.

use crate::data::{CellValue, Column};

use super::config::{
    ChartAggregate, ChartConfig, ChartError, ChartKind, ChartOrder, MAX_CATEGORIES, MAX_SERIES,
    MAX_SOURCE_ROWS,
};

/// A cell's numeric value, if it has one. Covers typed numbers plus
/// numeric-looking strings (wire layers often ship decimal/money values as
/// canonical strings) — except booleans: charting a bit column as 0/1 bars is
/// noise, so bools are not treated as numeric here.
fn numeric_cell(value: &CellValue) -> Option<f64> {
    match value {
        CellValue::Integer(v) => Some(*v as f64),
        CellValue::Decimal(v) => Some(*v).filter(|f| f.is_finite()),
        CellValue::Text(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

/// What a column can contribute to the chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnRole {
    /// Every non-null cell is numeric (typed or numeric-looking string).
    Numeric,
    /// String-ish with at least one non-numeric value — a label candidate.
    Label,
    /// Bools, dates, mixed junk, or all-NULL — contributes nothing.
    Other,
}

/// Classify one column by scanning its cells. NULLs and blank strings are
/// ignored (they become gaps); an all-NULL column is `Other` — there is no
/// honest way to know what it would have held.
fn classify_column(index: usize, rows: &[Vec<CellValue>]) -> ColumnRole {
    let mut non_null = 0usize;
    let mut numeric = 0usize;
    let mut texty = false;
    for row in rows {
        let Some(value) = row.get(index) else {
            continue;
        };
        match value {
            CellValue::None => {}
            CellValue::Text(s) if s.trim().is_empty() => {}
            other => {
                non_null += 1;
                if numeric_cell(other).is_some() {
                    numeric += 1;
                }
                if matches!(other, CellValue::Text(_)) {
                    texty = true;
                }
            }
        }
    }
    if non_null == 0 {
        ColumnRole::Other
    } else if numeric == non_null {
        ColumnRole::Numeric
    } else if texty {
        ColumnRole::Label
    } else {
        ColumnRole::Other
    }
}

/// A cell rendered as a category label.
fn label_text(value: Option<&CellValue>) -> String {
    match value {
        None | Some(CellValue::None) => "NULL".to_string(),
        Some(CellValue::Boolean(b)) => b.to_string(),
        Some(CellValue::Integer(v)) => v.to_string(),
        Some(CellValue::Decimal(v)) => v.to_string(),
        Some(CellValue::Date(v)) => v.to_string(),
        Some(CellValue::Text(s)) => s.clone(),
    }
}

/// Whether column `index` is entirely numeric over `rows` — drives the
/// sidebar's Values list.
pub(crate) fn is_numeric_column(index: usize, rows: &[Vec<CellValue>]) -> bool {
    classify_column(index, rows) == ColumnRole::Numeric
}

#[derive(Clone, Copy, Default)]
struct AggregateCell {
    count: usize,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
}

/// Grouped aggregate output: distinct categories in first-seen order, the
/// per-series aggregate values, and the source-row indices per group.
type Aggregated = (Vec<String>, Vec<Vec<Option<f64>>>, Vec<Vec<usize>>);

/// Group `rows` by the label returned from `label_for_row` and fold each
/// group's configured value columns into one aggregate per (category, series).
///
/// Returns the distinct categories in first-seen order, the per-series
/// aggregate values, and — for click-to-navigate — the source-row indices
/// that fed each category.
fn aggregate_rows(
    rows: &[Vec<CellValue>],
    label_for_row: &dyn Fn(usize, &Vec<CellValue>) -> String,
    value_columns: &[usize],
    aggregate: ChartAggregate,
) -> Aggregated {
    let mut categories = Vec::new();
    let mut category_indices = std::collections::HashMap::new();
    let mut groups: Vec<Vec<AggregateCell>> = Vec::new();
    let mut group_rows: Vec<Vec<usize>> = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let category = label_for_row(row_index, row);
        let group_index = if let Some(index) = category_indices.get(&category) {
            *index
        } else {
            let index = categories.len();
            category_indices.insert(category.clone(), index);
            categories.push(category);
            groups.push(vec![AggregateCell::default(); value_columns.len()]);
            group_rows.push(Vec::new());
            index
        };
        group_rows[group_index].push(row_index);
        for (series_index, column_index) in value_columns.iter().enumerate() {
            let Some(value) = row.get(*column_index).and_then(numeric_cell) else {
                continue;
            };
            let cell = &mut groups[group_index][series_index];
            cell.count += 1;
            cell.sum += value;
            cell.min = Some(cell.min.map_or(value, |minimum| minimum.min(value)));
            cell.max = Some(cell.max.map_or(value, |maximum| maximum.max(value)));
        }
    }
    let values = (0..value_columns.len())
        .map(|series_index| {
            groups
                .iter()
                .map(|group| {
                    let cell = group[series_index];
                    match aggregate {
                        ChartAggregate::None => None,
                        ChartAggregate::Sum => (cell.count > 0).then_some(cell.sum),
                        ChartAggregate::Average => {
                            (cell.count > 0).then(|| cell.sum / cell.count as f64)
                        }
                        ChartAggregate::Minimum => cell.min,
                        ChartAggregate::Maximum => cell.max,
                        ChartAggregate::Count => Some(cell.count as f64),
                    }
                })
                .collect()
        })
        .collect();
    (categories, values, group_rows)
}

/// One charted series: the column it came from and its per-category values
/// (`None` marks a gap — a null or non-numeric cell).
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    /// Display name (source column name).
    pub name: String,
    /// One value per category; `None` is a gap.
    pub values: Vec<Option<f64>>,
}

/// The chartable projection of a result set: categories, series, and the
/// source rows behind every category.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartData {
    /// Name of the column used for category labels, if one was found.
    pub label_column: Option<String>,
    /// Category labels in display order.
    pub categories: Vec<String>,
    /// Series in display order (capped at [`MAX_SERIES`]).
    pub series: Vec<ChartSeries>,
    /// Source-row indices feeding each displayed category, in display order.
    /// This is what click-to-navigate resolves a mark against.
    pub category_rows: Vec<Vec<usize>>,
    /// More categories existed than the configured top-N allowed.
    pub categories_truncated: bool,
    /// More numeric columns existed than [`MAX_SERIES`] allows.
    pub series_truncated: bool,
    /// More source rows existed than [`MAX_SOURCE_ROWS`] allows.
    pub source_truncated: bool,
}

impl ChartData {
    /// Extract chartable series with the default config. See
    /// [`ChartData::extract_with_config`].
    pub fn extract(columns: &[Column], rows: &[Vec<CellValue>]) -> Result<ChartData, ChartError> {
        Self::extract_with_config(columns, rows, &ChartConfig::default())
    }

    /// Extract chartable series from a result set. The label column is the
    /// configured one or the first string-ish column whose values are not all
    /// numeric-looking (falling back to 1-based row indices); the value
    /// columns are the configured numeric ones or every numeric column in
    /// SELECT order, capped at [`MAX_SERIES`]. Categories cap at the
    /// configured top-N; all caps set explicit truncation flags.
    pub fn extract_with_config(
        columns: &[Column],
        rows: &[Vec<CellValue>],
        config: &ChartConfig,
    ) -> Result<ChartData, ChartError> {
        if rows.is_empty() {
            return Err(ChartError::NoRows);
        }
        let source_truncated = rows.len() > MAX_SOURCE_ROWS;
        let rows = &rows[..rows.len().min(MAX_SOURCE_ROWS)];
        let column_count = columns
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        let roles: Vec<ColumnRole> = (0..column_count)
            .map(|i| classify_column(i, rows))
            .collect();
        let available_value_columns: Vec<usize> = roles
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == ColumnRole::Numeric)
            .map(|(i, _)| i)
            .collect();
        let value_columns = if config.value_columns.is_empty() {
            available_value_columns.clone()
        } else {
            config
                .value_columns
                .iter()
                .copied()
                .filter(|index| available_value_columns.contains(index))
                .collect()
        };
        if value_columns.is_empty() {
            return Err(ChartError::NoNumericColumns);
        }
        let series_truncated = value_columns.len() > MAX_SERIES;
        let value_columns = value_columns
            .into_iter()
            .take(MAX_SERIES)
            .collect::<Vec<_>>();
        let label_index = config
            .label_column
            .filter(|index| *index < column_count)
            .or_else(|| roles.iter().position(|role| *role == ColumnRole::Label));
        let label_for_row = |row_index: usize, row: &Vec<CellValue>| match label_index {
            Some(column) => label_text(row.get(column)),
            None => (row_index + 1).to_string(),
        };
        let (mut categories, mut values, mut category_rows) =
            if config.aggregate == ChartAggregate::None {
                let categories = rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| label_for_row(index, row))
                    .collect::<Vec<_>>();
                let values = value_columns
                    .iter()
                    .map(|column| {
                        rows.iter()
                            .map(|row| row.get(*column).and_then(numeric_cell))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let category_rows = (0..rows.len()).map(|index| vec![index]).collect();
                (categories, values, category_rows)
            } else {
                aggregate_rows(rows, &label_for_row, &value_columns, config.aggregate)
            };

        let mut order = (0..categories.len()).collect::<Vec<_>>();
        match config.order {
            ChartOrder::Source => {}
            ChartOrder::CategoryAscending => {
                order.sort_by(|left, right| categories[*left].cmp(&categories[*right]))
            }
            ChartOrder::ValueDescending => order.sort_by(|left, right| {
                let left = values.first().and_then(|series| series[*left]);
                let right = values.first().and_then(|series| series[*right]);
                right
                    .partial_cmp(&left)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        let top_n = config.top_n.clamp(1, MAX_CATEGORIES);
        let categories_truncated = order.len() > top_n;
        order.truncate(top_n);
        categories = order
            .iter()
            .map(|index| categories[*index].clone())
            .collect();
        category_rows = order
            .iter()
            .map(|index| category_rows[*index].clone())
            .collect();
        values = values
            .into_iter()
            .map(|series| order.iter().map(|index| series[*index]).collect())
            .collect();

        let series: Vec<ChartSeries> = value_columns
            .iter()
            .zip(values)
            .map(|(&col, values)| ChartSeries {
                name: columns
                    .get(col)
                    .map(|c| c.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("column {}", col + 1)),
                values,
            })
            .collect();

        Ok(ChartData {
            label_column: label_index.and_then(|i| columns.get(i).map(|c| c.name.clone())),
            categories,
            series,
            category_rows,
            categories_truncated,
            series_truncated,
            source_truncated,
        })
    }

    /// The value axis for this data under `kind`. Bars, areas, and histograms
    /// force a zero floor (and ceiling, for all-negative data) so bar length
    /// stays proportional to value; lines scale to the data range. (The paint
    /// path goes through [`super::paint::axis_for`], which sees only the
    /// legend-visible series.)
    #[must_use]
    pub fn axis(&self, kind: ChartKind) -> AxisScale {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for series in &self.series {
            for v in series.values.iter().flatten() {
                min = min.min(*v);
                max = max.max(*v);
            }
        }
        if !min.is_finite() || !max.is_finite() {
            // Every included point is a gap — chart an honest empty 0..1.
            return AxisScale::compute(0.0, 1.0, false);
        }
        AxisScale::compute(
            min,
            max,
            matches!(
                kind,
                ChartKind::Bar | ChartKind::Area | ChartKind::Histogram
            ),
        )
    }

    /// Why this data cannot be drawn as a pie/donut, or `None` if it can.
    #[must_use]
    pub fn radial_error(&self) -> Option<&'static str> {
        if self.series.len() != 1 {
            return Some("Pie and donut charts require exactly one numeric series");
        }
        if self.categories.len() > 16 {
            return Some("Pie and donut charts support at most 16 categories");
        }
        let values = &self.series[0].values;
        if values.iter().flatten().any(|value| *value < 0.0) {
            return Some("Pie and donut charts cannot represent negative values");
        }
        if !values.iter().flatten().any(|value| *value > 0.0) {
            return Some("Pie and donut charts need at least one positive value");
        }
        None
    }
}

/// The (column name, finite values, source-row-index) triple a histogram is
/// built from: the first configured value column that is numeric, else the
/// first numeric column. Row indices travel with each value so a clicked bin
/// can navigate to the rows inside it.
#[must_use]
pub fn histogram_source(
    columns: &[Column],
    rows: &[Vec<CellValue>],
    config: &ChartConfig,
) -> Option<(String, Vec<(f64, usize)>)> {
    let rows = &rows[..rows.len().min(MAX_SOURCE_ROWS)];
    let column_count = columns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let pick = config
        .value_columns
        .iter()
        .copied()
        .find(|index| *index < column_count && classify_column(*index, rows) == ColumnRole::Numeric)
        .or_else(|| {
            (0..column_count).find(|&index| classify_column(index, rows) == ColumnRole::Numeric)
        })?;
    let mut out = Vec::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        if let Some(value) = row.get(pick).and_then(numeric_cell) {
            out.push((value, row_index));
        }
    }
    if out.is_empty() {
        return None;
    }
    let name = columns
        .get(pick)
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("column {}", pick + 1));
    Some((name, out))
}

// ---------------------------------------------------------------------------
// Axis / tick math
// ---------------------------------------------------------------------------

/// A nice-number value axis: `min`/`max` are multiples of `step`, and `step`
/// follows the 1/2/5 progression.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisScale {
    /// Axis minimum.
    pub min: f64,
    /// Axis maximum.
    pub max: f64,
    /// Tick spacing, a 1/2/5 multiple of a power of ten.
    pub step: f64,
}

/// Round `x` to a "nice" number — 1, 2, or 5 times a power of ten. With
/// `round` the breakpoints sit between the candidates (for picking a step);
/// without, the result is the smallest nice number `>= x`.
#[must_use]
pub fn nice_num(x: f64, round: bool) -> f64 {
    if !x.is_finite() || x <= 0.0 {
        return 1.0;
    }
    let exp = x.log10().floor();
    let base = 10f64.powf(exp);
    let f = x / base;
    let nf = if round {
        if f < 1.5 {
            1.0
        } else if f < 3.0 {
            2.0
        } else if f < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nf * base
}

impl AxisScale {
    /// Build the axis over `[data_min, data_max]`. `force_zero_floor` (bars)
    /// extends the range to include zero. A degenerate range (single value,
    /// all equal, or nothing finite) still yields a usable non-zero span.
    #[must_use]
    pub fn compute(data_min: f64, data_max: f64, force_zero_floor: bool) -> AxisScale {
        let (mut min, mut max) = if data_min.is_finite() && data_max.is_finite() {
            (data_min.min(data_max), data_min.max(data_max))
        } else {
            (0.0, 1.0)
        };
        if force_zero_floor {
            min = min.min(0.0);
            max = max.max(0.0);
        }
        if min == max {
            // A single distinct value: anchor the other end at zero so the
            // mark's size still means something; 0 itself spans 0..1.
            if min == 0.0 {
                max = 1.0;
            } else if min > 0.0 {
                min = 0.0;
            } else {
                max = 0.0;
            }
        }
        let step = nice_num((max - min) / 5.0, true);
        let min = (min / step).floor() * step;
        let max = (max / step).ceil() * step;
        AxisScale { min, max, step }
    }

    /// Tick positions from `min` to `max` inclusive. Bounded, so a corrupt
    /// scale can never spin the paint pass.
    #[must_use]
    pub fn ticks(&self) -> Vec<f64> {
        if !self.step.is_finite()
            || self.step <= 0.0
            || !self.min.is_finite()
            || !self.max.is_finite()
        {
            return vec![self.min, self.max];
        }
        let mut out = Vec::new();
        let eps = self.step * 1e-6;
        for i in 0..64 {
            let t = self.min + i as f64 * self.step;
            if t > self.max + eps {
                break;
            }
            out.push(t);
        }
        out
    }

    /// Where `v` sits on the axis: 0.0 at `min`, 1.0 at `max`, clamped.
    #[must_use]
    pub fn fraction(&self, v: f64) -> f64 {
        let span = self.max - self.min;
        if !span.is_finite() || span <= 0.0 {
            return 0.0;
        }
        ((v - self.min) / span).clamp(0.0, 1.0)
    }
}

/// Format an axis tick label, abbreviating thousands/millions/billions.
#[must_use]
pub fn format_tick(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1e9 {
        format!("{}B", trim_float(v / 1e9))
    } else if abs >= 1e6 {
        format!("{}M", trim_float(v / 1e6))
    } else if abs >= 1e3 {
        format!("{}K", trim_float(v / 1e3))
    } else {
        trim_float(v)
    }
}

/// `v` to at most three decimals, trailing zeros trimmed, `-0` normalized.
fn trim_float(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Histogram binning
// ---------------------------------------------------------------------------

/// Contiguous numeric bins: `edges` has one more entry than `counts`; bin `i`
/// covers `[edges[i], edges[i + 1])` (the last bin is closed on the right so
/// the maximum value lands inside it).
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramBins {
    /// Bin edges; `edges.len() == counts.len() + 1`.
    pub edges: Vec<f64>,
    /// Population per bin.
    pub counts: Vec<usize>,
}

impl HistogramBins {
    /// The bin-range label for bin `index`, in the same abbreviated notation
    /// as axis ticks ("0–10", "1.5K–2K").
    #[must_use]
    pub fn label(&self, index: usize) -> String {
        format!(
            "{}\u{2013}{}",
            format_tick(self.edges[index]),
            format_tick(self.edges[index + 1])
        )
    }

    /// The bin index containing `value`, clamped into range (the last bin is
    /// closed on the right). Used by click-to-navigate to resolve a bin.
    #[must_use]
    pub fn bin_for(&self, value: f64) -> usize {
        let bins = self.counts.len();
        if bins == 0 {
            return 0;
        }
        let first = self.edges[0];
        let width = self.edges[1] - first;
        if width <= 0.0 || !width.is_finite() {
            return 0;
        }
        ((((value - first) / width).floor() as isize).clamp(0, bins as isize - 1)) as usize
    }
}

/// Bin `values` into contiguous nice-edged histogram bins.
///
/// The bin count targets Sturges' rule (`ceil(log2 n) + 1`) clamped to
/// `5..=30`; the bin width is then rounded to a 1/2/5-nice number and the
/// first edge aligned to a multiple of it, so every edge is a readable
/// number (the realized bin count can therefore drift slightly from the
/// target; it is re-widened along the 1/2/5 ladder if it would exceed 30).
/// Non-finite inputs are ignored; no finite values at all → `None` (no
/// chart). A single distinct value yields one centered unit-wide bin.
#[must_use]
pub fn histogram_bins(values: &[f64]) -> Option<HistogramBins> {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return None;
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &value in &finite {
        min = min.min(value);
        max = max.max(value);
    }
    if min == max {
        return Some(HistogramBins {
            edges: vec![min - 0.5, min + 0.5],
            counts: vec![finite.len()],
        });
    }
    let target = ((finite.len() as f64).log2().ceil() as usize + 1).clamp(5, 30);
    let mut width = nice_num((max - min) / target as f64, true);
    let mut first = (min / width).floor() * width;
    // Edge alignment can push the realized count past the clamp; widen along
    // the 1/2/5 ladder until it fits (bounded — each step at least doubles
    // coverage).
    for _ in 0..8 {
        if ((max - first) / width).ceil() as usize <= 30 {
            break;
        }
        width = nice_num(width * 1.5, false);
        first = (min / width).floor() * width;
    }
    let mut edges = vec![first];
    let eps = width * 1e-9;
    while edges.last().copied().unwrap_or(min) < max - eps && edges.len() < 64 {
        edges.push(first + edges.len() as f64 * width);
    }
    if edges.len() < 2 {
        edges.push(first + width);
    }
    let bins = edges.len() - 1;
    let mut counts = vec![0usize; bins];
    for &value in &finite {
        let index = (((value - first) / width).floor() as isize).clamp(0, bins as isize - 1);
        counts[index as usize] += 1;
    }
    Some(HistogramBins { edges, counts })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::field_reassign_with_default)]

    use super::*;
    use crate::data::ColumnKind;

    fn col(name: &str, kind: ColumnKind) -> Column {
        Column {
            name: name.to_string(),
            kind,
            width: 90.0,
        }
    }

    fn s(v: &str) -> CellValue {
        CellValue::Text(v.to_string())
    }

    fn extract_ok(columns: &[Column], rows: &[Vec<CellValue>], config: &ChartConfig) -> ChartData {
        ChartData::extract_with_config(columns, rows, config).expect("extraction succeeds")
    }

    #[test]
    fn extracts_first_label_and_all_numeric_series() {
        let columns = vec![col("name", ColumnKind::Text), col("v", ColumnKind::Integer)];
        let rows = vec![
            vec![s("a"), CellValue::Integer(1)],
            vec![s("b"), CellValue::Integer(2)],
        ];
        let data = extract_ok(&columns, &rows, &ChartConfig::default());
        assert_eq!(data.label_column.as_deref(), Some("name"));
        assert_eq!(data.categories, vec!["a", "b"]);
        assert_eq!(data.series.len(), 1);
        assert_eq!(data.series[0].name, "v");
        assert_eq!(data.series[0].values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(data.category_rows, vec![vec![0], vec![1]]);
        assert!(!data.categories_truncated && !data.series_truncated && !data.source_truncated);
    }

    #[test]
    fn numeric_looking_strings_chart_as_series() {
        let columns = vec![col("name", ColumnKind::Text), col("amt", ColumnKind::Text)];
        let rows = vec![vec![s("a"), s("1.5")], vec![s("b"), s("2")]];
        let data = extract_ok(&columns, &rows, &ChartConfig::default());
        assert_eq!(data.label_column.as_deref(), Some("name"));
        assert_eq!(data.series[0].values, vec![Some(1.5), Some(2.0)]);
    }

    #[test]
    fn label_falls_back_to_row_index_and_category_rows_follow() {
        let columns = vec![col("v", ColumnKind::Integer)];
        let rows = vec![vec![CellValue::Integer(5)], vec![CellValue::Integer(7)]];
        let data = extract_ok(&columns, &rows, &ChartConfig::default());
        assert_eq!(data.categories, vec!["1", "2"]);
        assert_eq!(data.series[0].values, vec![Some(5.0), Some(7.0)]);
        assert_eq!(data.category_rows, vec![vec![0], vec![1]]);
    }

    #[test]
    fn configured_label_and_values_drive_grouping() {
        let columns = vec![
            col("region", ColumnKind::Text),
            col("amount", ColumnKind::Decimal),
            col("ignored", ColumnKind::Integer),
        ];
        let rows = vec![
            vec![s("east"), CellValue::Decimal(10.0), CellValue::Integer(1)],
            vec![s("west"), CellValue::Decimal(20.0), CellValue::Integer(2)],
            vec![s("east"), CellValue::Decimal(30.0), CellValue::Integer(3)],
        ];
        let config = ChartConfig {
            label_column: Some(0),
            value_columns: vec![1],
            aggregate: ChartAggregate::Sum,
            ..ChartConfig::default()
        };
        let data = extract_ok(&columns, &rows, &config);
        assert_eq!(data.categories, vec!["east", "west"]);
        assert_eq!(data.series[0].values, vec![Some(40.0), Some(20.0)]);
        assert_eq!(data.category_rows, vec![vec![0, 2], vec![1]]);
    }

    #[test]
    fn aggregates_cover_average_minimum_maximum_and_count() {
        let columns = vec![col("g", ColumnKind::Text), col("v", ColumnKind::Decimal)];
        let base_rows = vec![
            vec![s("x"), CellValue::Decimal(4.0)],
            vec![s("x"), CellValue::Decimal(10.0)],
            vec![s("x"), CellValue::None],
            vec![s("y"), CellValue::Decimal(7.0)],
        ];
        let expect = |aggregate: ChartAggregate, values: Vec<Option<f64>>| {
            let config = ChartConfig {
                label_column: Some(0),
                value_columns: vec![1],
                aggregate,
                ..ChartConfig::default()
            };
            let data = extract_ok(&columns, &base_rows, &config);
            assert_eq!(data.series[0].values, values, "{aggregate:?}");
        };
        expect(ChartAggregate::Average, vec![Some(7.0), Some(7.0)]);
        expect(ChartAggregate::Minimum, vec![Some(4.0), Some(7.0)]);
        expect(ChartAggregate::Maximum, vec![Some(10.0), Some(7.0)]);
        expect(ChartAggregate::Count, vec![Some(2.0), Some(1.0)]);
        expect(ChartAggregate::Sum, vec![Some(14.0), Some(7.0)]);
    }

    #[test]
    fn ordering_and_top_n_permute_category_rows_too() {
        let columns = vec![col("name", ColumnKind::Text), col("v", ColumnKind::Integer)];
        let rows: Vec<Vec<CellValue>> = ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .map(|(i, name)| vec![s(name), CellValue::Integer((i * 10) as i64)])
            .collect();
        let config = ChartConfig {
            order: ChartOrder::ValueDescending,
            top_n: 2,
            ..ChartConfig::default()
        };
        let data = extract_ok(&columns, &rows, &config);
        assert_eq!(data.categories, vec!["d", "c"]);
        assert_eq!(data.series[0].values, vec![Some(30.0), Some(20.0)]);
        assert_eq!(data.category_rows, vec![vec![3], vec![2]]);
        assert!(data.categories_truncated);
    }

    #[test]
    fn nulls_and_blanks_become_gaps_with_null_labels() {
        let columns = vec![col("name", ColumnKind::Text), col("v", ColumnKind::Integer)];
        let rows = vec![
            vec![s("a"), CellValue::None],
            vec![CellValue::None, CellValue::Integer(3)],
        ];
        let data = extract_ok(&columns, &rows, &ChartConfig::default());
        assert_eq!(data.categories, vec!["a", "NULL"]);
        assert_eq!(data.series[0].values, vec![None, Some(3.0)]);
    }

    #[test]
    fn series_cap_sets_flag() {
        let columns: Vec<Column> = (0..6)
            .map(|i| col(&format!("c{i}"), ColumnKind::Integer))
            .collect();
        let rows = vec![(0..6)
            .map(|i| CellValue::Integer(i as i64))
            .collect::<Vec<_>>()];
        let data = extract_ok(&columns, &rows, &ChartConfig::default());
        assert_eq!(data.series.len(), MAX_SERIES);
        assert!(data.series_truncated);
    }

    #[test]
    fn categories_cap_sets_flag() {
        let columns = vec![col("name", ColumnKind::Text), col("v", ColumnKind::Integer)];
        let rows: Vec<Vec<CellValue>> = (0..(MAX_CATEGORIES + 5))
            .map(|i| vec![s(&i.to_string()), CellValue::Integer(i as i64)])
            .collect();
        let mut config = ChartConfig::default();
        config.top_n = MAX_CATEGORIES;
        let data = extract_ok(&columns, &rows, &config);
        assert_eq!(data.categories.len(), MAX_CATEGORIES);
        assert!(data.categories_truncated);
    }

    #[test]
    fn rejects_rows_without_numeric_columns() {
        let columns = vec![col("name", ColumnKind::Text)];
        let rows = vec![vec![s("a")], vec![s("b")]];
        assert_eq!(
            ChartData::extract_with_config(&columns, &rows, &ChartConfig::default()),
            Err(ChartError::NoNumericColumns)
        );
    }

    #[test]
    fn rejects_empty_rows() {
        let columns = vec![col("v", ColumnKind::Integer)];
        assert_eq!(
            ChartData::extract_with_config(&columns, &[], &ChartConfig::default()),
            Err(ChartError::NoRows)
        );
    }

    // -- Axis / tick math -------------------------------------------------

    #[test]
    fn bar_axis_zero_floors_to_nice_range() {
        let axis = AxisScale::compute(12.0, 48.0, true);
        assert_eq!((axis.min, axis.max, axis.step), (0.0, 50.0, 10.0));
    }

    #[test]
    fn negative_data_keeps_floor_and_includes_zero_tick() {
        let axis = AxisScale::compute(-18.0, 12.0, true);
        assert!(axis.min <= -18.0);
        assert!(axis.min < 0.0);
        assert!(axis.max >= 12.0);
        assert!(axis.max > 0.0);
        assert!(axis.ticks().contains(&0.0));
    }

    #[test]
    fn line_axis_does_not_force_zero() {
        let axis = AxisScale::compute(12.0, 48.0, false);
        assert!(
            axis.min > 0.0,
            "line axis scales to the data range, never forcing zero"
        );
        assert!(axis.max >= 48.0, "axis always covers the data");
    }

    #[test]
    fn single_value_spans_to_zero() {
        let axis = AxisScale::compute(7.0, 7.0, true);
        assert_eq!((axis.min, axis.max), (0.0, 7.0));
        let axis = AxisScale::compute(0.0, 0.0, true);
        assert_eq!((axis.min, axis.max), (0.0, 1.0));
        let axis = AxisScale::compute(-7.0, -7.0, true);
        assert_eq!((axis.min, axis.max), (-7.0, 0.0));
    }

    #[test]
    fn nice_number_ladder_hits_1_2_5() {
        for (input, expected) in [
            (0.8, 1.0),
            (1.4, 1.0),
            (1.6, 2.0),
            (2.8, 2.0),
            (3.0, 5.0),
            (6.0, 5.0),
            (7.0, 10.0),
        ] {
            assert_eq!(nice_num(input, true), expected, "round({input})");
        }
        for (input, expected) in [(0.7, 1.0), (1.0, 1.0), (1.4, 2.0), (4.0, 5.0), (6.0, 10.0)] {
            assert_eq!(nice_num(input, false), expected, "ceil({input})");
        }
    }

    #[test]
    fn format_tick_abbreviates_large_values() {
        assert_eq!(format_tick(0.0), "0");
        assert_eq!(format_tick(1_500.0), "1.5K");
        assert_eq!(format_tick(2_000_000.0), "2M");
        assert_eq!(format_tick(-3_000_000_000.0), "-3B");
        assert_eq!(format_tick(0.125), "0.125");
        assert_eq!(format_tick(-0.0001), "0", "-0 normalizes");
    }

    #[test]
    fn fraction_clamps() {
        let axis = AxisScale::compute(0.0, 10.0, false);
        assert_eq!(axis.fraction(-5.0), 0.0);
        assert_eq!(axis.fraction(5.0), 0.5);
        assert_eq!(axis.fraction(50.0), 1.0);
    }

    // -- Histogram binning -------------------------------------------------

    #[test]
    fn histogram_uniform_decades_bin_nicely() {
        let values: Vec<f64> = (0..100).map(|v| v as f64).collect();
        let bins = histogram_bins(&values).expect("bins");
        assert_eq!(bins.edges.first().copied(), Some(0.0));
        assert_eq!(bins.edges.last().copied(), Some(100.0));
        assert_eq!(bins.counts, vec![10; 10]);
        assert_eq!(bins.label(0), "0\u{2013}10");
    }

    #[test]
    fn histogram_single_value_yields_centered_bin() {
        let bins = histogram_bins(&[5.0, 5.0, 5.0]).expect("bins");
        assert_eq!(bins.edges, vec![4.5, 5.5]);
        assert_eq!(bins.counts, vec![3]);
    }

    #[test]
    fn histogram_empty_or_nonfinite_is_none() {
        assert!(histogram_bins(&[]).is_none());
        assert!(histogram_bins(&[f64::NAN, f64::INFINITY]).is_none());
    }

    #[test]
    fn histogram_bin_for_resolves_values_into_bins() {
        let values: Vec<f64> = (0..100).map(|v| v as f64).collect();
        let bins = histogram_bins(&values).expect("bins");
        assert_eq!(bins.bin_for(5.0), 0);
        assert_eq!(bins.bin_for(95.0), 9);
        assert_eq!(bins.bin_for(100.0), 9, "last bin is closed on the right");
    }

    // -- Histogram source ---------------------------------------------------

    #[test]
    fn histogram_source_prefers_configured_numeric_column() {
        let columns = vec![
            col("name", ColumnKind::Text),
            col("score", ColumnKind::Integer),
            col("delta", ColumnKind::Decimal),
        ];
        let rows = vec![
            vec![s("a"), CellValue::Integer(1), CellValue::Decimal(9.0)],
            vec![s("b"), CellValue::Integer(2), CellValue::Decimal(8.0)],
        ];
        let config = ChartConfig {
            value_columns: vec![2],
            ..ChartConfig::default()
        };
        let (name, pairs) = histogram_source(&columns, &rows, &config).expect("source");
        assert_eq!(name, "delta");
        assert_eq!(pairs, vec![(9.0, 0), (8.0, 1)]);

        // Without configuration, the first numeric column wins.
        let (name, pairs) =
            histogram_source(&columns, &rows, &ChartConfig::default()).expect("source");
        assert_eq!(name, "score");
        assert_eq!(pairs, vec![(1.0, 0), (2.0, 1)]);
    }
}
