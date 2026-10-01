//! Host-configurable font family for canvas-painted text.
//!
//! Cells, headers, the pivot grid, charts, and the status bar all paint with
//! one face. By default that is the platform's monospace family (Menlo,
//! Consolas, DejaVu Sans Mono); an app that lets people pick a results font
//! sets it here. Process-global, like [`crate::set_tooltip_show_delay`]: every
//! table picks it up on its next paint.

use std::sync::RwLock;

use gpui::SharedString;

static FAMILY: RwLock<Option<SharedString>> = RwLock::new(None);

/// Set the family every table paints its text with, or `None` to go back to
/// the platform monospace default. Takes effect on each table's next frame, so
/// hosts should notify live tables after calling it. The platform monospace
/// families stay on as fallbacks, so a name the system doesn't have still
/// renders.
pub fn set_grid_font_family(family: Option<SharedString>) {
    let family = family.filter(|f| !f.trim().is_empty());
    *FAMILY.write().unwrap_or_else(|e| e.into_inner()) = family;
}

/// The family set by [`set_grid_font_family`], or `None` when tables use the
/// platform monospace default.
#[must_use]
pub fn grid_font_family() -> Option<SharedString> {
    FAMILY.read().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_defaults_to_platform_and_follows_the_host() {
        // Process-global: capture and restore so parallel tests are unaffected.
        let original = grid_font_family();

        set_grid_font_family(Some("Fira Code".into()));
        assert_eq!(grid_font_family().as_deref(), Some("Fira Code"));
        assert_eq!(crate::grid::paint::grid_font().family.as_ref(), "Fira Code");

        set_grid_font_family(Some("  ".into()));
        assert_eq!(grid_font_family(), None);

        set_grid_font_family(None);
        assert_ne!(crate::grid::paint::grid_font().family.as_ref(), "Fira Code");

        set_grid_font_family(original);
    }
}
