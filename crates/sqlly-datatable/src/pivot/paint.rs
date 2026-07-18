//! Canvas paint pass for the pivot grid: virtualized data cells, indented
//! hierarchical row labels with chevrons, stacked multi-level column headers
//! with merged group spans, subtotal/grand-total styling, and scrollbars.
//!
//! Mirrors the flat grid's split: [`PivotPaintData`] is a cheap snapshot
//! (Arc clones + small copies) taken once per layout pass; the paint
//! functions are pure consumers of it.

use crate::config::{ResolvedColumnFormat, TextAlignment};
use crate::data::CellValue;
use crate::format::format_cell;
use crate::grid::paint::{paint_caret, CELL_TEXT_INSET, ICON_SCALE};
use crate::grid::selection::SortDirection;
use crate::grid::state::SCROLLBAR_SIZE;
use crate::grid::theme::GridTheme;
use crate::pivot::engine::{PivotResult, TOTAL_KEY};
use crate::pivot::state::{
    PivotHitResult, PivotSortKey, PivotState, VisibleCol, VisibleColKind, VisibleRow,
    VisibleRowKind, CHEVRON_SIZE, ROW_INDENT,
};

use gpui::{
    point, px, size, App, Bounds, ContentMask, CursorStyle, Hsla, PaintQuad, Pixels, Point,
    TextAlign, Window,
};
use std::sync::Arc;

/// Lightweight snapshot handed to the canvas paint closure.
#[derive(Clone)]
pub(crate) struct PivotPaintData {
    pub(crate) result: Arc<PivotResult>,
    pub(crate) visible_rows: Arc<Vec<VisibleRow>>,
    pub(crate) visible_cols: Arc<Vec<VisibleCol>>,
    pub(crate) theme: GridTheme,
    pub(crate) value_fmt: ResolvedColumnFormat,
    /// Per row-axis depth: (label alignment, negatives-in-red) from that
    /// field's effective label format.
    pub(crate) row_label_fmts: Vec<(TextAlignment, bool)>,
    /// Per column-axis depth: same, for the column header labels.
    pub(crate) col_label_fmts: Vec<(TextAlignment, bool)>,
    pub(crate) selection: Option<(usize, usize, usize, usize)>,
    pub(crate) hover_hit: Option<PivotHitResult>,
    pub(crate) sort: Option<(PivotSortKey, SortDirection)>,
    /// Flat/tabular row layout: each row field paints in its own row-header
    /// column instead of a single nested/indented column.
    pub(crate) flat_rows: bool,
    /// Number of row fields — the row-header column count when `flat_rows`.
    pub(crate) row_field_count: usize,
    pub(crate) show_row_subtotals: bool,
    pub(crate) scroll_offset: Point<Pixels>,
    pub(crate) row_height: f32,
    pub(crate) header_row_height: f32,
    pub(crate) header_levels: usize,
    pub(crate) row_header_width: f32,
    pub(crate) value_col_width: f32,
    pub(crate) font_size: f32,
    pub(crate) char_width: f32,
    pub(crate) is_ready: bool,
}

impl PivotPaintData {
    pub(crate) fn from_state(s: &PivotState) -> Self {
        let label_fmt = |field: usize| {
            s.label_formats
                .get(field)
                .map_or((TextAlignment::Left, false), |f| {
                    (f.alignment(), f.number.show_negative_red)
                })
        };
        Self {
            result: Arc::clone(&s.result),
            visible_rows: Arc::clone(&s.visible_rows),
            visible_cols: Arc::clone(&s.visible_cols),
            theme: s.theme.clone(),
            value_fmt: s.value_fmt.clone(),
            row_label_fmts: s.config.row_fields.iter().map(|&f| label_fmt(f)).collect(),
            col_label_fmts: s
                .config
                .column_fields
                .iter()
                .map(|&f| label_fmt(f))
                .collect(),
            selection: s.selection,
            hover_hit: s.hover_hit,
            sort: s.sort,
            flat_rows: s.config.flat_rows,
            row_field_count: s.config.row_fields.len(),
            show_row_subtotals: s.config.show_row_subtotals,
            scroll_offset: s.scroll_handle.offset(),
            row_height: s.row_height,
            header_row_height: s.header_row_height,
            header_levels: s.header_levels(),
            row_header_width: s.row_header_width,
            value_col_width: s.value_col_width,
            font_size: s.font_size,
            char_width: s.char_width,
            is_ready: s.config.is_ready(),
        }
    }

    fn header_height(&self) -> f32 {
        self.header_levels as f32 * self.header_row_height
    }

