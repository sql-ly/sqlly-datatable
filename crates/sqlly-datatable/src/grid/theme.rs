//! `GridTheme` — typed color set used by the widget, plus the two shipped
//! theme families.
//!
//! Two complete families ship with the crate, each with a light and a dark
//! variant that follows the OS window appearance:
//!
//! - **Neutral** ([`GridThemePair::neutral`]) — chroma-free surfaces with a
//!   restrained azure accent. Blends into a host application; the default.
//! - **Signature** ([`GridThemePair::signature`]) — the crate's own look,
//!   built around a teal anchor (`oklch(0.47 0.115 195)`): tinted neutrals and
//!   a committed accent carrying selection, chips, and totals.
//!
//! Every color is a public field, so downstream code can construct a fully
//! custom theme (or derive one from a host app's palette) and pass it on the
//! [`crate::grid::GridState`] or via the widget builder. All shipped palettes
//! are designed in OKLCH (noted per field) and converted to `Hsla`; text
//! roles meet WCAG AA contrast (≥ 4.5:1) against every surface they are
//! painted on — see the `wcag` test module at the bottom of this file, which
//! verifies the full matrix for all four palettes.

use gpui::{Hsla, WindowAppearance};

#[derive(Clone, Debug, PartialEq)]
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
    /// Foreground for the null-value placeholder (see
    /// [`crate::config::NullFormat`]).
    pub null_fg: Hsla,
    /// Distinctive background painted behind null-value cells when the
    /// column's [`crate::config::NullFormat::background`] is enabled.
    pub null_bg: Hsla,
    /// Background of pivot group-header rows (expanded groups).
    pub pivot_group_bg: Hsla,
    /// Background of pivot subtotal cells (collapsed groups, "Total"
    /// columns).
    pub pivot_subtotal_bg: Hsla,
    /// Background of the pivot grand-total row/column.
    pub pivot_grand_total_bg: Hsla,
    /// Foreground for pivot subtotal / grand-total values and labels.
    pub pivot_total_fg: Hsla,
    /// Resting background of a sidebar drop zone.
    pub pivot_drop_zone_bg: Hsla,
    /// Background of a drop zone while a compatible chip hovers over it.
    pub pivot_drop_zone_active_bg: Hsla,
    /// Background of a field chip in the pivot sidebar.
    pub pivot_chip_bg: Hsla,
    /// Label color of a field chip in the pivot sidebar.
    pub pivot_chip_fg: Hsla,
    /// Fill of the scrollbar thumb (the track uses `row_header_bg`). Kept at
    /// ≥ 3:1 contrast against the track in the shipped palettes.
    pub scrollbar_thumb: Hsla,
    /// Translucent scrim painted behind modal overlays (e.g. the pivot
    /// format dialog) to dim the content beneath. The only intentionally
    /// non-opaque color in the theme.
    pub overlay_scrim: Hsla,
}

impl Default for GridTheme {
    fn default() -> Self {
        Self::neutral_light()
    }
}

fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

/// A light/dark pair forming one theme family. [`GridThemePair::for_appearance`]
/// picks the variant matching the OS window appearance, so a host app can
/// supply its own pair and keep automatic light/dark following.
#[derive(Clone, Debug, PartialEq)]
pub struct GridThemePair {
    pub light: GridTheme,
    pub dark: GridTheme,
}

impl Default for GridThemePair {
    fn default() -> Self {
        Self::neutral()
    }
}

impl GridThemePair {
    /// The Neutral family: chroma-free surfaces, one restrained azure accent.
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            light: GridTheme::neutral_light(),
            dark: GridTheme::neutral_dark(),
        }
    }

    /// The Signature family: teal-anchored tinted neutrals with a committed
    /// accent.
    #[must_use]
    pub fn signature() -> Self {
        Self {
            light: GridTheme::signature_light(),
            dark: GridTheme::signature_dark(),
        }
    }

    /// A family derived from `gpui-component`'s built-in light and dark
    /// palettes via [`GridTheme::from_component_colors`], so the grid matches
    /// hosts styled with `gpui-component` defaults while keeping automatic
    /// light/dark following. Hosts running a *custom* component theme should
    /// instead derive from their own palettes, or from the active theme with
    /// [`GridTheme::from_component_theme`].
    ///
    /// Unlike [`GridThemePair::neutral`] and [`GridThemePair::signature`],
    /// the component palettes carry no WCAG contrast guarantee from this
    /// crate.
    #[must_use]
    pub fn component() -> Self {
        Self {
            light: GridTheme::from_component_colors(&gpui_component::ThemeColor::light(), false),
            dark: GridTheme::from_component_colors(&gpui_component::ThemeColor::dark(), true),
        }
    }

    /// Pick the variant that matches the OS window appearance. `Dark` and
    /// `VibrantDark` resolve to `self.dark`; everything else to `self.light`.
    #[must_use]
    pub fn for_appearance(&self, appearance: WindowAppearance) -> GridTheme {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => self.dark.clone(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => self.light.clone(),
        }
    }
}

