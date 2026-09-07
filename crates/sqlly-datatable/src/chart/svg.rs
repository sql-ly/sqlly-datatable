//! SVG export: render a [`ChartPaint`] as a standalone SVG document
//! mirroring the canvas painter's geometry.

use gpui::Hsla;

use super::config::ChartKind;
use super::model::format_tick;
use super::paint::{category_color, ChartPaint};

/// `color` as an `#rrggbb` hex string (alpha is reported separately via the
/// `*-opacity` attributes).
fn hsla_hex(color: Hsla) -> String {
    let rgba: gpui::Rgba = color.into();
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u8
    )
}

fn svg_fill(color: Hsla) -> String {
    let rgba: gpui::Rgba = color.into();
    if rgba.a < 0.999 {
        format!(
            "fill=\"{}\" fill-opacity=\"{:.3}\"",
            hsla_hex(color),
            rgba.a
        )
    } else {
        format!("fill=\"{}\"", hsla_hex(color))
    }
}

fn svg_stroke(color: Hsla) -> String {
    let rgba: gpui::Rgba = color.into();
    if rgba.a < 0.999 {
        format!(
            "stroke=\"{}\" stroke-opacity=\"{:.3}\"",
            hsla_hex(color),
            rgba.a
        )
    } else {
        format!("stroke=\"{}\"", hsla_hex(color))
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render `paint` as a standalone SVG document mirroring the canvas
/// painter's geometry — marks, gridlines, tick labels, thinned category
/// labels — plus the legend (rendered as UI above the canvas on screen) as a
/// top band. Colors come straight from the paint struct's theme-resolved
/// `Hsla`s.
///
/// Deliberate approximation: with no text shaper available, label widths are
/// estimated at 0.6 × font size per character, so gutters and legend spacing
/// can differ a few pixels from the on-screen canvas; category labels are
/// thinned like the canvas but not truncated (SVG text may overhang its
/// slot slightly).
#[must_use]
pub fn chart_svg(paint: &ChartPaint, width: f32, height: f32) -> String {
    use std::fmt::Write as _;
    let p = paint;
    let fs = p.font_size;
    let tick_fs = fs * 0.85;
    let char_w = tick_fs * 0.6;
    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" font-family=\"sans-serif\">"
    );
    let _ = write!(
        svg,
        "<rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" {}/>",
        svg_fill(p.background)
    );

    // Legend band: series swatches (category swatches for radial kinds).
    let radial = matches!(p.kind, ChartKind::Pie | ChartKind::Donut);
    let legend: Vec<(&str, Hsla)> = if radial {
        p.categories
            .iter()
            .enumerate()
            .map(|(i, category)| {
                let color = p
                    .slice_colors
                    .get(i)
                    .copied()
                    .unwrap_or_else(|| category_color(&p.theme, i));
                (category.as_str(), color)
            })
            .collect()
    } else {
        p.series
            .iter()
            .map(|series| (series.name.as_str(), series.color))
            .collect()
    };
    let legend_h = fs * 1.6 + 6.0;
    let mut lx = 8.0f32;
    for (name, color) in &legend {
        let _ = write!(
            svg,
            "<rect x=\"{lx:.1}\" y=\"{:.1}\" width=\"10\" height=\"10\" rx=\"2\" {}/>",
            legend_h / 2.0 - 5.0,
            svg_fill(*color)
        );
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{tick_fs:.1}\" {}>{}</text>",
            lx + 14.0,
            legend_h / 2.0 + tick_fs * 0.35,
            svg_fill(p.text_dim),
            xml_escape(name)
        );
        lx += 14.0 + name.chars().count() as f32 * char_w + 14.0;
    }

    if radial {
        svg_radial_marks(p, width, height, legend_h, &mut svg);
        svg.push_str("</svg>");
        return svg;
    }

    let n = p.categories.len();
    let n_series = p.series.len();
    let ticks = p.axis.ticks();
    let widest_tick = ticks
        .iter()
        .map(|t| format_tick(*t).chars().count())
        .max()
        .unwrap_or(1) as f32
        * char_w;
    let left = widest_tick + 14.0;
    let right = 10.0;
    let top = legend_h + 8.0;
    let bottom = fs * 1.4 + 8.0;
    let plot_w = width - left - right;
    let plot_h = height - top - bottom;
    if plot_w < 20.0 || plot_h < 20.0 || n == 0 || n_series == 0 {
        svg.push_str("</svg>");
        return svg;
    }
    let y_of = |v: f64| top + plot_h * (1.0 - p.axis.fraction(v) as f32);

    // Gridlines + right-aligned tick labels.
    for t in &ticks {
        let y = y_of(*t);
        let _ = write!(
            svg,
            "<line x1=\"{left:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" {} stroke-width=\"1\"/>",
            left + plot_w,
            svg_stroke(p.grid_line)
        );
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"{tick_fs:.1}\" {}>{}</text>",
            left - 6.0,
            y + tick_fs * 0.35,
            svg_fill(p.text_dim),
            xml_escape(&format_tick(*t))
        );
    }

    let cat_w = plot_w / n as f32;
    let x_center = |ci: usize| left + (ci as f32 + 0.5) * cat_w;
    match p.kind {
        ChartKind::Bar | ChartKind::Histogram => {
            let baseline = y_of(0.0);
            let band = if p.kind == ChartKind::Histogram {
                cat_w.max(1.0)
            } else {
                (cat_w * 0.72).max(1.0)
            };
            let bar_w = (band / n_series as f32).max(1.0);
            for ci in 0..n {
                let group_x = left + ci as f32 * cat_w + (cat_w - band) / 2.0;
                for (si, series) in p.series.iter().enumerate() {
                    let Some(v) = series.values.get(ci).copied().flatten() else {
                        continue;
                    };
                    let yv = y_of(v);
                    let (bar_y, bar_h) = if yv <= baseline {
                        (yv, baseline - yv)
                    } else {
                        (baseline, yv - baseline)
                    };
                    let _ = write!(
                        svg,
                        "<rect x=\"{:.1}\" y=\"{bar_y:.1}\" width=\"{:.1}\" height=\"{:.1}\" {}/>",
                        group_x + si as f32 * bar_w,
                        (bar_w - 1.0).max(1.0),
                        bar_h.max(1.0),
                        svg_fill(series.color)
                    );
                }
            }
        }
        ChartKind::Line | ChartKind::Area => {
            let baseline = y_of(0.0);
            for series in &p.series {
                let color = series.color;
                let mut run: Vec<(f32, f32)> = Vec::new();
                let flush = |run: &mut Vec<(f32, f32)>, svg: &mut String| {
                    if p.kind == ChartKind::Area && !run.is_empty() {
                        let mut d = format!("M{:.1} {baseline:.1}", run[0].0);
                        for &(x, y) in run.iter() {
                            let _ = write!(d, " L{x:.1} {y:.1}");
                        }
                        let _ = write!(d, " L{:.1} {baseline:.1} Z", run[run.len() - 1].0);
                        let _ = write!(svg, "<path d=\"{d}\" {}/>", svg_fill(color.opacity(0.24)));
                    }
                    if run.len() >= 2 {
                        let mut d = format!("M{:.1} {:.1}", run[0].0, run[0].1);
                        for &(x, y) in &run[1..] {
                            let _ = write!(d, " L{x:.1} {y:.1}");
                        }
                        let _ = write!(
                            svg,
                            "<path d=\"{d}\" fill=\"none\" {} stroke-width=\"2\"/>",
                            svg_stroke(color)
                        );
                    } else if let Some(&(x, y)) = run.first() {
                        let _ = write!(
                            svg,
                            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"4\" height=\"4\" {}/>",
                            x - 2.0,
                            y - 2.0,
                            svg_fill(color)
                        );
                    }
                    run.clear();
                };
                for ci in 0..n {
                    match series.values.get(ci).copied().flatten() {
                        Some(v) => run.push((x_center(ci), y_of(v))),
                        None => flush(&mut run, &mut svg),
                    }
                }
                flush(&mut run, &mut svg);
            }
        }
        ChartKind::Scatter => {
            for series in &p.series {
                for ci in 0..n {
                    if let Some(value) = series.values.get(ci).copied().flatten() {
                        let _ = write!(
                            svg,
                            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"6\" height=\"6\" {}/>",
                            x_center(ci) - 3.0,
                            y_of(value) - 3.0,
                            svg_fill(series.color)
                        );
                    }
                }
            }
        }
        ChartKind::Pie | ChartKind::Donut => unreachable!("radial charts return before axes"),
    }

    // Category labels, thinned like the canvas.
    let label_y = top + plot_h + 5.0 + tick_fs;
    let min_label_w = 56.0f32;
    let step = ((min_label_w / cat_w).ceil() as usize).max(1);
    for ci in (0..n).step_by(step) {
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{label_y:.1}\" text-anchor=\"middle\" font-size=\"{tick_fs:.1}\" {}>{}</text>",
            x_center(ci),
            svg_fill(p.text_dim),
            xml_escape(&p.categories[ci])
        );
    }

    svg.push_str("</svg>");
    svg
}

