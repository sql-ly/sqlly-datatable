//! Canvas paint functions and the lightweight snapshot that GPUI's
//! `canvas(...)` closure hands to the painter.
//!
//! `PaintData` is constructed once per layout pass; it clones the small
//! state needed for paint (selection, scroll offsets, resolved formats) but
//! keeps the bulk [`crate::data::GridData`] behind a count of visible rows
//! rather than copying the entire dataset.

use crate::config::ResolvedColumnFormat;
use crate::data::Column;
use crate::grid::menu::{self};
use crate::grid::selection::{
    is_cell_selected, is_column_selected, is_row_selected, HitResult, Selection, SortDirection,
};
use crate::grid::state::{state_inner, GridDisplayRow, GridState, RowGroup, SCROLLBAR_SIZE};
use crate::grid::theme::GridTheme;

use gpui::{
    point, px, size, App, Bounds, ContentMask, CursorStyle, Hsla, PaintQuad, Pixels, Point, Window,
};
use std::sync::Arc;

const fn hsla_const(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

/// The monospace face used for all canvas-painted text (cells, headers,
/// pivot, status bar). The generic `"monospace"` request resolves to a
/// single-face family on macOS, which silently drops the bold and italic
/// variants the painters ask for — so name a real family per platform and
/// carry the others as fallbacks.
pub(crate) fn grid_font() -> gpui::Font {
    #[cfg(target_os = "macos")]
    let family = "Menlo";
    #[cfg(target_os = "windows")]
    let family = "Consolas";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let family = "DejaVu Sans Mono";
    gpui::Font {
        family: family.into(),
        features: gpui::FontFeatures::default(),
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            "Menlo".into(),
            "Consolas".into(),
            "DejaVu Sans Mono".into(),
            "monospace".into(),
        ])),
    }
}

/// Advance width of one glyph of [`grid_font`] at the given size, from the
/// family's em-square metrics (Menlo and DejaVu Sans Mono ≈ 0.602 em,
/// Consolas ≈ 0.55 em). Seed value for the `char_width` approximations;
/// hosts that change the font size or family can override the stored field.
pub(crate) fn default_char_width(font_size: f32) -> f32 {
    #[cfg(target_os = "windows")]
    let ratio = 0.55;
    #[cfg(not(target_os = "windows"))]
    let ratio = 0.6022;
    font_size * ratio
}

#[derive(Clone)]
pub(crate) struct PaintData {
    pub(crate) display_rows: Arc<Vec<GridDisplayRow>>,
    pub(crate) row_groups: Arc<Vec<RowGroup>>,
    pub(crate) grouped_column: Option<usize>,
    /// Windowed-row mode (see [`crate::grid::state::RowWindow`]): the grid
    /// presents `total_rows` virtual rows while `rows` holds only a resident
    /// window starting at `offset`.
    pub(crate) window: Option<crate::grid::state::RowWindow>,
    pub(crate) selection: Selection,
    pub(crate) sort: Option<(usize, SortDirection)>,
    pub(crate) theme: GridTheme,
    pub(crate) columns: Vec<Column>,
    pub(crate) resolved_formats: Vec<ResolvedColumnFormat>,
    pub(crate) rows: Arc<Vec<Vec<crate::data::CellValue>>>,
    pub(crate) filters_active: Vec<bool>,
    pub(crate) scroll_offset: Point<Pixels>,
    pub(crate) row_height: f32,
    pub(crate) header_height: f32,
    pub(crate) row_header_width: f32,
    pub(crate) font_size: f32,
    pub(crate) char_width: f32,
    pub(crate) drag_rect: Option<(Point<Pixels>, Point<Pixels>)>,
    pub(crate) hover_hit: Option<HitResult>,
    /// Hint painted centered in the data area when there are zero rows.
    pub(crate) empty_text: String,
}