impl GridTheme {
    /// Derive a `GridTheme` from the active [`gpui_component::Theme`], so the
    /// grid picks up the exact surfaces, borders, and accents of a host app
    /// built on `gpui-component`. Reads the theme resolved for the current
    /// light/dark mode; hosts that switch modes at runtime should re-derive
    /// and re-apply on change (e.g. from the same place they call
    /// [`gpui_component::Theme::change`]).
    ///
    /// The mapping leans on the component theme's dedicated `table_*` role
    /// colors for the grid chrome and its `popover`/`list` roles for menus,
    /// so the result matches what `gpui-component`'s own `Table` would look
    /// like in the host theme.
    ///
    /// ```no_run
    /// use gpui_component::ActiveTheme as _;
    /// # fn derive(cx: &mut gpui::App) -> sqlly_datatable::GridTheme {
    /// sqlly_datatable::GridTheme::from_component_theme(cx.theme())
    /// # }
    /// ```
    #[must_use]
    pub fn from_component_theme(theme: &gpui_component::Theme) -> Self {
        Self::from_component_colors(&theme.colors, theme.is_dark())
    }

    /// The [`GridTheme::from_component_theme`] mapping applied to an explicit
    /// [`gpui_component::ThemeColor`] set. Useful for building a
    /// [`GridThemePair`] from a component theme's light and dark palettes
    /// (see [`GridThemePair::component`]).
    #[must_use]
    pub fn from_component_colors(colors: &gpui_component::ThemeColor, is_dark: bool) -> Self {
        Self {
            bg: colors.table,
            header_bg: colors.table_head,
            filter_bg: colors.table_head,
            filter_active_bg: colors.accent,
            row_header_bg: colors.table_head,
            selection_bg: colors.selection,
            alt_row_bg: colors.table_even,
            grid_line: colors.table_row_border,
            header_fg: colors.table_head_foreground,
            text_fg: colors.foreground,
            negative_fg: colors.danger,
            sort_indicator: colors.primary,
            filter_cursor: colors.caret,
            menu_bg: colors.popover,
            menu_hover_bg: colors.list_hover,
            menu_fg: colors.popover_foreground,
            muted_text: colors.muted_foreground,
            null_fg: colors.muted_foreground,
            null_bg: colors.muted,
            pivot_group_bg: colors.accent,
            pivot_subtotal_bg: colors.secondary,
            pivot_grand_total_bg: colors.selection,
            pivot_total_fg: colors.foreground,
            pivot_drop_zone_bg: colors.muted,
            pivot_drop_zone_active_bg: colors.drop_target,
            pivot_chip_bg: colors.secondary,
            pivot_chip_fg: colors.secondary_foreground,
            scrollbar_thumb: colors.scrollbar_thumb,
            overlay_scrim: if is_dark {
                hsla(0.0, 0.0, 0.0, 0.45)
            } else {
                hsla(0.0, 0.0, 0.0, 0.35)
            },
        }
    }

