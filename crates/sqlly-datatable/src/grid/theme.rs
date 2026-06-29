//! `GridTheme` — typed color set used by the widget. Default is monochrome on
//! white; downstream code that wants a dark mode or accent palette can
//! construct a custom theme and pass it on the [`crate::grid::GridState`].

use gpui::Hsla;

#[derive(Clone, Debug)]
pub struct GridTheme {
    pub bg: Hsla,
    pub header_bg: Hsla,
    pub filter_bg: Hsla,
    pub filter_active_bg: Hsla,
    pub row_header_bg: Hsla,
    pub selection_bg: Hsla,
    pub alt_row_bg: Hsla,
    pub grid_line: Hsla,
    pub header_fg: Hsla,
    pub text_fg: Hsla,
    pub negative_fg: Hsla,
    pub sort_indicator: Hsla,
    pub filter_cursor: Hsla,
}

impl Default for GridTheme {
    fn default() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 1.0, 1.0),
            header_bg: hsla(0.0, 0.0, 0.93, 1.0),
            filter_bg: hsla(0.0, 0.0, 0.96, 1.0),
            filter_active_bg: hsla(0.58, 0.30, 0.85, 1.0),
            row_header_bg: hsla(0.0, 0.0, 0.90, 1.0),
            selection_bg: hsla(0.58, 0.50, 0.80, 0.50),
            alt_row_bg: hsla(0.0, 0.0, 0.95, 1.0),
            grid_line: hsla(0.0, 0.0, 0.85, 1.0),
            header_fg: hsla(0.0, 0.0, 0.15, 1.0),
            text_fg: hsla(0.0, 0.0, 0.1, 1.0),
            negative_fg: hsla(0.0, 0.75, 0.45, 1.0),
            sort_indicator: hsla(0.58, 0.50, 0.40, 1.0),
            filter_cursor: hsla(0.0, 0.0, 0.1, 1.0),
        }
    }
}

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}