impl PaintData {
    pub(crate) fn from_state(s: &GridState) -> Self {
        Self {
            display_rows: Arc::clone(&s.display_rows),
            row_groups: Arc::clone(&s.row_groups),
            grouped_column: s.grouped_column(),
            window: s.window,
            selection: s.selection.clone(),
            sort: s.sort,
            theme: s.theme.clone(),
            columns: s.data.columns.clone(),
            resolved_formats: s.resolved_formats.clone(),
            rows: Arc::clone(&s.data_rows),
            filters_active: s.filters.iter().map(|f| f.is_active()).collect(),
            scroll_offset: s.scroll_handle.offset(),
            row_height: s.row_height,
            header_height: s.header_height,
            row_header_width: s.row_header_width,
            font_size: s.font_size,
            char_width: s.char_width,
            drag_rect: s.drag_screen_rect(),
            hover_hit: s.hover_hit,
            empty_text: s.config.empty_text.clone(),
        }
    }

    /// Number of rows the grid presents (virtual total in windowed mode).
    fn display_row_count(&self) -> usize {
        self.window
            .map(|w| w.total_rows)
            .unwrap_or(self.display_rows.len())
    }

    /// Maps a display row to an index into `rows`, or `None` when the row is
    /// not resident (windowed rows that have not been paged in yet — painted
    /// as an empty placeholder row).
    fn resident_row(&self, display_row: usize) -> Option<usize> {
        match self.window {
            Some(w) => display_row
                .checked_sub(w.offset)
                .filter(|r| *r < self.rows.len()),
            None => match self.display_rows.get(display_row) {
                Some(GridDisplayRow::Data { source_row, .. }) => Some(*source_row),
                _ => None,
            },
        }
    }

    fn group(&self, display_row: usize) -> Option<&RowGroup> {
        match self.display_rows.get(display_row) {
            Some(GridDisplayRow::GroupHeader { group }) => self.row_groups.get(*group),
            _ => None,
        }
    }

    fn row_number(&self, display_row: usize) -> usize {
        match self.display_rows.get(display_row) {
            Some(GridDisplayRow::Data { flat_row, .. }) => flat_row + 1,
            _ => display_row + 1,
        }
    }
}

pub(crate) struct StatusBarData {
    pub(crate) text: String,
    pub(crate) theme: GridTheme,
    pub(crate) font_size: f32,
}

impl StatusBarData {
    pub(crate) fn from_state(s: &GridState) -> Self {
        Self {
            text: state_inner::format_current_status(s),
            theme: s.theme.clone(),
            font_size: s.font_size,
        }
    }
}

