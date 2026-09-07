//! Canvas painter for the chart tab, plus the geometry mirror used by
//! click-to-navigate hit testing.
//!
//! The painter is a port of the chart renderer the host app shipped
//! in-tree: gridlines + right-aligned tick labels, grouped bars, gapped
//! polylines/areas, scatter dots, thinned category labels, and radial
//! pie/donut slices. Colors are derived from the host-supplied
//! [`GridTheme`] — no chart-specific theme fields exist; see
//! [`series_color`].

use gpui::{point, px, size, App, Bounds, Hsla, PaintQuad, Point, TextAlign, Window};

use crate::grid::paint::{default_char_width, grid_font};
use crate::grid::theme::GridTheme;

use super::config::ChartKind;
use super::model::{format_tick, AxisScale, HistogramBins};

/// Hue step (golden ratio conjugate) between consecutive series colors —
/// maximally distant hues for any prefix of the series.
const SERIES_HUE_STEP: f32 = 0.618_034;

/// A single paintable series: name, resolved color, per-category values.
#[derive(Clone, Debug)]
pub(crate) struct PaintSeries {
    /// Display name (source column name).
    pub(crate) name: String,
    /// Series color.
    pub(crate) color: Hsla,
    /// One value per category; `None` is a gap.
    pub(crate) values: Vec<Option<f64>>,
}

/// Everything the paint closure needs, resolved on the render pass (theme
/// reads happen here; the closure is `'static`). Also the input to
/// [`super::svg::chart_svg`] — legend-hidden entries are already excluded by
/// the time a `ChartPaint` exists — and to [`ChartPaint::hit_test`].
#[derive(Clone)]
pub struct ChartPaint {
    /// Chart kind being painted.
    pub(crate) kind: ChartKind,
    /// Category labels in display order.
    pub(crate) categories: Vec<String>,
    /// Visible series in display order.
    pub(crate) series: Vec<PaintSeries>,
    /// Per-category slice colors for radial kinds (empty otherwise), keyed
    /// to the ORIGINAL category index so hiding a slice never recolors the
    /// rest.
    pub(crate) slice_colors: Vec<Hsla>,
    /// Source rows behind each displayed category — the click-to-navigate
    /// payload. For histograms these are the rows inside each bin.
    pub(crate) category_rows: Vec<Vec<usize>>,
    /// Value axis (zero-floored for bar-like kinds).
    pub(crate) axis: AxisScale,
    /// Theme snapshot the derived colors came from (used by the radial
    /// slice-color fallback).
    pub(crate) theme: GridTheme,
    /// Plot background.
    pub(crate) background: Hsla,
    /// Tick/label text color.
    pub(crate) text_dim: Hsla,
    /// Gridline color.
    pub(crate) grid_line: Hsla,
    /// Base font size for tick and category labels.
    pub(crate) font_size: f32,
}