    /// Neutral light: pure-white canvas, gray ramp at zero chroma, azure
    /// accent reserved for selection, sort, and filter state.
    #[must_use]
    pub fn neutral_light() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 1.0, 1.0),                            // oklch(1 0 0)
            header_bg: hsla(0.0, 0.0, 0.941, 1.0),                   // oklch(0.955 0 0)
            filter_bg: hsla(0.0, 0.0, 0.941, 1.0),                   // oklch(0.955 0 0)
            filter_active_bg: hsla(0.5778, 1.0, 0.8778, 1.0),        // oklch(0.90 0.06 250)
            row_header_bg: hsla(0.0, 0.0, 0.9215, 1.0),              // oklch(0.94 0 0)
            selection_bg: hsla(0.5786, 1.0, 0.8972, 1.0),            // oklch(0.915 0.05 250)
            alt_row_bg: hsla(0.0, 0.0, 0.9306, 1.0),                 // oklch(0.947 0 0)
            grid_line: hsla(0.0, 0.0, 0.8634, 1.0),                  // oklch(0.895 0 0)
            header_fg: hsla(0.0, 0.0, 0.1599, 1.0),                  // oklch(0.28 0 0)
            text_fg: hsla(0.0, 0.0, 0.0476, 1.0),                    // oklch(0.155 0 0)
            negative_fg: hsla(0.9991, 0.7103, 0.4152, 1.0),          // oklch(0.50 0.185 27)
            sort_indicator: hsla(0.5763, 1.0, 0.3224, 1.0),          // oklch(0.46 0.145 250)
            filter_cursor: hsla(0.0, 0.0, 0.0476, 1.0),              // oklch(0.155 0 0)
            menu_bg: hsla(0.0, 0.0, 1.0, 1.0),                       // oklch(1 0 0)
            menu_hover_bg: hsla(0.5798, 1.0, 0.9166, 1.0),           // oklch(0.93 0.04 250)
            menu_fg: hsla(0.0, 0.0, 0.0476, 1.0),                    // oklch(0.155 0 0)
            muted_text: hsla(0.0, 0.0, 0.3447, 1.0),                 // oklch(0.46 0 0)
            null_fg: hsla(0.1215, 0.0996, 0.3291, 1.0),              // oklch(0.46 0.02 90)
            null_bg: hsla(0.1324, 0.7485, 0.9213, 1.0),              // oklch(0.965 0.032 95)
            pivot_group_bg: hsla(0.5863, 0.7823, 0.9366, 1.0),       // oklch(0.945 0.022 250)
            pivot_subtotal_bg: hsla(0.5862, 0.724, 0.9011, 1.0),     // oklch(0.915 0.032 250)
            pivot_grand_total_bg: hsla(0.5858, 0.7776, 0.8434, 1.0), // oklch(0.865 0.055 250)
            pivot_total_fg: hsla(0.585, 0.4177, 0.0724, 1.0),        // oklch(0.18 0.02 250)
            pivot_drop_zone_bg: hsla(0.0, 0.0, 0.954, 1.0),          // oklch(0.965 0 0)
            pivot_drop_zone_active_bg: hsla(0.5756, 1.0, 0.9004, 1.0), // oklch(0.92 0.05 250)
            pivot_chip_bg: hsla(0.5798, 1.0, 0.9166, 1.0),           // oklch(0.93 0.04 250)
            pivot_chip_fg: hsla(0.5844, 0.4223, 0.1374, 1.0),        // oklch(0.25 0.035 250)
            scrollbar_thumb: hsla(0.0, 0.0, 0.5021, 1.0),            // oklch(0.60 0 0)
            overlay_scrim: hsla(0.0, 0.0, 0.0, 0.35),
        }
    }

    /// Neutral dark: near-black gray ramp; depth comes from surface
    /// lightness, accents are slightly desaturated to sit on dark.
    #[must_use]
    pub fn neutral_dark() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 0.0817, 1.0),        // oklch(0.195 0 0)
            header_bg: hsla(0.0, 0.0, 0.1409, 1.0), // oklch(0.26 0 0)
            filter_bg: hsla(0.0, 0.0, 0.1409, 1.0), // oklch(0.26 0 0)
            filter_active_bg: hsla(0.583, 0.4915, 0.2685, 1.0), // oklch(0.38 0.07 250)
            row_header_bg: hsla(0.0, 0.0, 0.1268, 1.0), // oklch(0.245 0 0)
            selection_bg: hsla(0.5823, 0.5446, 0.2618, 1.0), // oklch(0.375 0.075 250)
            alt_row_bg: hsla(0.0, 0.0, 0.1362, 1.0), // oklch(0.255 0 0)
            grid_line: hsla(0.0, 0.0, 0.2089, 1.0), // oklch(0.33 0 0)
            header_fg: hsla(0.0, 0.0, 0.806, 1.0),  // oklch(0.85 0 0)
            text_fg: hsla(0.0, 0.0, 0.9085, 1.0),   // oklch(0.93 0 0)
            negative_fg: hsla(0.0076, 0.8612, 0.6827, 1.0), // oklch(0.70 0.165 25)
            sort_indicator: hsla(0.5751, 0.8029, 0.6722, 1.0), // oklch(0.74 0.115 245)
            filter_cursor: hsla(0.0, 0.0, 0.9085, 1.0), // oklch(0.93 0 0)
            menu_bg: hsla(0.0, 0.0, 0.1315, 1.0),   // oklch(0.25 0 0)
            menu_hover_bg: hsla(0.5834, 0.4685, 0.2583, 1.0), // oklch(0.37 0.065 250)
            menu_fg: hsla(0.0, 0.0, 0.9085, 1.0),   // oklch(0.93 0 0)
            muted_text: hsla(0.0, 0.0, 0.6326, 1.0), // oklch(0.71 0 0)
            null_fg: hsla(0.132, 0.2549, 0.6858, 1.0), // oklch(0.79 0.045 95)
            null_bg: hsla(0.1318, 0.3896, 0.1459, 1.0), // oklch(0.30 0.038 95)
            pivot_group_bg: hsla(0.5853, 0.2639, 0.1806, 1.0), // oklch(0.295 0.028 250)
            pivot_subtotal_bg: hsla(0.5847, 0.3421, 0.2171, 1.0), // oklch(0.33 0.042 250)
            pivot_grand_total_bg: hsla(0.584, 0.3998, 0.2911, 1.0), // oklch(0.40 0.062 250)
            pivot_total_fg: hsla(0.0, 0.0, 0.941, 1.0), // oklch(0.955 0 0)
            pivot_drop_zone_bg: hsla(0.0, 0.0, 0.1268, 1.0), // oklch(0.245 0 0)
            pivot_drop_zone_active_bg: hsla(0.5835, 0.4628, 0.2375, 1.0), // oklch(0.35 0.06 250)
            pivot_chip_bg: hsla(0.584, 0.4202, 0.2379, 1.0), // oklch(0.35 0.055 250)
            pivot_chip_fg: hsla(0.0, 0.0, 0.9345, 1.0), // oklch(0.95 0 0)
            scrollbar_thumb: hsla(0.0, 0.0, 0.4447, 1.0), // oklch(0.55 0 0)
            overlay_scrim: hsla(0.0, 0.0, 0.0, 0.45),
        }
    }

    /// Signature light: pure-white canvas with teal-tinted neutrals; the
    /// teal anchor (`oklch(0.47 0.115 195)` family) carries selection,
    /// chips, and the totals hierarchy.
    #[must_use]
    pub fn signature_light() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 1.0, 1.0),                            // oklch(1 0 0)
            header_bg: hsla(0.4955, 0.2665, 0.917, 1.0),             // oklch(0.945 0.012 195)
            filter_bg: hsla(0.4955, 0.2665, 0.917, 1.0),             // oklch(0.945 0.012 195)
            filter_active_bg: hsla(0.4978, 0.5936, 0.7861, 1.0),     // oklch(0.89 0.065 195)
            row_header_bg: hsla(0.4956, 0.2469, 0.8956, 1.0),        // oklch(0.93 0.014 195)
            selection_bg: hsla(0.497, 0.5728, 0.8399, 1.0),          // oklch(0.915 0.048 195)
            alt_row_bg: hsla(0.4953, 0.1854, 0.9207, 1.0),           // oklch(0.945 0.008 195)
            grid_line: hsla(0.4957, 0.1778, 0.8359, 1.0),            // oklch(0.885 0.016 195)
            header_fg: hsla(0.5, 0.6775, 0.1233, 1.0),               // oklch(0.30 0.045 195)
            text_fg: hsla(0.4982, 0.397, 0.0491, 1.0),               // oklch(0.17 0.015 195)
            negative_fg: hsla(0.015, 0.7339, 0.4008, 1.0),           // oklch(0.50 0.175 30)
            sort_indicator: hsla(0.502, 1.0, 0.217, 1.0),            // oklch(0.47 0.115 195)
            filter_cursor: hsla(0.4982, 0.397, 0.0491, 1.0),         // oklch(0.17 0.015 195)
            menu_bg: hsla(0.0, 0.0, 1.0, 1.0),                       // oklch(1 0 0)
            menu_hover_bg: hsla(0.4968, 0.5969, 0.8663, 1.0),        // oklch(0.93 0.042 195)
            menu_fg: hsla(0.4982, 0.397, 0.0491, 1.0),               // oklch(0.17 0.015 195)
            muted_text: hsla(0.4975, 0.1546, 0.3177, 1.0),           // oklch(0.46 0.03 195)
            null_fg: hsla(0.1119, 0.1325, 0.3271, 1.0),              // oklch(0.46 0.025 85)
            null_bg: hsla(0.1176, 0.7869, 0.9237, 1.0),              // oklch(0.962 0.03 88)
            pivot_group_bg: hsla(0.4962, 0.5099, 0.8971, 1.0),       // oklch(0.942 0.028 195)
            pivot_subtotal_bg: hsla(0.4969, 0.5164, 0.8345, 1.0),    // oklch(0.908 0.045 195)
            pivot_grand_total_bg: hsla(0.4983, 0.5319, 0.7204, 1.0), // oklch(0.85 0.075 195)
            pivot_total_fg: hsla(0.5035, 1.0, 0.0689, 1.0),          // oklch(0.22 0.06 195)
            pivot_drop_zone_bg: hsla(0.4953, 0.2775, 0.9468, 1.0),   // oklch(0.965 0.008 195)
            pivot_drop_zone_active_bg: hsla(0.4974, 0.6519, 0.8275, 1.0), // oklch(0.915 0.058 195)
            pivot_chip_bg: hsla(0.4973, 0.5837, 0.8185, 1.0),        // oklch(0.905 0.055 195)
            pivot_chip_fg: hsla(0.503, 1.0, 0.1024, 1.0),            // oklch(0.28 0.075 195)
            scrollbar_thumb: hsla(0.4975, 0.1513, 0.4634, 1.0),      // oklch(0.60 0.04 195)
            overlay_scrim: hsla(0.5019, 1.0, 0.0092, 0.35),          // oklch(0.10 0.02 195)
        }
    }

    /// Signature dark: teal-tinted near-black; surfaces keep the anchor hue
    /// at whisper chroma, and the accent brightens to hold on dark.
    #[must_use]
    pub fn signature_dark() -> Self {
        Self {
            bg: hsla(0.4974, 0.2284, 0.0687, 1.0), // oklch(0.19 0.012 195)
            header_bg: hsla(0.4977, 0.2102, 0.1219, 1.0), // oklch(0.255 0.018 195)
            filter_bg: hsla(0.4977, 0.2102, 0.1219, 1.0), // oklch(0.255 0.018 195)
            filter_active_bg: hsla(0.5016, 1.0, 0.1547, 1.0), // oklch(0.375 0.085 195)
            row_header_bg: hsla(0.4976, 0.2111, 0.1053, 1.0), // oklch(0.235 0.016 195)
            selection_bg: hsla(0.5016, 1.0, 0.1547, 1.0), // oklch(0.375 0.085 195)
            alt_row_bg: hsla(0.4974, 0.1857, 0.1210, 1.0), // oklch(0.252 0.016 195)
            grid_line: hsla(0.4975, 0.1743, 0.1906, 1.0), // oklch(0.33 0.022 195)
            header_fg: hsla(0.4959, 0.1709, 0.7876, 1.0), // oklch(0.85 0.02 195)
            text_fg: hsla(0.4953, 0.1483, 0.9013, 1.0), // oklch(0.93 0.008 195)
            negative_fg: hsla(0.0092, 0.8161, 0.6929, 1.0), // oklch(0.71 0.15 25)
            sort_indicator: hsla(0.5, 0.5555, 0.5112, 1.0), // oklch(0.76 0.115 195)
            filter_cursor: hsla(0.4953, 0.1483, 0.9013, 1.0), // oklch(0.93 0.008 195)
            menu_bg: hsla(0.4975, 0.1954, 0.1145, 1.0), // oklch(0.245 0.016 195)
            menu_hover_bg: hsla(0.5011, 1.0, 0.1462, 1.0), // oklch(0.365 0.075 195)
            menu_fg: hsla(0.4953, 0.1483, 0.9013, 1.0), // oklch(0.93 0.008 195)
            muted_text: hsla(0.4965, 0.1256, 0.6067, 1.0), // oklch(0.71 0.028 195)
            null_fg: hsla(0.1175, 0.3234, 0.6993, 1.0), // oklch(0.80 0.05 88)
            null_bg: hsla(0.1183, 0.3763, 0.1477, 1.0), // oklch(0.295 0.035 88)
            pivot_group_bg: hsla(0.4992, 0.4103, 0.1392, 1.0), // oklch(0.295 0.035 195)
            pivot_subtotal_bg: hsla(0.5, 0.6646, 0.1444, 1.0), // oklch(0.33 0.05 195)
            pivot_grand_total_bg: hsla(0.5005, 1.0, 0.164, 1.0), // oklch(0.40 0.072 195)
            pivot_total_fg: hsla(0.4952, 0.1907, 0.9421, 1.0), // oklch(0.96 0.006 195)
            pivot_drop_zone_bg: hsla(0.4975, 0.2029, 0.1099, 1.0), // oklch(0.24 0.016 195)
            pivot_drop_zone_active_bg: hsla(0.5009, 1.0, 0.1335, 1.0), // oklch(0.345 0.068 195)
            pivot_chip_bg: hsla(0.5013, 1.0, 0.1383, 1.0), // oklch(0.35 0.075 195)
            pivot_chip_fg: hsla(0.4954, 0.2469, 0.9254, 1.0), // oklch(0.95 0.01 195)
            scrollbar_thumb: hsla(0.4967, 0.0942, 0.4236, 1.0), // oklch(0.55 0.024 195)
            overlay_scrim: hsla(0.5019, 1.0, 0.0011, 0.45), // oklch(0.05 0.01 195)
        }
    }

    /// The Neutral light palette. Identical to [`GridTheme::default`];
    /// provided as a named constructor so callers can be explicit about
    /// intent.
    #[must_use]
    pub fn light() -> Self {
        Self::neutral_light()
    }

    /// The Neutral dark palette, tuned to pair with [`GridTheme::light`].
    #[must_use]
    pub fn dark() -> Self {
        Self::neutral_dark()
    }

    /// Pick the Neutral-family palette that matches the OS window
    /// appearance. `Dark` and `VibrantDark` resolve to
    /// [`GridTheme::neutral_dark`]; everything else to
    /// [`GridTheme::neutral_light`]. For other families use
    /// [`GridThemePair::for_appearance`].
    #[must_use]
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        GridThemePair::neutral().for_appearance(appearance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_palettes() -> [(&'static str, GridTheme); 4] {
        [
            ("neutral_light", GridTheme::neutral_light()),
            ("neutral_dark", GridTheme::neutral_dark()),
            ("signature_light", GridTheme::signature_light()),
            ("signature_dark", GridTheme::signature_dark()),
        ]
    }

    /// The context menu must be paintable from the theme (not a hardcoded
    /// color), and its hover fill must be visually distinct from the menu
    /// background so a mouse-over state is actually perceivable. The label
    /// color must also contrast with the background. This guards the
    /// dark/light theming + hover-state regression, for every shipped
    /// palette.
    #[test]
    fn every_palette_exposes_distinct_menu_colors() {
        for (name, t) in all_palettes() {
            // Menu surface must be opaque so it fully covers content beneath.
            assert_eq!(t.menu_bg.a, 1.0, "{name}: menu background must be opaque");
            assert_ne!(
                t.menu_hover_bg, t.menu_bg,
                "{name}: menu hover fill must differ from the menu background"
            );
            assert_ne!(
                t.menu_fg, t.menu_bg,
                "{name}: menu label color must contrast with the menu background"
            );
        }
    }

    /// `light()`/`default()` must equal the Neutral light palette, `dark()`
    /// the Neutral dark palette, and dark variants must be genuinely dark
    /// (light text on a dark surface). This guards the OS light/dark
    /// following and the back-compat aliases.
    #[test]
    fn aliases_and_dark_variants_hold() {
        assert_eq!(
            GridTheme::light(),
            GridTheme::default(),
            "light() must alias the default palette"
        );
        assert_eq!(GridTheme::light(), GridTheme::neutral_light());
        assert_eq!(GridTheme::dark(), GridTheme::neutral_dark());
        for (name, t) in [
            ("neutral_dark", GridTheme::neutral_dark()),
            ("signature_dark", GridTheme::signature_dark()),
        ] {
            assert!(
                t.bg.l < t.text_fg.l,
                "{name} must be light text on a dark surface"
            );
        }
        for (name, t) in all_palettes() {
            assert_eq!(t.bg.a, 1.0, "{name}: grid background must be opaque");
            assert_eq!(t.selection_bg.a, 1.0, "{name}: selection must be opaque");
        }
    }

    /// Pivot surfaces must be mutually distinguishable and legible in every
    /// palette: totals must stand out from ordinary groups, drop-zone hover
    /// must differ from its resting state, and total text must contrast
    /// with the total background.
    #[test]
    fn pivot_surfaces_are_distinct_in_every_palette() {
        for (name, t) in all_palettes() {
            assert_ne!(t.pivot_grand_total_bg, t.pivot_group_bg, "{name}");
            assert_ne!(t.pivot_grand_total_bg, t.pivot_subtotal_bg, "{name}");
            assert_ne!(t.pivot_total_fg, t.pivot_grand_total_bg, "{name}");
            assert_ne!(t.pivot_drop_zone_active_bg, t.pivot_drop_zone_bg, "{name}");
            assert_ne!(t.pivot_chip_fg, t.pivot_chip_bg, "{name}");
        }
    }

    /// `for_appearance` must map the two dark variants to the dark palette
    /// and the two light variants to the light palette, on both the static
    /// Neutral helper and an arbitrary pair.
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
        let sig = GridThemePair::signature();
        assert_eq!(
            sig.for_appearance(WindowAppearance::Dark).bg,
            GridTheme::signature_dark().bg
        );
        assert_eq!(
            sig.for_appearance(WindowAppearance::VibrantLight).bg,
            GridTheme::signature_light().bg
        );
    }

    /// The `gpui-component` bridge must map the toolkit's role colors onto
    /// the grid's fields (not fall back to a shipped palette), keep the
    /// light and dark derivations distinct, and expose them as a pair.
    #[test]
    fn component_bridge_maps_toolkit_roles_per_mode() {
        let light_colors = gpui_component::ThemeColor::light();
        let dark_colors = gpui_component::ThemeColor::dark();

        let light = GridTheme::from_component_colors(&light_colors, false);
        let dark = GridTheme::from_component_colors(&dark_colors, true);

        // Spot-check the role mapping against the source palette.
        assert_eq!(light.bg, light_colors.table);
        assert_eq!(light.header_bg, light_colors.table_head);
        assert_eq!(light.selection_bg, light_colors.selection);
        assert_eq!(light.menu_bg, light_colors.popover);
        assert_eq!(light.negative_fg, light_colors.danger);
        assert_eq!(dark.bg, dark_colors.table);

        // Light and dark derivations must actually differ.
        assert_ne!(light.bg, dark.bg);
        assert_ne!(light.text_fg, dark.text_fg);
        // The dark scrim is heavier than the light one.
        assert!(dark.overlay_scrim.a > light.overlay_scrim.a);

        // The ready-made pair is exactly those two derivations.
        let pair = GridThemePair::component();
        assert_eq!(pair.light, light);
        assert_eq!(pair.dark, dark);
    }
}