/// Pie/donut wedges into `svg`, mirroring the canvas radial painter's
/// polygon sampling within the area below the legend band.
fn svg_radial_marks(p: &ChartPaint, width: f32, height: f32, legend_h: f32, svg: &mut String) {
    use std::fmt::Write as _;
    let Some(values) = p.series.first().map(|series| &series.values) else {
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
    let center_x = width / 2.0;
    let center_y = legend_h + (height - legend_h) / 2.0;
    let outer = (width.min(height - legend_h) * 0.4).max(1.0);
    let inner = if p.kind == ChartKind::Donut {
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
        let mut d = if inner == 0.0 {
            format!("M{center_x:.1} {center_y:.1}")
        } else {
            format!(
                "M{:.1} {:.1}",
                center_x + inner * start.cos(),
                center_y + inner * start.sin()
            )
        };
        for step in 0..=steps {
            let angle = start + sweep * step as f32 / steps as f32;
            let _ = write!(
                d,
                " L{:.1} {:.1}",
                center_x + outer * angle.cos(),
                center_y + outer * angle.sin()
            );
        }
        if inner > 0.0 {
            for step in (0..=steps).rev() {
                let angle = start + sweep * step as f32 / steps as f32;
                let _ = write!(
                    d,
                    " L{:.1} {:.1}",
                    center_x + inner * angle.cos(),
                    center_y + inner * angle.sin()
                );
            }
        }
        d.push_str(" Z");
        let color = p
            .slice_colors
            .get(index)
            .copied()
            .unwrap_or_else(|| category_color(&p.theme, index));
        let _ = write!(svg, "<path d=\"{d}\" {}/>", svg_fill(color));
        start = end;
    }
}