    /// Column-axis node shown at `depth` for visible column `col` (paint-side
    /// twin of `PivotState::col_ancestor_at`).
    fn col_ancestor_at(&self, col: usize, depth: usize) -> Option<usize> {
        let vc = self.visible_cols.get(col)?;
        if vc.key == TOTAL_KEY {
            return None;
        }
        let mut id = vc.key;
        let mut d = self.result.col_nodes.get(id)?.depth;
        if depth > d {
            return None;
        }
        while d > depth {
            id = self.result.col_nodes[id].parent?;
            d -= 1;
        }
        Some(id)
    }

    /// Row-node ids along a flat (tabular) leaf row's grouping path, root →
    /// leaf (one per row field, outermost first). Each id indexes
    /// `result.row_nodes` for that field's label and sort key. Empty for the
    /// grand-total row (`key == TOTAL_KEY`), handled separately.
    fn flat_row_node_ids(&self, row: &VisibleRow) -> Vec<usize> {
        if row.key == TOTAL_KEY {
            return Vec::new();
        }
        let mut ids = Vec::new();
        let mut cur = Some(row.key);
        while let Some(id) = cur {
            ids.push(id);
            cur = self.result.row_nodes.get(id).and_then(|n| n.parent);
        }
        ids.reverse();
        ids
    }

    fn row_label(&self, row: &VisibleRow) -> String {
        match row.kind {
            VisibleRowKind::GrandTotal => "Grand Total".into(),
            _ if row.key == TOTAL_KEY => "Total".into(),
            _ => self
                .result
                .row_nodes
                .get(row.key)
                .map(|n| n.label.clone())
                .unwrap_or_default(),
        }
    }
}

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
        border_color: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
        border_widths: Default::default(),
        corner_radii: Default::default(),
        border_style: Default::default(),
    });
}

fn text_w_approx(text: &str, char_width: f32) -> f32 {
    text.chars().count() as f32 * char_width
}

/// `true` when a raw grouping value is a negative number (drives red group
/// labels for fields configured with `show_negative_red`).
fn cell_is_negative(cell: &CellValue) -> bool {
    match cell {
        CellValue::Integer(v) => *v < 0,
        CellValue::Decimal(v) => *v < 0.0,
        _ => false,
    }
}

/// X position for a label of `text_w` within `[min_x, max_x)` under `align`,
/// clamped so the label never starts left of `min_x`.
fn aligned_x(align: TextAlignment, min_x: f32, max_x: f32, text_w: f32) -> f32 {
    match align {
        TextAlignment::Left => min_x,
        TextAlignment::Center => (min_x + (max_x - min_x - text_w) * 0.5).max(min_x),
        TextAlignment::Right => (max_x - text_w).max(min_x),
    }
}