/// Largest index `<= idx` that falls on a UTF-8 char boundary of `text`.
/// Guards the truncation slice: the shaper's byte index should always land
/// on a boundary, but a mid-char index from a foreign glyph cluster would
/// otherwise panic the paint pass on multi-byte input (emoji, CJK).
pub(crate) fn floor_char_boundary(text: &str, idx: usize) -> usize {
    let mut idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
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

/// Painted size multiplier for indicator glyphs (sort arrows, hover hints,
/// the active-filter marker), relative to the grid font size.
pub(crate) const ICON_SCALE: f32 = 4.0 / 3.0;

/// Glyph painted next to a column's sort button while a filter is active on
/// it. An emoji, so it renders from the system color-emoji fallback rather
/// than the grid's text color.
const FILTER_ICON: &str = "🔽";

pub(crate) fn paint_scrollbars(
    data: &PaintData,
    window: &mut Window,
    ox: f32,
    oy: f32,
    sw: f32,
    sh: f32,
    theme: &GridTheme,
) {
    let scroll = data.scroll_offset;
    let (content_w, content_h) = (
        data.columns.iter().map(|c| c.width).sum::<f32>(),
        data.display_row_count() as f32 * data.row_height,
    );
    let vw_full = sw - data.row_header_width;
    let vh_full = sh - data.header_height;
    let has_v = content_h > vh_full;
    let has_h = content_w > vw_full;
    let reserved_w = if has_v { SCROLLBAR_SIZE } else { 0.0 };
    let reserved_h = if has_h { SCROLLBAR_SIZE } else { 0.0 };
    let vw = vw_full - reserved_w;
    let vh = vh_full - reserved_h;
    let max_x = (content_w - vw).max(0.0);
    let max_y = (content_h - vh).max(0.0);
    let (sx, sy) = (f32::from(scroll.x), f32::from(scroll.y));
    let track_bg = theme.row_header_bg;

    if has_v {
        let track_x = ox + sw - SCROLLBAR_SIZE;
        let track_y = oy + data.header_height;
        let track_h = sh - data.header_height - reserved_h;
        if track_h > 0.0 {
            fill_quad(window, track_x, track_y, SCROLLBAR_SIZE, track_h, track_bg);
            // 1px separator so the track reads as a scrollbar gutter rather
            // than blending into the last column.
            fill_quad(window, track_x, track_y, 1.0, track_h, theme.grid_line);
            let thumb_h = ((track_h * (vh / content_h)).max(20.0)).min(track_h);
            let frac = if max_y > 0.0 { sy / max_y } else { 0.0 };
            let thumb_y = track_y + frac * (track_h - thumb_h);
            fill_quad(
                window,
                track_x + 3.0,
                thumb_y,
                SCROLLBAR_SIZE - 6.0,
                thumb_h,
                theme.scrollbar_thumb,
            );
        }
    }
    if has_h {
        let track_x = ox + data.row_header_width;
        let track_y = oy + sh - SCROLLBAR_SIZE;
        let track_w = sw - data.row_header_width - reserved_w;
        if track_w > 0.0 {
            fill_quad(window, track_x, track_y, track_w, SCROLLBAR_SIZE, track_bg);
            // 1px separator so the track reads as a scrollbar gutter rather
            // than blending into the bottom row.
            fill_quad(window, track_x, track_y, track_w, 1.0, theme.grid_line);
            let thumb_w = ((track_w * (vw / content_w)).max(20.0)).min(track_w);
            let frac = if max_x > 0.0 { sx / max_x } else { 0.0 };
            let thumb_x = track_x + frac * (track_w - thumb_w);
            fill_quad(
                window,
                thumb_x,
                track_y + 3.0,
                thumb_w,
                SCROLLBAR_SIZE - 6.0,
                theme.scrollbar_thumb,
            );
        }
    }
    if has_v && has_h {
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

pub(crate) fn paint_grid(
    data: &PaintData,
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
) {
    if matches!(data.hover_hit, Some(HitResult::ColumnBorder(_))) {
        window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
    }
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let sw = f32::from(bounds.size.width);
    let sh = f32::from(bounds.size.height);
    let (sx, sy) = (
        f32::from(data.scroll_offset.x),
        f32::from(data.scroll_offset.y),
    );
    let row_h = data.row_height;
    let hdr_h = data.header_height;
    let rhw = data.row_header_width;
    let fs = data.font_size;
    let cw = data.char_width;
    let theme = &data.theme;

    let text_system = window.text_system().clone();
    let font_size = px(fs);
    let line_height = px(fs * 1.2);
    let font = grid_font();
    let italic_font = {
        let mut f = font.clone();
        f.style = gpui::FontStyle::Italic;
        f
    };
    let bold_font = {
        let mut f = font.clone();
        f.weight = gpui::FontWeight::BOLD;
        f
    };
    let paint_txt_styled = |win: &mut Window,
                            cx: &mut App,
                            text: &str,
                            x: f32,
                            y: f32,
                            color: Hsla,
                            max_w: Option<f32>,
                            italic: bool,
                            bold: bool| {
        let face = if italic {
            &italic_font
        } else if bold {
            &bold_font
        } else {
            &font
        };
        let mk_run = |t: &str| gpui::TextRun {
            len: t.len(),
            color,
            font: face.clone(),
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
                let truncated = &text[..floor_char_boundary(text, byte_idx)];
                text_system.shape_line(
                    truncated.to_owned().into(),
                    font_size,
                    &[mk_run(truncated)],
                    None,
                )
            }
            _ => shaped,
        };
        let _ = shaped.paint(Point { x: px(x), y: px(y) }, line_height, win, cx);
    };
    let paint_txt = |win: &mut Window,
                     cx: &mut App,
                     text: &str,
                     x: f32,
                     y: f32,
                     color: Hsla,
                     max_w: Option<f32>| {
        paint_txt_styled(win, cx, text, x, y, color, max_w, false, false);
    };
    let paint_txt_bold = |win: &mut Window,
                          cx: &mut App,
                          text: &str,
                          x: f32,
                          y: f32,
                          color: Hsla,
                          max_w: Option<f32>| {
        paint_txt_styled(win, cx, text, x, y, color, max_w, false, true);
    };
    // Indicator glyphs (sort arrows, hover hints) paint larger than cell
    // text so they read at a glance.
    let icon_fs = px(fs * ICON_SCALE);
    let icon_line_height = px(fs * ICON_SCALE * 1.2);
    let paint_icon =
        |win: &mut Window, cx: &mut App, text: &str, x: f32, y: f32, color: Hsla, bold: bool| {
            let face = if bold { &bold_font } else { &font };
            let run = gpui::TextRun {
                len: text.len(),
                color,
                font: face.clone(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = text_system.shape_line(text.to_owned().into(), icon_fs, &[run], None);
            let _ = shaped.paint(Point { x: px(x), y: px(y) }, icon_line_height, win, cx);
        };

    fill_quad(window, ox, oy, sw, sh, theme.bg);
    fill_quad(window, ox, oy, rhw, sh, theme.row_header_bg);

    let data_y = hdr_h;
    let visible_h = sh - data_y;
    let first_row = ((sy / row_h) as usize).min(data.display_row_count());
    let vis_rows = ((visible_h / row_h) as usize) + 1;
    let last_row = (first_row + vis_rows).min(data.display_row_count());

    // Scrollbar reservations — mirrors `paint_scrollbars`. Cell/header
    // painting is clipped so partially visible rows/columns never bleed
    // past the grid bounds or under the scrollbar strips.
    let (content_w, content_h) = (
        data.columns.iter().map(|c| c.width).sum::<f32>(),
        data.display_row_count() as f32 * row_h,
    );
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

    // Row backgrounds and horizontal grid lines stop at the last column's
    // right edge; the area past the columns stays blank.
    let cols_w = (content_w - sx).clamp(0.0, sw - rhw);
    let cells_clip = clip(ox + rhw, oy + data_y, sw - rhw - rsv_w, sh - data_y - rsv_h);
    window.with_content_mask(cells_clip, |window| {
        for dr in first_row..last_row {
            let y = oy + data_y + (dr as f32 * row_h) - sy;
            if y + row_h < oy + data_y || y > oy + sh {
                continue;
            }
            if let Some(group) = data.group(dr) {
                fill_quad(window, ox + rhw, y, cols_w, row_h, theme.pivot_group_bg);
                let marker = if group.collapsed { ">" } else { "v" };
                let noun = if group.row_count == 1 { "row" } else { "rows" };
                let label = format!("{marker}  {}  ({} {noun})", group.label, group.row_count);
                paint_txt_bold(
                    window,
                    cx,
                    &label,
                    ox + rhw + 8.0,
                    y + (row_h - fs) * 0.5,
                    theme.pivot_total_fg,
                    Some(cols_w - 16.0),
                );
                fill_quad(window, ox, y + row_h, rhw + cols_w, 1.0, theme.grid_line);
                continue;
            }
            let row_sel = is_row_selected(&data.selection, dr);
            let alt = dr % 2 == 1;
            if row_sel {
                fill_quad(window, ox + rhw, y, cols_w, row_h, theme.selection_bg);
            } else if alt {
                fill_quad(window, ox + rhw, y, cols_w, row_h, theme.alt_row_bg);
            }
            // Windowed rows that are not resident paint as an empty
            // placeholder row (background + grid lines only) — the host is
            // already paging them in.
            let Some(row_idx) = data.resident_row(dr) else {
                fill_quad(window, ox, y + row_h, rhw + cols_w, 1.0, theme.grid_line);
                continue;
            };

            let mut col_x = rhw - sx;
            for (ci, col) in data.columns.iter().enumerate() {
                let x = ox + col_x;
                let w = col.width;
                if x + w < ox + rhw || x > ox + sw {
                    col_x += w;
                    continue;
                }
                let cell_sel = is_cell_selected(&data.selection, dr, ci);
                if cell_sel {
                    fill_quad(window, x, y, w, row_h, theme.selection_bg);
                }
                let cell = &data.rows[row_idx][ci];
                let fmt = &data.resolved_formats[ci];
                let is_null = matches!(cell, crate::data::CellValue::None);
                if is_null && fmt.null.background && !cell_sel {
                    fill_quad(window, x, y, w, row_h, theme.null_bg);
                }
                let (text, is_neg) = if is_null {
                    (fmt.null.text.clone(), false)
                } else {
                    crate::format::format_cell(cell, fmt)
                };
                let color = if is_null {
                    theme.null_fg
                } else if is_neg && fmt.number.show_negative_red {
                    theme.negative_fg
                } else {
                    theme.text_fg
                };
                let text_w = text_w_approx(&text, cw);
                let tx = match fmt.alignment() {
                    crate::config::TextAlignment::Left => x + 8.0,
                    crate::config::TextAlignment::Center => x + (w - text_w) * 0.5,
                    crate::config::TextAlignment::Right => x + w - text_w - 8.0,
                };
                let ty = y + (row_h - fs) * 0.5;
                paint_txt_styled(
                    window,
                    cx,
                    &text,
                    tx,
                    ty,
                    color,
                    Some(w - 16.0),
                    is_null && fmt.null.italic,
                    false,
                );
                fill_quad(window, x + w, y, 1.0, row_h, theme.grid_line);
                col_x += w;
            }
            fill_quad(window, ox, y + row_h, rhw + cols_w, 1.0, theme.grid_line);
        }
    });

    let row_header_clip = clip(ox, oy + data_y, rhw, sh - data_y - rsv_h);
    window.with_content_mask(row_header_clip, |window| {
        for dr in first_row..last_row {
            let y = oy + data_y + (dr as f32 * row_h) - sy;
            if y + row_h < oy + data_y || y > oy + sh {
                continue;
            }
            if data.group(dr).is_some() {
                fill_quad(window, ox, y, rhw, row_h, theme.pivot_group_bg);
                fill_quad(window, ox, y + row_h, rhw, 1.0, theme.grid_line);
                continue;
            }
            let row_sel = is_row_selected(&data.selection, dr);
            let alt = dr % 2 == 1;
            let rh_bg = if row_sel {
                theme.selection_bg
            } else if alt {
                theme.alt_row_bg
            } else {
                theme.row_header_bg
            };
            fill_quad(window, ox, y, rhw, row_h, rh_bg);
            paint_txt(
                window,
                cx,
                &data.row_number(dr).to_string(),
                ox + 6.0,
                y + (row_h - fs) * 0.5,
                theme.header_fg,
                None,
            );
            fill_quad(window, ox, y + row_h, rhw, 1.0, theme.grid_line);
        }
    });

    fill_quad(window, ox, oy, sw, hdr_h, theme.header_bg);
    let header_clip = clip(ox + rhw, oy, sw - rhw - rsv_w, hdr_h);
    window.with_content_mask(header_clip, |window| {
        let mut col_x = rhw - sx;
        for (ci, col) in data.columns.iter().enumerate() {
            let x = ox + col_x;
            let w = col.width;
            if x + w < ox + rhw || x > ox + sw {
                col_x += w;
                continue;
            }
            if is_column_selected(&data.selection, ci) {
                fill_quad(window, x, oy, w, hdr_h, theme.selection_bg);
            }
            if data.grouped_column == Some(ci) {
                fill_quad(window, x, oy + hdr_h - 3.0, w, 3.0, theme.sort_indicator);
            }
            paint_txt_bold(
                window,
                cx,
                &col.name,
                x + 8.0,
                oy + (hdr_h - fs) * 0.5,
                theme.header_fg,
                Some(w - 28.0),
            );
            let btn_w = 20.0;
            let btn_x = x + w - btn_w;
            let btn_y = oy + 4.0;
            let btn_h = hdr_h - 8.0;
            let sorted = matches!(data.sort, Some((sc, _)) if sc == ci);
            let hovered = matches!(
                data.hover_hit,
                Some(HitResult::SortButton(h) | HitResult::ColumnHeader(h)) if h == ci
            );
            // The sort affordance stays quiet at rest: the outlined button
            // (with a "-" cycle hint) appears only while the column header is
            // hovered, and a sorted column shows just the accent direction
            // glyph. The hit target itself is unchanged.
            if hovered && !sorted {
                fill_quad(window, btn_x, btn_y, btn_w, btn_h, theme.header_bg);
                // 1px outline around the button (top, bottom, left, right).
                fill_quad(window, btn_x, btn_y, btn_w, 1.0, theme.grid_line);
                fill_quad(
                    window,
                    btn_x,
                    btn_y + btn_h - 1.0,
                    btn_w,
                    1.0,
                    theme.grid_line,
                );
                fill_quad(window, btn_x, btn_y, 1.0, btn_h, theme.grid_line);
                fill_quad(
                    window,
                    btn_x + btn_w - 1.0,
                    btn_y,
                    1.0,
                    btn_h,
                    theme.grid_line,
                );
            }
            let indicator = match data.sort {
                Some((sc, SortDirection::Ascending)) if sc == ci => {
                    Some(("↑", theme.sort_indicator, true))
                }
                Some((sc, SortDirection::Descending)) if sc == ci => {
                    Some(("↓", theme.sort_indicator, true))
                }
                _ if hovered => Some(("-", theme.header_fg, false)),
                _ => None,
            };
            if let Some((ind, ind_color, ind_bold)) = indicator {
                paint_icon(
                    window,
                    cx,
                    ind,
                    btn_x + (btn_w - cw * ICON_SCALE) * 0.5,
                    oy + (hdr_h - fs * ICON_SCALE) * 0.5,
                    ind_color,
                    ind_bold,
                );
            }
            if data.filters_active[ci] {
                paint_icon(
                    window,
                    cx,
                    FILTER_ICON,
                    btn_x - fs * ICON_SCALE - 4.0,
                    oy + (hdr_h - fs * ICON_SCALE) * 0.5,
                    theme.sort_indicator,
                    false,
                );
            }
            fill_quad(window, x + w, oy, 1.0, hdr_h, theme.grid_line);
            col_x += w;
        }
    });
    fill_quad(window, ox, oy, rhw, hdr_h, theme.row_header_bg);

    fill_quad(window, ox, oy + hdr_h, sw, 1.0, theme.grid_line);
    fill_quad(window, ox + rhw, oy, 1.0, sh, theme.grid_line);

    // Empty result set: a quiet centered hint instead of a bare void. The
    // text is host-configurable (and localizable) via `GridConfig::empty_text`.
    if data.display_row_count() == 0 && !data.empty_text.is_empty() {
        let tw = text_w_approx(&data.empty_text, cw);
        let tx = ox + rhw + ((sw - rhw) - tw) * 0.5;
        let ty = oy + hdr_h + (sh - hdr_h - fs).max(0.0) * 0.35;
        paint_txt(
            window,
            cx,
            &data.empty_text,
            tx.max(ox + rhw + 8.0),
            ty,
            theme.muted_text,
            Some(sw - rhw - 16.0),
        );
    }

    if let Some((start, current)) = data.drag_rect {
        // `drag_rect` corners are grid-relative; shift by the grid origin to
        // paint them in the window's absolute coordinate space. Clipped to the
        // grid bounds so a drag past the edge cannot paint outside the grid.
        let (sx0, sy0) = (ox + f32::from(start.x), oy + f32::from(start.y));
        let (sx1, sy1) = (ox + f32::from(current.x), oy + f32::from(current.y));
        let (rx, ry) = (sx0.min(sx1), sy0.min(sy1));
        let (rw, rh) = ((sx1 - sx0).abs(), (sy1 - sy0).abs());
        // Accent marquee outline so the drag rectangle reads while cells
        // beneath are still catching up with the live selection.
        window.with_content_mask(clip(ox, oy, sw, sh), |window| {
            window.paint_quad(PaintQuad {
                bounds: Bounds {
                    origin: Point {
                        x: px(rx),
                        y: px(ry),
                    },
                    size: size(px(rw), px(rh)),
                },
                background: hsla_const(0.0, 0.0, 0.0, 0.0).into(),
                border_color: theme.sort_indicator,
                border_widths: gpui::Edges::all(px(1.0)),
                corner_radii: Default::default(),
                border_style: Default::default(),
            });
        });
    }

    paint_scrollbars(data, window, ox, oy, sw, sh, theme);

    // The context menu is no longer painted here. It is rendered as a
    // `deferred` + `anchored` overlay in `widget.rs` so that it paints — and
    // receives mouse events — on top of everything, including regions outside
    // the grid widget's layout bounds (e.g. a host header above the grid). The
    // filter panel uses the same overlay mechanism, so it is not painted here
    // either.
}

fn text_w_approx(text: &str, char_width: f32) -> f32 {
    text.chars().count() as f32 * char_width
}

pub(crate) fn paint_status_bar(
    data: &StatusBarData,
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
) {
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let sw = f32::from(bounds.size.width);
    let sh = f32::from(bounds.size.height);
    let theme = &data.theme;
    let fs = data.font_size;

    fill_quad(window, ox, oy, sw, sh, theme.header_bg);
    fill_quad(window, ox, oy, sw, 1.0, theme.grid_line);

    let text_system = window.text_system().clone();
    let font_size = px(fs);
    let line_height = px(fs * 1.2);
    let font = grid_font();
    let run = gpui::TextRun {
        len: data.text.len(),
        color: theme.text_fg,
        font,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = text_system.shape_line(data.text.clone().into(), font_size, &[run], None);
    let _ = shaped.paint(
        Point {
            x: px(ox + 8.0),
            y: px(oy + (sh - fs) * 0.5),
        },
        line_height,
        window,
        cx,
    );
}

// Re-export MenuAction so widget code can mention it without a long path.
#[allow(unused_imports)]
pub(crate) use menu::MenuAction as _MenuAction;

#[cfg(test)]
mod tests {
    use super::floor_char_boundary;

    /// The truncation slice must land on char boundaries for multi-byte
    /// input — a mid-char index would panic the paint pass.
    #[test]
    fn floor_char_boundary_handles_multibyte_input() {
        let s = "a👍字م"; // ASCII, emoji (4 bytes), CJK (3), Arabic (2)
        for idx in 0..=s.len() + 2 {
            let b = floor_char_boundary(s, idx);
            assert!(b <= s.len());
            assert!(
                s.is_char_boundary(b),
                "idx {idx} floored to non-boundary {b}"
            );
            // Never floors past the char containing `idx`.
            assert!(idx.min(s.len()) - b < 4);
        }
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, s.len()), s.len());
        assert_eq!(floor_char_boundary("", 5), 0);
        // Slicing at the floored index never panics.
        let _ = &s[..floor_char_boundary(s, 3)];
        let _ = &s[..floor_char_boundary(s, 6)];
    }
}
