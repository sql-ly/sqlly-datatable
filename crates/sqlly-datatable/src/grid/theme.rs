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
    /// Background fill of the right-click context menu / filter popup surface.
    pub menu_bg: Hsla,
    /// Fill drawn behind the menu item currently under the pointer (hover).
    pub menu_hover_bg: Hsla,
    /// Foreground color for menu item labels.
    pub menu_fg: Hsla,
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
            menu_bg: hsla(0.0, 0.0, 1.0, 1.0),
            menu_hover_bg: hsla(0.58, 0.45, 0.85, 1.0),
            menu_fg: hsla(0.0, 0.0, 0.1, 1.0),
        }
    }
}

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The context menu must be paintable from the theme (not a hardcoded
    /// color), and its hover fill must be visually distinct from the menu
    /// background so a mouse-over state is actually perceivable. The label
    /// color must also contrast with the background. This guards the
    /// dark/light theming + hover-state regression.
    #[test]
    fn default_theme_exposes_distinct_menu_colors() {
        let t = GridTheme::default();
        // Menu surface must be opaque so it fully covers content beneath it.
        assert_eq!(t.menu_bg.a, 1.0, "menu background must be opaque");
        // Hover fill must differ from the surface, else hover is invisible.
        assert_ne!(
            t.menu_hover_bg, t.menu_bg,
            "menu hover fill must differ from the menu background"
        );
        // Label color must differ from the surface for legible text.
        assert_ne!(
            t.menu_fg, t.menu_bg,
            "menu label color must contrast with the menu background"
        );
    }
}