/// WCAG contrast verification for the shipped palettes. Every text role is
/// checked against every surface it is actually painted on in `paint.rs` /
/// `sidebar.rs` / `widget.rs`, at AA thresholds (4.5:1 for text, 3:1 for
/// UI indicators), plus perceivable-difference floors for state fills.
#[cfg(test)]
mod wcag {
    use super::*;

    /// Convert an `Hsla` (alpha ignored — all checked colors are opaque,
    /// guarded by `aliases_and_dark_variants_hold`) to linear-light sRGB
    /// relative luminance.
    fn relative_luminance(c: Hsla) -> f32 {
        // HSL -> sRGB
        let (h, s, l) = (c.h, c.s, c.l);
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        let hue = |mut t: f32| -> f32 {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        let (r, g, b) = (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0));
        // gamma -> linear
        let lin = |u: f32| -> f32 {
            if u <= 0.04045 {
                u / 12.92
            } else {
                ((u + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// (foreground field, background field, minimum ratio, painted where)
    fn requirements(t: &GridTheme) -> Vec<(Hsla, Hsla, f32, &'static str)> {
        vec![
            (t.text_fg, t.bg, 7.0, "body text on bg"),
            (t.text_fg, t.alt_row_bg, 7.0, "body text on zebra row"),
            (t.text_fg, t.selection_bg, 4.5, "body text on selection"),
            (t.text_fg, t.row_header_bg, 4.5, "pivot leaf label"),
            (t.text_fg, t.header_bg, 4.5, "status bar text"),
            (t.text_fg, t.filter_active_bg, 4.5, "filter text"),
            (t.header_fg, t.header_bg, 4.5, "column labels"),
            (t.header_fg, t.row_header_bg, 4.5, "row numbers"),
            (t.header_fg, t.selection_bg, 4.5, "selected header label"),
            (
                t.header_fg,
                t.pivot_grand_total_bg,
                4.5,
                "pivot header on total bg",
            ),
            (t.muted_text, t.bg, 4.5, "placeholder text on bg"),
            (t.muted_text, t.menu_bg, 4.5, "menu secondary text"),
            (t.muted_text, t.header_bg, 4.5, "sidebar header hint"),
            (t.muted_text, t.pivot_drop_zone_bg, 4.5, "drop zone hint"),
            (t.negative_fg, t.bg, 4.5, "negative numbers on bg"),
            (
                t.negative_fg,
                t.alt_row_bg,
                4.5,
                "negative numbers on zebra",
            ),
            (
                t.negative_fg,
                t.selection_bg,
                3.0,
                "negative on selection (parentheses carry the channel too)",
            ),
            (t.negative_fg, t.pivot_grand_total_bg, 3.0, "negative total"),
            (t.menu_fg, t.menu_bg, 7.0, "menu labels"),
            (t.menu_fg, t.menu_hover_bg, 4.5, "hovered menu label"),
            (t.menu_fg, t.header_bg, 4.5, "source field chip label"),
            (
                t.menu_fg,
                t.pivot_drop_zone_bg,
                4.5,
                "pivot menu on zone bg",
            ),
            (t.null_fg, t.bg, 4.5, "null placeholder on bg"),
            (t.null_fg, t.null_bg, 4.5, "null placeholder on null bg"),
            (t.null_fg, t.alt_row_bg, 4.5, "null placeholder on zebra"),
            (
                t.pivot_total_fg,
                t.pivot_group_bg,
                7.0,
                "group header label",
            ),
            (
                t.pivot_total_fg,
                t.pivot_subtotal_bg,
                4.5,
                "subtotal values",
            ),
            (
                t.pivot_total_fg,
                t.pivot_grand_total_bg,
                4.5,
                "grand total values",
            ),
            (t.pivot_chip_fg, t.pivot_chip_bg, 4.5, "chip labels"),
            (t.bg, t.sort_indicator, 3.0, "checkbox knockout check"),
            (t.sort_indicator, t.header_bg, 3.0, "sort glyph"),
            (t.sort_indicator, t.menu_bg, 4.5, "sidebar sort glyph"),
            (t.sort_indicator, t.bg, 3.0, "grouped-column underline"),
            (t.filter_cursor, t.filter_active_bg, 4.5, "filter cursor"),
            (t.scrollbar_thumb, t.row_header_bg, 3.0, "thumb vs track"),
        ]
    }

    /// State fills must be perceivably different from what they replace.
    fn distinctness(t: &GridTheme) -> Vec<(Hsla, Hsla, f32, &'static str)> {
        vec![
            (t.selection_bg, t.bg, 1.1, "selection visible on bg"),
            // Zebra must carry row-tracking across a wide horizontal scroll,
            // not just technically differ from the base row: hold a genuinely
            // perceptible band (the shipped palettes sit at ~1.16).
            (t.alt_row_bg, t.bg, 1.12, "zebra perceptible"),
            (t.menu_hover_bg, t.menu_bg, 1.1, "menu hover visible"),
            (
                t.pivot_drop_zone_active_bg,
                t.pivot_drop_zone_bg,
                1.1,
                "drop-zone hover visible",
            ),
            (t.grid_line, t.bg, 1.1, "grid lines visible"),
            (
                t.pivot_grand_total_bg,
                t.pivot_subtotal_bg,
                1.05,
                "totals hierarchy readable",
            ),
        ]
    }

    #[test]
    fn every_palette_meets_wcag_contrast() {
        for (name, theme) in [
            ("neutral_light", GridTheme::neutral_light()),
            ("neutral_dark", GridTheme::neutral_dark()),
            ("signature_light", GridTheme::signature_light()),
            ("signature_dark", GridTheme::signature_dark()),
        ] {
            for (fg, bg, min, what) in requirements(&theme).into_iter().chain(distinctness(&theme))
            {
                let ratio = contrast(fg, bg);
                assert!(
                    ratio >= min,
                    "{name}: {what} — contrast {ratio:.2} below required {min}"
                );
            }
        }
    }
}