/// The value axis over the *visible* series under `kind` — bars, areas, and
/// histograms force a zero floor; the rest scale to the data range.
pub(crate) fn axis_for(series: &[PaintSeries], kind: ChartKind) -> AxisScale {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for series in series {
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

/// The color for series `index`, derived from the host theme's accent
/// (`sort_indicator`) by golden-ratio hue rotation. A grayscale accent
/// (where hue rotation would be invisible) falls back to a lightness ladder
/// so the series stay distinguishable anyway.
pub(crate) fn series_color(theme: &GridTheme, index: usize) -> Hsla {
    let accent = theme.sort_indicator;
    if accent.s >= 0.08 {
        let hue = (accent.h + SERIES_HUE_STEP * index as f32).fract();
        let lightness = (accent.l + (0.6 - accent.l) * 0.35).clamp(0.3, 0.72);
        Hsla {
            h: hue,
            s: accent.s.clamp(0.45, 0.85),
            l: lightness,
            a: accent.a,
        }
    } else {
        const LIGHTNESS: [f32; 4] = [0.72, 0.52, 0.34, 0.62];
        Hsla {
            h: accent.h,
            s: accent.s,
            l: LIGHTNESS[index % LIGHTNESS.len()],
            a: accent.a,
        }
    }
}

/// The slice color for radial category `index` — the series-color ladder at
/// decreasing opacity so a long tail of slices fades gracefully.
pub(crate) fn category_color(theme: &GridTheme, index: usize) -> Hsla {
    series_color(theme, index % 4).opacity((1.0 - (index / 4) as f32 * 0.12).max(0.52))
}

impl ChartPaint {
    pub(crate) fn new(
        kind: ChartKind,
        categories: Vec<String>,
        series: Vec<PaintSeries>,
        slice_colors: Vec<Hsla>,
        category_rows: Vec<Vec<usize>>,
        theme: &GridTheme,
        font_size: f32,
    ) -> Self {
        let axis = axis_for(&series, kind);
        Self {
            kind,
            categories,
            series,
            slice_colors,
            category_rows,
            axis,
            theme: theme.clone(),
            background: theme.bg,
            text_dim: theme.muted_text,
            grid_line: theme.grid_line,
            font_size,
        }
    }

    /// Bins become the categories (labeled with their value ranges), the
    /// counts become a single bar series on a zero-floored axis, and
    /// `bin_rows` (source rows per bin, in bin order) rides along as the
    /// click-to-navigate payload.
    pub(crate) fn from_histogram(
        name: &str,
        bins: &HistogramBins,
        bin_rows: Vec<Vec<usize>>,
        theme: &GridTheme,
        font_size: f32,
    ) -> Self {
        let categories = (0..bins.counts.len()).map(|i| bins.label(i)).collect();
        let series = vec![PaintSeries {
            name: name.to_string(),
            color: series_color(theme, 0),
            values: bins
                .counts
                .iter()
                .map(|count| Some(*count as f64))
                .collect(),
        }];
        Self::new(
            ChartKind::Histogram,
            categories,
            series,
            Vec::new(),
            bin_rows,
            theme,
            font_size,
        )
    }

    /// The source rows a click at grid-relative `(x, y)` inside a
    /// `width` × `height` plot lands on, as a `(category, series)` hit.
    ///
    /// For cartesian kinds any click inside the plot area resolves to the
    /// category slot it falls in (the plot's margins mirror the painter's,
    /// with the left gutter estimated from character widths — a few pixels
    /// of drift at the very left edge is fine for hit testing). For radial
    /// kinds the click must land inside a slice ring segment, replicating
    /// the painter's sweep accumulation exactly.
    pub(crate) fn hit_test(
        &self,
        width: f32,
        height: f32,
        x: f32,
        y: f32,
    ) -> Option<(usize, Option<usize>)> {
        let n = self.categories.len();
        if n == 0 || self.series.is_empty() {
            return None;
        }
        if matches!(self.kind, ChartKind::Pie | ChartKind::Donut) {
            return self.hit_test_radial(width, height, x, y);
        }
        // Estimated left gutter: widest tick label approximated from the
        // monospace advance width, same margins as the painter otherwise.
        let tick_fs = self.font_size * 0.85;
        let widest_tick_chars = self
            .axis
            .ticks()
            .iter()
            .map(|t| format_tick(*t).chars().count())
            .max()
            .unwrap_or(1) as f32;
        let left = default_char_width(tick_fs) * widest_tick_chars + 14.0;
        let right = 10.0;
        let top = 8.0;
        let bottom = self.font_size * 1.4 + 8.0;
        let plot_w = width - left - right;
        let plot_h = height - top - bottom;
        if plot_w < 20.0 || plot_h < 20.0 {
            return None;
        }
        if x < left || x > left + plot_w || y < top || y > top + plot_h {
            return None;
        }
        let cat_w = plot_w / n as f32;
        let category = (((x - left) / cat_w).floor() as usize).min(n - 1);
        if self
            .category_rows
            .get(category)
            .is_none_or(|rows| rows.is_empty())
        {
            return None;
        }
        // For grouped bars, report the series whose bar occupies the click
        // column when that series has a value there; anything else reports
        // the whole category (line/scatter/area marks overlap slots).
        if self.kind == ChartKind::Bar && self.series.len() > 1 {
            let band = (cat_w * 0.72).max(1.0);
            let bar_w = (band / self.series.len() as f32).max(1.0);
            let group_x = category as f32 * cat_w + (cat_w - band) / 2.0;
            let into = x - left - group_x;
            if into >= 0.0 && into <= band {
                let series = (into / bar_w).floor() as usize;
                if series < self.series.len()
                    && series_index_has_value(&self.series[series], category)
                {
                    return Some((category, Some(series)));
                }
            }
        }
        Some((category, None))
    }

    /// Radial hit test: ring containment (donut hole excluded) plus sweep
    /// accumulation identical to `paint_radial`.
    fn hit_test_radial(
        &self,
        width: f32,
        height: f32,
        x: f32,
        y: f32,
    ) -> Option<(usize, Option<usize>)> {
        let values = &self.series.first()?.values;
        let total: f64 = values
            .iter()
            .flatten()
            .copied()
            .filter(|value| *value > 0.0)
            .sum();
        if total <= 0.0 {
            return None;
        }
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let outer = (width.min(height) * 0.4).max(1.0);
        let inner = if self.kind == ChartKind::Donut {
            outer * 0.56
        } else {
            0.0
        };
        let dx = x - center_x;
        let dy = y - center_y;
        let r = dx.hypot(dy);
        if r > outer || r < inner {
            return None;
        }
        let rel = (((dy.atan2(dx) + std::f32::consts::FRAC_PI_2) % std::f32::consts::TAU)
            + std::f32::consts::TAU)
            % std::f32::consts::TAU;
        let mut angle = 0f32;
        for (index, value) in values.iter().enumerate() {
            let value = value.unwrap_or(0.0).max(0.0);
            if value == 0.0 {
                continue;
            }
            let sweep = (value / total) as f32 * std::f32::consts::TAU;
            angle += sweep;
            if rel <= angle {
                return Some((index, None));
            }
        }
        None
    }

    /// Paint the whole plot into `bounds`: gridlines + tick labels, marks
    /// (grouped bars or per-series polylines with gaps at NULLs), and
    /// thinned category labels. Radial kinds go through
    /// `paint_radial_chart` and paint no axes.
    pub(crate) fn paint(&self, bounds: Bounds<gpui::Pixels>, window: &mut Window, cx: &mut App) {
        let n = self.categories.len();
        let n_series = self.series.len();
        if n == 0 || n_series == 0 {
            return;
        }
        let text_system = window.text_system().clone();
        let font = grid_font();
        let tick_fs = self.font_size * 0.85;
        let line_height = |fs: f32| px(fs * 1.4);
        let shape = |text: &str, color: Hsla, fs: f32| {
            let run = gpui::TextRun {
                len: text.len(),
                color,
                font: font.clone(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            text_system.shape_line(text.to_owned().into(), px(fs), &[run], None)
        };

        // Layout: the left gutter fits the widest tick label; the bottom
        // band holds one row of category labels.
        let ticks = self.axis.ticks();
        let tick_lines: Vec<(f64, gpui::ShapedLine)> = ticks
            .iter()
            .map(|&t| (t, shape(&format_tick(t), self.text_dim, tick_fs)))
            .collect();
        let widest_tick = tick_lines
            .iter()
            .map(|(_, l)| f32::from(l.width))
            .fold(0.0f32, f32::max);
        let ox = f32::from(bounds.origin.x);
        let oy = f32::from(bounds.origin.y);
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        if matches!(self.kind, ChartKind::Pie | ChartKind::Donut) {
            self.paint_radial(ox, oy, w, h, window);
            return;
        }
        let left = widest_tick + 14.0;
        let right = 10.0;
        let top = 8.0;
        let bottom = self.font_size * 1.4 + 8.0;
        let plot_w = w - left - right;
        let plot_h = h - top - bottom;
        if plot_w < 20.0 || plot_h < 20.0 {
            return;
        }
        let x0 = ox + left;
        let y0 = oy + top;
        let y_of = |v: f64| y0 + plot_h * (1.0 - self.axis.fraction(v) as f32);

        // Gridlines + right-aligned tick labels.
        for (t, line) in &tick_lines {
            let y = y_of(*t);
            fill_quad(window, x0, y, plot_w, 1.0, self.grid_line);
            let lw = f32::from(line.width);
            let _ = line.paint(
                Point {
                    x: px(x0 - lw - 6.0),
                    y: px(y - tick_fs * 0.7),
                },
                line_height(tick_fs),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        let cat_w = plot_w / n as f32;
        match self.kind {
            ChartKind::Bar | ChartKind::Histogram => {
                // Grouped bars around a zero baseline (the axis includes zero by
                // construction). A present zero value paints a 1px sliver — an
                // honest "zero", visually distinct from a NULL gap's nothing.
                // Histogram bins are contiguous: the band is the full category
                // slot (bars touch, minus the 1px hairline) instead of the
                // grouped 72% band.
                let baseline = y_of(0.0);
                let band = if self.kind == ChartKind::Histogram {
                    cat_w.max(1.0)
                } else {
                    (cat_w * 0.72).max(1.0)
                };
                let bar_w = (band / n_series as f32).max(1.0);
                for ci in 0..n {
                    let group_x = x0 + ci as f32 * cat_w + (cat_w - band) / 2.0;
                    for (si, series) in self.series.iter().enumerate() {
                        let Some(v) = series.values.get(ci).copied().flatten() else {
                            continue;
                        };
                        let yv = y_of(v);
                        let (bar_y, bar_h) = if yv <= baseline {
                            (yv, baseline - yv)
                        } else {
                            (baseline, yv - baseline)
                        };
                        fill_quad(
                            window,
                            group_x + si as f32 * bar_w,
                            bar_y,
                            (bar_w - 1.0).max(1.0),
                            bar_h.max(1.0),
                            series.color,
                        );
                    }
                }
            }
            ChartKind::Line | ChartKind::Area => {
                // One polyline per series, broken at NULL gaps. A run of a single
                // point (isolated between gaps) paints a small marker instead —
                // a stroke needs two points to exist.
                let x_center = |ci: usize| x0 + (ci as f32 + 0.5) * cat_w;
                for series in &self.series {
                    let color = series.color;
                    let mut run: Vec<(f32, f32)> = Vec::new();
                    let flush = |run: &mut Vec<(f32, f32)>, window: &mut Window| {
                        if self.kind == ChartKind::Area && !run.is_empty() {
                            let baseline = y_of(0.0);
                            let mut area = gpui::PathBuilder::fill();
                            area.move_to(point(px(run[0].0), px(baseline)));
                            for &(x, y) in run.iter() {
                                area.line_to(point(px(x), px(y)));
                            }
                            area.line_to(point(px(run[run.len() - 1].0), px(baseline)));
                            area.close();
                            if let Ok(path) = area.build() {
                                window.paint_path(path, color.opacity(0.24));
                            }
                        }
                        if run.len() >= 2 {
                            let mut b = gpui::PathBuilder::stroke(px(2.0));
                            b.move_to(point(px(run[0].0), px(run[0].1)));
                            for &(x, y) in &run[1..] {
                                b.line_to(point(px(x), px(y)));
                            }
                            if let Ok(path) = b.build() {
                                window.paint_path(path, color);
                            }
                        } else if let Some(&(x, y)) = run.first() {
                            fill_quad(window, x - 2.0, y - 2.0, 4.0, 4.0, color);
                        }
                        run.clear();
                    };
                    for ci in 0..n {
                        match series.values.get(ci).copied().flatten() {
                            Some(v) => run.push((x_center(ci), y_of(v))),
                            None => flush(&mut run, window),
                        }
                    }
                    flush(&mut run, window);
                }
            }
            ChartKind::Scatter => {
                let x_center = |ci: usize| x0 + (ci as f32 + 0.5) * cat_w;
                for series in &self.series {
                    for ci in 0..n {
                        if let Some(value) = series.values.get(ci).copied().flatten() {
                            fill_quad(
                                window,
                                x_center(ci) - 3.0,
                                y_of(value) - 3.0,
                                6.0,
                                6.0,
                                series.color,
                            );
                        }
                    }
                }
            }
            ChartKind::Pie | ChartKind::Donut => {
                unreachable!("radial charts return before axes")
            }
        }

        // Category labels: thinned so they never overlap, each truncated to
        // its slot the same way the grid truncates cell text.
        let label_y = y0 + plot_h + 5.0;
        let min_label_w = 56.0f32;
        let step = ((min_label_w / cat_w).ceil() as usize).max(1);
        let slot_w = cat_w * step as f32 - 6.0;
        for ci in (0..n).step_by(step) {
            let text = &self.categories[ci];
            let mut line = shape(text, self.text_dim, tick_fs);
            if f32::from(line.width) > slot_w {
                let byte_idx = line.index_for_x(px(slot_w)).unwrap_or(0);
                let truncated = &text[..floor_char_boundary(text, byte_idx)];
                if truncated.is_empty() {
                    continue;
                }
                line = shape(truncated, self.text_dim, tick_fs);
            }
            let lw = f32::from(line.width);
            let center = x0 + (ci as f32 + 0.5) * cat_w;
            let x = (center - lw / 2.0).max(x0).min(x0 + plot_w - lw);
            let _ = line.paint(
                Point {
                    x: px(x),
                    y: px(label_y),
                },
                line_height(tick_fs),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }

    /// Paint a pie/donut: slices are swept clockwise from 12 o'clock, each
    /// a filled polygon sampled along its arc (donut slices also return
    /// along the inner radius).
    fn paint_radial(&self, ox: f32, oy: f32, width: f32, height: f32, window: &mut Window) {
        let Some(values) = self.series.first().map(|series| &series.values) else {
            return;
        };
        let total: f64 = values
            .iter()
            .flatten()
            .copied()
            .filter(|value| *value > 0.0)
            .sum();
        if total <= 0.0 {
            return;
        }
        let center_x = ox + width / 2.0;
        let center_y = oy + height / 2.0;
        let outer = (width.min(height) * 0.4).max(1.0);
        let inner = if self.kind == ChartKind::Donut {
            outer * 0.56
        } else {
            0.0
        };
        let mut start = -std::f32::consts::FRAC_PI_2;
        for (index, value) in values.iter().enumerate() {
            let value = value.unwrap_or(0.0).max(0.0);
            if value == 0.0 {
                continue;
            }
            let sweep = (value / total) as f32 * std::f32::consts::TAU;
            let end = start + sweep;
            let steps = ((sweep / std::f32::consts::TAU * 96.0).ceil() as usize).clamp(2, 96);
            let mut points = Vec::with_capacity(steps * 2 + 3);
            if inner == 0.0 {
                points.push(point(px(center_x), px(center_y)));
            } else {
                points.push(point(
                    px(center_x + inner * start.cos()),
                    px(center_y + inner * start.sin()),
                ));
            }
            for step in 0..=steps {
                let angle = start + sweep * step as f32 / steps as f32;
                points.push(point(
                    px(center_x + outer * angle.cos()),
                    px(center_y + outer * angle.sin()),
                ));
            }
            if inner > 0.0 {
                for step in (0..=steps).rev() {
                    let angle = start + sweep * step as f32 / steps as f32;
                    points.push(point(
                        px(center_x + inner * angle.cos()),
                        px(center_y + inner * angle.sin()),
                    ));
                }
            }
            let mut path = gpui::PathBuilder::fill();
            path.add_polygon(&points, true);
            if let Ok(path) = path.build() {
                let color = self
                    .slice_colors
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| category_color(&self.theme, index));
                window.paint_path(path, color);
            }
            start = end;
        }
    }
}

/// Whether `series` has a value (not a gap) at `category`.
fn series_index_has_value(series: &PaintSeries, category: usize) -> bool {
    series.values.get(category).is_some_and(|v| v.is_some())
}

/// Fill an axis-aligned rectangle. Same shape as the grid painter's quad
/// helper: background only, no border.
fn fill_quad(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    window.paint_quad(PaintQuad {
        bounds: Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        },
        background: color.into(),
        border_color: gpui::transparent_black(),
        border_widths: Default::default(),
        corner_radii: Default::default(),
        border_style: Default::default(),
    });
}

/// Largest index `<= idx` on a UTF-8 char boundary — guards label truncation
/// against multi-byte input (mirrors the grid painter's helper).
fn floor_char_boundary(text: &str, idx: usize) -> usize {
    let mut idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