/// Paint the whole pivot grid into `bounds`.
#[allow(clippy::too_many_lines)]
pub(crate) fn paint_pivot_grid(
    data: &PivotPaintData,
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
) {
    // Hand cursor over the clickable header surfaces (sort targets and
    // expand/collapse chevrons). Must happen during paint — GPUI panics if
    // the cursor style is set from an event handler.
    if matches!(
        data.hover_hit,
        Some(PivotHitResult::ColBorder { .. } | PivotHitResult::RowHeaderBorder)
    ) {
        window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
    } else if matches!(data.hover_hit, Some(PivotHitResult::RowBorder { .. })) {
        window.set_window_cursor_style(CursorStyle::ResizeUpDown);
    } else if matches!(
        data.hover_hit,
        Some(
            PivotHitResult::RowChevron { .. }
                | PivotHitResult::ColChevron { .. }
                | PivotHitResult::ColHeader { .. }
                | PivotHitResult::Corner
        )
    ) {
        window.set_window_cursor_style(CursorStyle::PointingHand);
    }

    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let sw = f32::from(bounds.size.width);
    let sh = f32::from(bounds.size.height);
    let sx = f32::from(data.scroll_offset.x);
    let sy = f32::from(data.scroll_offset.y);
    let theme = &data.theme;
    let row_h = data.row_height;
    let hdr_row_h = data.header_row_height;
    let hdr_h = data.header_height();
    let rhw = data.row_header_width;
    let col_w = data.value_col_width;
    let fs = data.font_size;
    let cw = data.char_width;

    let text_system = window.text_system().clone();
    let font_size = px(fs);
    let line_height = px(fs * 1.2);
    let font = crate::grid::paint::grid_font();
    let bold_font = {
        let mut f = font.clone();
        f.weight = gpui::FontWeight::BOLD;
        f
    };
    let paint_txt_weighted = |win: &mut Window,
                              cx: &mut App,
                              text: &str,
                              x: f32,
                              y: f32,
                              color: Hsla,
                              max_w: Option<f32>,
                              bold: bool| {
        if text.is_empty() {
            return;
        }
        let mk_run = |t: &str| gpui::TextRun {
            len: t.len(),
            color,
            font: if bold {
                bold_font.clone()
            } else {
                font.clone()
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped =
            text_system.shape_line(text.to_owned().into(), font_size, &[mk_run(text)], None);
        let shaped = match max_w {
            Some(mw) if mw <= 0.0 => return,
            Some(mw) if f32::from(shaped.width) > mw => {
                let byte_idx = shaped.index_for_x(px(mw)).unwrap_or(0);
                let truncated = &text[..crate::grid::paint::floor_char_boundary(text, byte_idx)];
                text_system.shape_line(
                    truncated.to_owned().into(),
                    font_size,
                    &[mk_run(truncated)],
                    None,
                )
            }
            _ => shaped,
        };
        let _ = shaped.paint(Point { x: px(x), y: px(y) }, line_height, TextAlign::Left,
                        None,
                        win, cx);
    };
    let paint_txt = |win: &mut Window,
                     cx: &mut App,
                     text: &str,
                     x: f32,
                     y: f32,
                     color: Hsla,
                     max_w: Option<f32>| {
        paint_txt_weighted(win, cx, text, x, y, color, max_w, false);
    };
    // Indicator glyphs (sort arrows, hover hints) paint larger than cell
    // text so they read at a glance (matches the flat grid).
    let icon_fs = px(fs * ICON_SCALE);
    let icon_line_height = px(fs * ICON_SCALE * 1.2);
    let paint_icon =
        |win: &mut Window, cx: &mut App, text: &str, x: f32, y: f32, color: Hsla, bold: bool| {
            let run = gpui::TextRun {
                len: text.len(),
                color,
                font: if bold {
                    bold_font.clone()
                } else {
                    font.clone()
                },
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = text_system.shape_line(text.to_owned().into(), icon_fs, &[run], None);
            let _ = shaped.paint(Point { x: px(x), y: px(y) }, icon_line_height, TextAlign::Left,
                        None,
                        win, cx);
        };

    fill_quad(window, ox, oy, sw, sh, theme.bg);

    let n_rows = data.visible_rows.len();
    let n_cols = data.visible_cols.len();

    // Empty state: prompt for configuration instead of painting a grid.
    if n_rows == 0 || n_cols == 0 {
        let msg = if data.is_ready {
            "No data matches the current pivot filters."
        } else {
            "Drag fields into Rows, Columns, and Values to build the pivot."
        };
        let tw = text_w_approx(msg, cw);
        paint_txt(
            window,
            cx,
            msg,
            ox + (sw - tw) * 0.5,
            oy + sh * 0.4,
            theme.muted_text,
            None,
        );
        return;
    }

    // Scrollbar reservations (mirrors PivotState::scrollbar_reserved).
    let content_w = n_cols as f32 * col_w;
    let content_h = n_rows as f32 * row_h;
    let rsv_w = if content_h > sh - hdr_h {
        SCROLLBAR_SIZE
    } else {
        0.0
    };
    let rsv_h = if content_w > sw - rhw {
        SCROLLBAR_SIZE
    } else {
        0.0
    };

    let clip = |x: f32, y: f32, w: f32, h: f32| {
        Some(ContentMask {
            bounds: Bounds {
                origin: point(px(x), px(y)),
                size: size(px(w.max(0.0)), px(h.max(0.0))),
            },
        })
    };

    // Visible index ranges (virtualization).
    let first_row = ((sy / row_h) as usize).min(n_rows);
    let last_row = (first_row + ((sh - hdr_h) / row_h) as usize + 2).min(n_rows);
    let first_col = ((sx / col_w) as usize).min(n_cols);
    let last_col = (first_col + ((sw - rhw) / col_w) as usize + 2).min(n_cols);

    let is_selected = |r: usize, c: usize| {
        data.selection
            .is_some_and(|(r1, c1, r2, c2)| r >= r1 && r <= r2 && c >= c1 && c <= c2)
    };

    // Background color for a (row, col) cell, before selection.
    let cell_bg = |vr: &VisibleRow, vc: &VisibleCol| -> Option<Hsla> {
        let row_total = matches!(vr.kind, VisibleRowKind::GrandTotal);
        let col_total = matches!(vc.kind, VisibleColKind::GrandTotal);
        if row_total || col_total {
            return Some(theme.pivot_grand_total_bg);
        }
        let row_sub = matches!(vr.kind, VisibleRowKind::GroupHeader { .. });
        let col_sub = matches!(
            vc.kind,
            VisibleColKind::Subtotal | VisibleColKind::Collapsed
        );
        if row_sub {
            return Some(theme.pivot_group_bg);
        }
        if col_sub {
            return Some(theme.pivot_subtotal_bg);
        }
        if vr.zebra {
            return Some(theme.alt_row_bg);
        }
        None
    };

    // ---- Data cells --------------------------------------------------------
    let cells_clip = clip(ox + rhw, oy + hdr_h, sw - rhw - rsv_w, sh - hdr_h - rsv_h);
    window.with_content_mask(cells_clip, |window| {
        for r in first_row..last_row {
            let vr = &data.visible_rows[r];
            let y = oy + hdr_h + r as f32 * row_h - sy;
            let is_open_header = matches!(vr.kind, VisibleRowKind::GroupHeader { expanded: true });
            let show_values = !is_open_header || data.show_row_subtotals;
            for c in first_col..last_col {
                let vc = &data.visible_cols[c];
                let x = ox + rhw + c as f32 * col_w - sx;
                if let Some(bg) = cell_bg(vr, vc) {
                    fill_quad(window, x, y, col_w, row_h, bg);
                }
                if is_selected(r, c) {
                    fill_quad(window, x, y, col_w, row_h, theme.selection_bg);
                }
                if show_values {
                    let cell = data.result.value(vr.key, vc.key);
                    if !matches!(cell, CellValue::None) {
                        let (text, is_neg) = format_cell(cell, &data.value_fmt);
                        let emphasized = matches!(
                            vr.kind,
                            VisibleRowKind::GrandTotal | VisibleRowKind::GroupHeader { .. }
                        ) || matches!(
                            vc.kind,
                            VisibleColKind::GrandTotal
                                | VisibleColKind::Subtotal
                                | VisibleColKind::Collapsed
                        );
                        let color = if is_neg && data.value_fmt.number.show_negative_red {
                            theme.negative_fg
                        } else if emphasized {
                            theme.pivot_total_fg
                        } else {
                            theme.text_fg
                        };
                        let bold = matches!(vr.kind, VisibleRowKind::GrandTotal)
                            || matches!(vc.kind, VisibleColKind::GrandTotal);
                        let text_w = text_w_approx(&text, cw);
                        // Clamp into the cell so oversized values truncate
                        // instead of bleeding into the neighboring column.
                        let tx = match data.value_fmt.alignment() {
                            crate::config::TextAlignment::Left => x + CELL_TEXT_INSET,
                            crate::config::TextAlignment::Center => x + (col_w - text_w) * 0.5,
                            crate::config::TextAlignment::Right => {
                                x + col_w - text_w - CELL_TEXT_INSET
                            }
                        }
                        .max(x + 4.0);
                        paint_txt_weighted(
                            window,
                            cx,
                            &text,
                            tx,
                            y + (row_h - fs) * 0.5,
                            color,
                            Some(x + col_w - 4.0 - tx),
                            bold,
                        );
                    }
                }
                fill_quad(window, x + col_w - 1.0, y, 1.0, row_h, theme.grid_line);
            }
            // Guarded upper bound: zero-sized first-frame bounds (web) make
            // `sw - rhw` negative, and an inverted `clamp` range panics.
            let line_w = (content_w - sx).clamp(0.0, (sw - rhw).max(0.0));
            fill_quad(
                window,
                ox + rhw,
                y + row_h - 1.0,
                line_w,
                1.0,
                theme.grid_line,
            );
        }
    });

    // ---- Row header column -------------------------------------------------
    let row_hdr_clip = clip(ox, oy + hdr_h, rhw, sh - hdr_h - rsv_h);
    window.with_content_mask(row_hdr_clip, |window| {
        for r in first_row..last_row {
            let vr = &data.visible_rows[r];
            let y = oy + hdr_h + r as f32 * row_h - sy;
            let bg = match vr.kind {
                VisibleRowKind::GrandTotal => theme.pivot_grand_total_bg,
                VisibleRowKind::GroupHeader { .. } => theme.pivot_group_bg,
                VisibleRowKind::Leaf if vr.zebra => theme.alt_row_bg,
                VisibleRowKind::Leaf => theme.row_header_bg,
            };
            fill_quad(window, ox, y, rhw, row_h, bg);
            let ty = y + (row_h - fs) * 0.5;
            if data.flat_rows && data.row_field_count >= 1 {
                // Tabular layout: one row-header sub-column per row field.
                let n = data.row_field_count;
                let sub_w = rhw / n as f32;
                if matches!(vr.kind, VisibleRowKind::GrandTotal) {
                    paint_txt_weighted(
                        window,
                        cx,
                        "Grand Total",
                        ox + 4.0,
                        ty,
                        theme.pivot_total_fg,
                        Some(rhw - 8.0),
                        true,
                    );
                } else {
                    let ids = data.flat_row_node_ids(vr);
                    for i in 0..n {
                        let cx0 = ox + i as f32 * sub_w;
                        let node = ids.get(i).and_then(|&id| data.result.row_nodes.get(id));
                        let label = node.map(|n| n.label.as_str()).unwrap_or("");
                        let (align, red) = data
                            .row_label_fmts
                            .get(i)
                            .copied()
                            .unwrap_or((TextAlignment::Left, false));
                        let lx = aligned_x(
                            align,
                            cx0 + 4.0,
                            cx0 + sub_w - 6.0,
                            text_w_approx(label, cw),
                        );
                        let color = if red && node.is_some_and(|n| cell_is_negative(&n.sort_key)) {
                            theme.negative_fg
                        } else {
                            theme.text_fg
                        };
                        paint_txt(
                            window,
                            cx,
                            label,
                            lx,
                            ty,
                            color,
                            Some(cx0 + sub_w - lx - 6.0),
                        );
                        if i + 1 < n {
                            fill_quad(window, cx0 + sub_w - 1.0, y, 1.0, row_h, theme.grid_line);
                        }
                    }
                }
            } else {
                let indent = ox + vr.depth as f32 * ROW_INDENT + 4.0;
                let mut label_x = indent;
                if let VisibleRowKind::GroupHeader { expanded } = vr.kind {
                    paint_caret(window, indent + 2.0, ty, fs, expanded, theme.pivot_total_fg);
                    label_x = indent + CHEVRON_SIZE + 4.0;
                }
                let mut color = match vr.kind {
                    VisibleRowKind::Leaf => theme.text_fg,
                    _ => theme.pivot_total_fg,
                };
                let label = data.row_label(vr);
                let mut lx = label_x;
                if !matches!(vr.kind, VisibleRowKind::GrandTotal) {
                    if let Some(&(align, red)) = data.row_label_fmts.get(vr.depth) {
                        lx = aligned_x(align, label_x, ox + rhw - 6.0, text_w_approx(&label, cw));
                        let neg = data
                            .result
                            .row_nodes
                            .get(vr.key)
                            .is_some_and(|n| cell_is_negative(&n.sort_key));
                        if red && neg {
                            color = theme.negative_fg;
                        }
                    }
                }
                paint_txt_weighted(
                    window,
                    cx,
                    &label,
                    lx,
                    ty,
                    color,
                    Some(ox + rhw - lx - 6.0),
                    matches!(vr.kind, VisibleRowKind::GrandTotal),
                );
            }
            fill_quad(window, ox, y + row_h - 1.0, rhw, 1.0, theme.grid_line);
        }
    });

    // ---- Header block ------------------------------------------------------
    fill_quad(window, ox, oy, sw, hdr_h, theme.header_bg);

    // Caption row (level 0), data side: the column field names, muted.
    let caption_y = oy + (hdr_row_h - fs) * 0.5;
    if !data.result.col_field_names.is_empty() {
        let names = data.result.col_field_names.join(" / ");
        paint_txt(
            window,
            cx,
            &names,
            ox + rhw + 8.0,
            caption_y,
            theme.muted_text,
            Some(sw - rhw - 16.0),
        );
        // Column-label sort (set from the context menu) gets the same bold
        // accent direction glyph as the other sort targets.
        if let Some((PivotSortKey::ColLabel, dir)) = data.sort {
            let names_w = text_w_approx(&names, cw).min(sw - rhw - 16.0);
            paint_icon(
                window,
                cx,
                if dir == SortDirection::Ascending {
                    "↑"
                } else {
                    "↓"
                },
                ox + rhw + 8.0 + names_w + 6.0,
                caption_y - fs * (ICON_SCALE - 1.0) * 0.5,
                theme.sort_indicator,
                true,
            );
        }
    }

    // Column label rows (levels 1..header_levels).
    let hdr_clip = clip(
        ox + rhw,
        oy + hdr_row_h,
        sw - rhw - rsv_w,
        hdr_h - hdr_row_h,
    );
    window.with_content_mask(hdr_clip, |window| {
        let levels = data.header_levels;
        for level in 1..levels {
            let depth = level - 1;
            let ly = oy + level as f32 * hdr_row_h;
            let mut c = first_col;
            while c < last_col {
                let vc = &data.visible_cols[c];
                let x = ox + rhw + c as f32 * col_w - sx;

                // Grand-total column: one label spanning all label levels.
                if vc.kind == VisibleColKind::GrandTotal {
                    if level == 1 {
                        fill_quad(
                            window,
                            x,
                            ly,
                            col_w,
                            hdr_h - hdr_row_h,
                            theme.pivot_grand_total_bg,
                        );
                        let label = "Grand Total";
                        let tw = text_w_approx(label, cw);
                        paint_txt_weighted(
                            window,
                            cx,
                            label,
                            x + (col_w - tw) * 0.5,
                            ly + (hdr_row_h - fs) * 0.5,
                            theme.pivot_total_fg,
                            Some(col_w - 8.0 - cw * ICON_SCALE),
                            true,
                        );
                        let sorted = match data.sort {
                            Some((PivotSortKey::RowsByColumn(k), dir)) if k == vc.key => Some(dir),
                            _ => None,
                        };
                        let hovered = matches!(
                            data.hover_hit,
                            Some(PivotHitResult::ColHeader { col: hc, .. }) if hc == c
                        );
                        let gx = x + col_w - cw * ICON_SCALE - 6.0;
                        let gy = ly + (hdr_row_h - fs * ICON_SCALE) * 0.5;
                        match sorted {
                            Some(dir) => paint_icon(
                                window,
                                cx,
                                if dir == SortDirection::Ascending {
                                    "↑"
                                } else {
                                    "↓"
                                },
                                gx,
                                gy,
                                theme.sort_indicator,
                                true,
                            ),
                            None if hovered => {
                                paint_icon(window, cx, "-", gx, gy, theme.pivot_total_fg, false);
                            }
                            None => {}
                        }
                        fill_quad(window, x, ly, 1.0, hdr_h - hdr_row_h, theme.grid_line);
                    }
                    c += 1;
                    continue;
                }

                // Single-value column when there are no column fields.
                if vc.key == TOTAL_KEY {
                    let label = data.result.value_caption.clone();
                    let tw = text_w_approx(&label, cw);
                    paint_txt(
                        window,
                        cx,
                        &label,
                        x + (col_w - tw) * 0.5,
                        ly + (hdr_row_h - fs) * 0.5,
                        theme.header_fg,
                        Some(col_w - 8.0 - cw * ICON_SCALE),
                    );
                    let sorted = match data.sort {
                        Some((PivotSortKey::RowsByColumn(k), dir)) if k == vc.key => Some(dir),
                        _ => None,
                    };
                    let hovered = matches!(
                        data.hover_hit,
                        Some(PivotHitResult::ColHeader { level: hl, col: hc })
                            if hl == level && hc == c
                    );
                    let gx = x + col_w - cw * ICON_SCALE - 6.0;
                    let gy = ly + (hdr_row_h - fs * ICON_SCALE) * 0.5;
                    match sorted {
                        Some(dir) => paint_icon(
                            window,
                            cx,
                            if dir == SortDirection::Ascending {
                                "↑"
                            } else {
                                "↓"
                            },
                            gx,
                            gy,
                            theme.sort_indicator,
                            true,
                        ),
                        None if hovered => {
                            paint_icon(window, cx, "-", gx, gy, theme.header_fg, false);
                        }
                        None => {}
                    }
                    c += 1;
                    continue;
                }

                let Some(node) = data.col_ancestor_at(c, depth) else {
                    // No ancestor at this depth: below a collapsed group or a
                    // subtotal column. Label subtotal columns "Total" at the
                    // innermost level.
                    if level == levels - 1 && vc.kind == VisibleColKind::Subtotal {
                        fill_quad(window, x, ly, col_w, hdr_row_h, theme.pivot_subtotal_bg);
                        let label = "Total";
                        let tw = text_w_approx(label, cw);
                        paint_txt(
                            window,
                            cx,
                            label,
                            x + (col_w - tw) * 0.5,
                            ly + (hdr_row_h - fs) * 0.5,
                            theme.pivot_total_fg,
                            None,
                        );
                        let sorted = match data.sort {
                            Some((PivotSortKey::RowsByColumn(k), dir)) if k == vc.key => Some(dir),
                            _ => None,
                        };
                        let hovered = matches!(
                            data.hover_hit,
                            Some(PivotHitResult::ColHeader { level: hl, col: hc })
                                if hl == level && hc == c
                        );
                        let gx = x + col_w - cw * ICON_SCALE - 6.0;
                        let gy = ly + (hdr_row_h - fs * ICON_SCALE) * 0.5;
                        match sorted {
                            Some(dir) => paint_icon(
                                window,
                                cx,
                                if dir == SortDirection::Ascending {
                                    "↑"
                                } else {
                                    "↓"
                                },
                                gx,
                                gy,
                                theme.sort_indicator,
                                true,
                            ),
                            None if hovered => {
                                paint_icon(window, cx, "-", gx, gy, theme.pivot_total_fg, false);
                            }
                            None => {}
                        }
                    }
                    c += 1;
                    continue;
                };

                // Merge the run of columns sharing this ancestor. The run may
                // start left of the visible range; anchor the label at the
                // true start so it doesn't jump while scrolling.
                let mut run_end = c + 1;
                while run_end < n_cols && data.col_ancestor_at(run_end, depth) == Some(node) {
                    run_end += 1;
                }
                let mut run_start = c;
                while run_start > 0 && data.col_ancestor_at(run_start - 1, depth) == Some(node) {
                    run_start -= 1;
                }
                let span_x = ox + rhw + run_start as f32 * col_w - sx;
                let span_w = (run_end - run_start) as f32 * col_w;

                let node_ref = &data.result.col_nodes[node];
                let collapsed_here = vc.kind == VisibleColKind::Collapsed && vc.key == node;
                if collapsed_here {
                    fill_quad(
                        window,
                        span_x,
                        ly,
                        span_w,
                        hdr_row_h,
                        theme.pivot_subtotal_bg,
                    );
                }
                let mut label_x = span_x + 6.0;
                if node_ref.depth == depth {
                    if !node_ref.is_leaf() {
                        paint_caret(
                            window,
                            span_x + 4.0,
                            ly + (hdr_row_h - fs) * 0.5,
                            fs,
                            !collapsed_here,
                            theme.pivot_total_fg,
                        );
                        label_x = span_x + 4.0 + CHEVRON_SIZE;
                    }
                    let mut label = node_ref.label.clone();
                    if collapsed_here {
                        label.push_str(" Total");
                    }
                    // The innermost level is a sort target; reserve room at
                    // the right edge for the sort glyph so labels don't jump
                    // when it appears (mirrors the flat grid's header).
                    let sortable = level == levels - 1;
                    let reserve = if sortable { cw * ICON_SCALE + 8.0 } else { 0.0 };
                    let mut color = theme.header_fg;
                    let mut lx = label_x;
                    if let Some(&(align, red)) = data.col_label_fmts.get(depth) {
                        lx = aligned_x(
                            align,
                            label_x,
                            span_x + span_w - 4.0 - reserve,
                            text_w_approx(&label, cw),
                        );
                        if red && cell_is_negative(&node_ref.sort_key) {
                            color = theme.negative_fg;
                        }
                    }
                    paint_txt(
                        window,
                        cx,
                        &label,
                        lx,
                        ly + (hdr_row_h - fs) * 0.5,
                        color,
                        Some(span_x + span_w - lx - 4.0 - reserve),
                    );
                    if sortable {
                        let sorted = match data.sort {
                            Some((PivotSortKey::RowsByColumn(k), dir)) if k == vc.key => Some(dir),
                            _ => None,
                        };
                        let hovered = matches!(
                            data.hover_hit,
                            Some(PivotHitResult::ColHeader { level: hl, col: hc })
                                if hl == level && hc >= run_start && hc < run_end
                        );
                        let gx = span_x + span_w - cw * ICON_SCALE - 6.0;
                        let gy = ly + (hdr_row_h - fs * ICON_SCALE) * 0.5;
                        match sorted {
                            Some(dir) => paint_icon(
                                window,
                                cx,
                                if dir == SortDirection::Ascending {
                                    "↑"
                                } else {
                                    "↓"
                                },
                                gx,
                                gy,
                                theme.sort_indicator,
                                true,
                            ),
                            None if hovered => {
                                paint_icon(window, cx, "-", gx, gy, theme.header_fg, false);
                            }
                            None => {}
                        }
                    }
                }
                fill_quad(window, span_x, ly, 1.0, hdr_row_h, theme.grid_line);
                fill_quad(
                    window,
                    span_x + span_w - 1.0,
                    ly,
                    1.0,
                    hdr_row_h,
                    theme.grid_line,
                );
                c = run_end;
            }
            fill_quad(
                window,
                ox + rhw,
                ly + hdr_row_h - 1.0,
                sw - rhw,
                1.0,
                theme.grid_line,
            );
        }
    });

    // ---- Corner ------------------------------------------------------------
    fill_quad(window, ox, oy, rhw, hdr_h, theme.row_header_bg);
    paint_txt(
        window,
        cx,
        &data.result.value_caption,
        ox + 8.0,
        oy + (hdr_row_h - fs) * 0.5,
        theme.header_fg,
        Some(rhw - 16.0),
    );
    if !data.result.row_field_names.is_empty() {
        let ny = oy + hdr_h - hdr_row_h + (hdr_row_h - fs) * 0.5;
        // Clicking the corner cycles the row-label sort; give it the same
        // affordance as a sortable column header (reserved glyph slot,
        // hover hint, bold accent direction glyph).
        if data.flat_rows && data.row_field_count >= 1 {
            // One field-name header per row-header sub-column, matching the
            // tabular row labels below.
            let n = data.row_field_count;
            let sub_w = rhw / n as f32;
            let hdr_top = oy + hdr_h - hdr_row_h;
            for i in 0..n {
                let cx0 = ox + i as f32 * sub_w;
                let name = data
                    .result
                    .row_field_names
                    .get(i)
                    .map(String::as_str)
                    .unwrap_or("");
                // Only the last sub-column reserves the sort-glyph slot.
                let max_w = if i + 1 == n {
                    sub_w - 8.0 - cw * ICON_SCALE - 6.0
                } else {
                    sub_w - 12.0
                };
                paint_txt(
                    window,
                    cx,
                    name,
                    cx0 + 8.0,
                    ny,
                    theme.muted_text,
                    Some(max_w.max(0.0)),
                );
                if i + 1 < n {
                    fill_quad(
                        window,
                        cx0 + sub_w - 1.0,
                        hdr_top,
                        1.0,
                        hdr_row_h,
                        theme.grid_line,
                    );
                }
            }
        } else {
            let names = data.result.row_field_names.join(" › ");
            paint_txt(
                window,
                cx,
                &names,
                ox + 8.0,
                ny,
                theme.muted_text,
                Some(rhw - 16.0 - cw * ICON_SCALE - 6.0),
            );
        }
        let sorted = match data.sort {
            Some((PivotSortKey::RowLabel, dir)) => Some(dir),
            _ => None,
        };
        let hovered = matches!(data.hover_hit, Some(PivotHitResult::Corner));
        let gx = ox + rhw - cw * ICON_SCALE - 8.0;
        let ny = ny - fs * (ICON_SCALE - 1.0) * 0.5;
        match sorted {
            Some(dir) => paint_icon(
                window,
                cx,
                if dir == SortDirection::Ascending {
                    "↑"
                } else {
                    "↓"
                },
                gx,
                ny,
                theme.sort_indicator,
                true,
            ),
            None if hovered => {
                paint_icon(window, cx, "-", gx, ny, theme.header_fg, false);
            }
            None => {}
        }
    }

    // Frame lines.
    fill_quad(window, ox, oy + hdr_h - 1.0, sw, 1.0, theme.grid_line);
    fill_quad(window, ox + rhw - 1.0, oy, 1.0, sh, theme.grid_line);
    fill_quad(window, ox, oy + hdr_row_h - 1.0, sw, 1.0, theme.grid_line);

    paint_pivot_scrollbars(
        data, window, ox, oy, sw, sh, content_w, content_h, rsv_w, rsv_h,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_pivot_scrollbars(
    data: &PivotPaintData,
    window: &mut Window,
    ox: f32,
    oy: f32,
    sw: f32,
    sh: f32,
    content_w: f32,
    content_h: f32,
    rsv_w: f32,
    rsv_h: f32,
) {
    let theme = &data.theme;
    let hdr_h = data.header_height();
    let rhw = data.row_header_width;
    let sx = f32::from(data.scroll_offset.x);
    let sy = f32::from(data.scroll_offset.y);
    let track_bg = theme.row_header_bg;

    if rsv_w > 0.0 {
        let track_x = ox + sw - SCROLLBAR_SIZE;
        let track_y = oy + hdr_h;
        let track_h = sh - hdr_h - rsv_h;
        if track_h > 0.0 {
            fill_quad(window, track_x, track_y, SCROLLBAR_SIZE, track_h, track_bg);
            fill_quad(window, track_x, track_y, 1.0, track_h, theme.grid_line);
            let max_y = (content_h - track_h).max(0.0);
            let thumb_h = ((track_h * (track_h / content_h)).max(20.0)).min(track_h);
            let frac = if max_y > 0.0 { sy / max_y } else { 0.0 };
            fill_quad(
                window,
                track_x + 3.0,
                track_y + frac * (track_h - thumb_h),
                SCROLLBAR_SIZE - 6.0,
                thumb_h,
                theme.scrollbar_thumb,
            );
        }
    }
    if rsv_h > 0.0 {
        let track_x = ox + rhw;
        let track_y = oy + sh - SCROLLBAR_SIZE;
        let track_w = sw - rhw - rsv_w;
        if track_w > 0.0 {
            fill_quad(window, track_x, track_y, track_w, SCROLLBAR_SIZE, track_bg);
            fill_quad(window, track_x, track_y, track_w, 1.0, theme.grid_line);
            let max_x = (content_w - track_w).max(0.0);
            let thumb_w = ((track_w * (track_w / content_w)).max(20.0)).min(track_w);
            let frac = if max_x > 0.0 { sx / max_x } else { 0.0 };
            fill_quad(
                window,
                track_x + frac * (track_w - thumb_w),
                track_y + 3.0,
                thumb_w,
                SCROLLBAR_SIZE - 6.0,
                theme.scrollbar_thumb,
            );
        }
    }
    if rsv_w > 0.0 && rsv_h > 0.0 {
        fill_quad(
            window,
            ox + sw - SCROLLBAR_SIZE,
            oy + sh - SCROLLBAR_SIZE,
            SCROLLBAR_SIZE,
            SCROLLBAR_SIZE,
            track_bg,
        );
    }
}
