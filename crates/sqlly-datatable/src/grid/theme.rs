//! `GridTheme` — typed color set used by the widget. Default is monochrome on
//! white; downstream code that wants a dark mode or accent palette can
//! construct a custom theme and pass it on the [`crate::grid::GridState`].

use gpui::{Hsla, WindowAppearance};

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
    /// Muted text color for labels, placeholders, and secondary text inside
    /// the filter panel and context menu. Chosen for legibility against
    /// `menu_bg` / `bg` in both light and dark palettes.
    pub muted_text: Hsla,
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
            muted_text: hsla(0.0, 0.0, 0.5, 1.0),
        }
    }
}

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

impl GridTheme {
    /// The light palette. Identical to [`GridTheme::default`]; provided as a
    /// named constructor so callers can be explicit about intent.
    #[must_use]
    pub fn light() -> Self {
        Self::default()
    }

    /// A dark palette tuned to pair with the light one: light text on dark
    /// surfaces, matching accent hue (0.58) for selection/sort/menu-hover.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 0.12, 1.0),
            header_bg: hsla(0.0, 0.0, 0.18, 1.0),
            filter_bg: hsla(0.0, 0.0, 0.15, 1.0),
            filter_active_bg: hsla(0.58, 0.40, 0.30, 1.0),
            row_header_bg: hsla(0.0, 0.0, 0.16, 1.0),
            selection_bg: hsla(0.58, 0.50, 0.45, 0.50),
            alt_row_bg: hsla(0.0, 0.0, 0.15, 1.0),
            grid_line: hsla(0.0, 0.0, 0.28, 1.0),
            header_fg: hsla(0.0, 0.0, 0.80, 1.0),
            text_fg: hsla(0.0, 0.0, 0.90, 1.0),
            negative_fg: hsla(0.0, 0.70, 0.62, 1.0),
            sort_indicator: hsla(0.58, 0.60, 0.68, 1.0),
            filter_cursor: hsla(0.0, 0.0, 0.90, 1.0),
            menu_bg: hsla(0.0, 0.0, 0.16, 1.0),
            menu_hover_bg: hsla(0.58, 0.45, 0.38, 1.0),
            menu_fg: hsla(0.0, 0.0, 0.90, 1.0),
            muted_text: hsla(0.0, 0.0, 0.55, 1.0),
        }
    }

    /// Pick the palette that matches the OS window appearance. `Dark` and
    /// `VibrantDark` resolve to [`GridTheme::dark`]; everything else to
    /// [`GridTheme::light`].
    #[must_use]
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }
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

    /// `light()` must equal the default palette, and `dark()` must be a
    /// genuinely different, legible palette (dark surface, light text). This
    /// guards the OS light/dark following.
    #[test]
    fn light_matches_default_and_dark_differs() {
        assert_eq!(
            GridTheme::light().bg,
            GridTheme::default().bg,
            "light() must alias the default palette"
        );
        let dark = GridTheme::dark();
        assert_ne!(dark.bg, GridTheme::light().bg, "dark bg must differ");
        // Dark surface should be darker than its text (light-on-dark).
        assert!(
            dark.bg.l < dark.text_fg.l,
            "dark theme must be light text on a dark surface"
        );
        assert_eq!(dark.menu_bg.a, 1.0, "dark menu background must be opaque");
        assert_ne!(
            dark.menu_hover_bg, dark.menu_bg,
            "dark menu hover fill must differ from the menu background"
        );
    }

    /// `for_appearance` must map the two dark variants to the dark palette and
    /// the two light variants to the light palette.
    #[test]
    fn for_appearance_maps_dark_and_light_variants() {
        assert_eq!(
            GridTheme::for_appearance(WindowAppearance::Dark).bg,
            GridTheme::dark().bg
        );
        assert_eq!(
            GridTheme::for_appearance(WindowAppearance::VibrantDark).bg,
            GridTheme::dark().bg
        );
        assert_eq!(
            GridTheme::for_appearance(WindowAppearance::Light).bg,
            GridTheme::light().bg
        );
        assert_eq!(
            GridTheme::for_appearance(WindowAppearance::VibrantLight).bg,
            GridTheme::light().bg
        );
    }
}
